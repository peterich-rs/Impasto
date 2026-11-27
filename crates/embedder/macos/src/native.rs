//! Native plugin callbacks scoped to the AppKit host invocation.

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Arc, Mutex, Weak};

use impasto::os::{NativeChannel, NativeEventChannel};

static ACTIVE: Mutex<Weak<NativeChannel>> = Mutex::new(Weak::new());
static ACTIVE_EVENTS: Mutex<Option<NativeEventChannel>> = Mutex::new(None);

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Callbacks {
    context: *mut c_void,
    open_url: Option<extern "C" fn(*mut c_void, *const c_char) -> bool>,
    service_request: Option<extern "C" fn(*mut c_void, u64, *const c_char, *const c_char)>,
    closed: Option<extern "C" fn(*mut c_void)>,
}

pub(crate) struct Host {
    legacy_open: Option<crate::OpenUrl>,
    callbacks: Option<Callbacks>,
    channel: Option<Arc<NativeChannel>>,
    events: Option<NativeEventChannel>,
}

impl Host {
    pub(crate) fn new(legacy_open: Option<crate::OpenUrl>, callbacks: Option<Callbacks>) -> Self {
        let channel = callbacks.and_then(|callbacks| {
            let request = callbacks.service_request?;
            let context = callbacks.context.expose_provenance();
            let channel = Arc::new_cyclic(|channel: &Weak<NativeChannel>| {
                let channel = Weak::clone(channel);
                NativeChannel::new(move |id, operation, payload| {
                    let (Ok(operation), Ok(payload)) =
                        (CString::new(operation), CString::new(payload))
                    else {
                        return false;
                    };
                    let Some(channel) = channel.upgrade() else {
                        return false;
                    };
                    crate::app::dispatch_main(Box::new(move || {
                        if channel.contains(id) {
                            request(
                                std::ptr::with_exposed_provenance_mut(context),
                                id,
                                operation.as_ptr(),
                                payload.as_ptr(),
                            );
                        }
                    }));
                    true
                })
            });
            *ACTIVE.lock().unwrap_or_else(|error| error.into_inner()) = Arc::downgrade(&channel);
            Some(channel)
        });
        let events = channel
            .as_deref()
            .map(|native| NativeEventChannel::new(native.clone()));
        *ACTIVE_EVENTS
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = events.clone();
        Self {
            legacy_open,
            callbacks,
            channel,
            events,
        }
    }

    pub(crate) fn channel(&self) -> Option<NativeChannel> {
        self.channel.as_deref().cloned()
    }

    pub(crate) fn events(&self) -> Option<NativeEventChannel> {
        self.events.clone()
    }

    pub(crate) fn url_handler(&self) -> impl FnOnce(&str) -> bool + use<> {
        let callbacks = self.callbacks;
        let legacy = self.legacy_open;
        move |url| {
            let Ok(url) = CString::new(url) else {
                return false;
            };
            if let Some(callbacks) = callbacks
                && let Some(open) = callbacks.open_url
            {
                return open(callbacks.context, url.as_ptr());
            }
            legacy.is_some_and(|open| open(url.as_ptr()))
        }
    }

    pub(crate) fn close(&mut self) {
        if let Some(events) = self.events.take() {
            events.close();
            *ACTIVE_EVENTS
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
        }
        if let Some(callbacks) = self.callbacks.take()
            && let Some(closed) = callbacks.closed
        {
            closed(callbacks.context);
        }
        if let Some(channel) = self.channel.take() {
            channel.close();
            *ACTIVE.lock().unwrap_or_else(|error| error.into_inner()) = Weak::new();
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn impasto_macos_service_event(
    topic: *const c_char,
    payload: *const c_char,
) -> bool {
    if topic.is_null() || payload.is_null() {
        return false;
    }
    // SAFETY: the host lends both NUL-terminated strings for this call only.
    let (Ok(topic), Ok(payload)) = (unsafe {
        (
            CStr::from_ptr(topic).to_str(),
            CStr::from_ptr(payload).to_str(),
        )
    }) else {
        return false;
    };
    let events = ACTIVE_EVENTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    events.is_some_and(|events| events.emit(topic, payload))
}

impl Drop for Host {
    fn drop(&mut self) {
        self.close();
    }
}

fn active() -> Option<Arc<NativeChannel>> {
    ACTIVE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .upgrade()
}

#[unsafe(no_mangle)]
extern "C" fn impasto_macos_service_pending(id: u64) -> bool {
    active().is_some_and(|channel| channel.contains(id))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn impasto_macos_service_result(
    id: u64,
    status: u32,
    payload: *const c_char,
) -> bool {
    let Some(channel) = active() else {
        return false;
    };
    let payload = if payload.is_null() {
        None
    } else {
        // SAFETY: the host guarantees a valid NUL-terminated string for the duration of this call.
        let Ok(text) = (unsafe { CStr::from_ptr(payload) }).to_str() else {
            return false;
        };
        Some(text.to_owned())
    };
    let result = if status == 0 {
        Ok(payload)
    } else {
        Err(payload.unwrap_or_else(|| "the platform request failed".into()))
    };
    channel.complete(id, result)
}
