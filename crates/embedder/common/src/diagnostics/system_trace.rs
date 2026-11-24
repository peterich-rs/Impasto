//! Bridges `tracing` spans and events to platform profilers (Apple `os_signpost` and Android `ATrace`).

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::fmt::Write as _;

use tracing::field::{Field, Visit};
use tracing::span::Id;
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;

#[cfg(target_vendor = "apple")]
mod apple {
    use std::ffi::{CStr, c_char};

    #[allow(unsafe_code, reason = "Apple signpost shim with a C ABI")]
    unsafe extern "C" {
        fn impasto_signpost_enabled() -> bool;
        fn impasto_signpost_begin(name: *const c_char) -> u64;
        fn impasto_signpost_end(id: u64);
    }

    #[allow(unsafe_code, reason = "Apple signpost shim with a C ABI")]
    pub(super) fn enabled() -> bool {
        // SAFETY: the shim initializes its log once and takes no arguments.
        unsafe { impasto_signpost_enabled() }
    }

    #[allow(unsafe_code, reason = "Apple signpost shim with a C ABI")]
    pub(super) fn begin(name: &CStr) -> u64 {
        // SAFETY: name is NUL-terminated and remains live throughout the call.
        unsafe { impasto_signpost_begin(name.as_ptr()) }
    }

    #[allow(unsafe_code, reason = "Apple signpost shim with a C ABI")]
    pub(super) fn end(id: u64) {
        // SAFETY: callers forward only nonzero IDs returned by begin.
        unsafe { impasto_signpost_end(id) };
    }
}

#[cfg(target_os = "android")]
mod android {
    use std::ffi::CStr;

    #[allow(unsafe_code, reason = "Android platform trace API")]
    pub(super) fn enabled() -> bool {
        // SAFETY: no arguments or thread affinity for the enabled query.
        unsafe { ndk_sys::ATrace_isEnabled() }
    }

    #[allow(unsafe_code, reason = "Android platform trace API")]
    pub(super) fn begin(name: &CStr) -> u64 {
        // SAFETY: NDK ATrace is available since API 23; name is NUL-terminated and live for this call.
        unsafe {
            if ndk_sys::ATrace_isEnabled() {
                ndk_sys::ATrace_beginSection(name.as_ptr());
                1
            } else {
                0
            }
        }
    }

    #[allow(unsafe_code, reason = "Android platform trace API")]
    pub(super) fn end(token: u64) {
        if token != 0 {
            // SAFETY: begin opened a section on this thread and the token is thread-local.
            unsafe { ndk_sys::ATrace_endSection() };
        }
    }
}

fn platform_enabled() -> bool {
    #[cfg(target_vendor = "apple")]
    {
        apple::enabled()
    }
    #[cfg(target_os = "android")]
    {
        android::enabled()
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
    {
        false
    }
}

fn begin_section(_name: &CStr) -> u64 {
    #[cfg(target_vendor = "apple")]
    {
        apple::begin(_name)
    }
    #[cfg(target_os = "android")]
    {
        android::begin(_name)
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
    {
        0
    }
}

fn end_section(_token: u64) {
    #[cfg(target_vendor = "apple")]
    if _token != 0 {
        apple::end(_token);
    }
    #[cfg(target_os = "android")]
    if _token != 0 {
        android::end(_token);
    }
}

fn interested(metadata: &Metadata<'_>) -> bool {
    metadata.target().starts_with("impasto::")
        && (metadata.is_span() || *metadata.level() <= Level::DEBUG)
}

thread_local! {
    static TOKENS: RefCell<Vec<(u64, u64)>> = const { RefCell::new(Vec::new()) };
}

fn span_label(metadata: &Metadata<'static>) -> &'static str {
    match metadata.name() {
        "frame" => "Impasto frame",
        "raster" => "Impasto raster",
        "input" => "Impasto input",
        "script" => "Impasto script",
        "reactive" => "Impasto reactive",
        "reactive_post" => "Impasto reactive after frame callbacks",
        "layout" => "Impasto layout",
        "list_op" => "Impasto ListOp",
        "record_layers" => "Impasto record",
        "canvas_prepare" => "Impasto Canvas prepare",
        "canvas_draw" => "Impasto Canvas draw",
        "submit" => "Impasto submit",
        "present" => "Impasto present",
        name => name,
    }
}

struct SystemTraceFilter;

impl<S> Filter<S> for SystemTraceFilter {
    fn enabled(&self, metadata: &Metadata<'_>, _cx: &Context<'_, S>) -> bool {
        platform_enabled() && interested(metadata)
    }

    fn callsite_enabled(&self, metadata: &'static Metadata<'static>) -> Interest {
        if cfg!(any(target_vendor = "apple", target_os = "android")) && interested(metadata) {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }
}

struct SystemTraceLayer;

impl<S> Layer<S> for SystemTraceLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        let token = if platform_enabled()
            && let Some(span) = ctx.span(id)
            && let Ok(name) = CString::new(span_label(span.metadata()))
        {
            begin_section(&name)
        } else {
            0
        };
        TOKENS.with_borrow_mut(|stack| stack.push((id.into_u64(), token)));
    }

    fn on_exit(&self, id: &Id, _ctx: Context<'_, S>) {
        let _ = TOKENS.try_with(|stack| {
            let mut stack = stack.borrow_mut();
            let Some(index) = stack.iter().rposition(|&(span, _)| span == id.into_u64()) else {
                return;
            };
            if cfg!(target_os = "android") {
                // ATrace sections use a native thread-local stack. Close any
                // intervening sections to keep begin/end pairs balanced.
                for (_, token) in stack.drain(index..).rev() {
                    end_section(token);
                }
            } else {
                end_section(stack.remove(index).1);
            }
        });
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if !platform_enabled() {
            return;
        }
        let mut text = String::new();
        let message = {
            let mut visitor = EventFormat {
                out: &mut text,
                message: String::new(),
            };
            event.record(&mut visitor);
            visitor.message
        };
        let label = if message.is_empty() {
            if text.is_empty() {
                event.metadata().name().to_owned()
            } else {
                format!("{}{text}", event.metadata().name())
            }
        } else if text.is_empty() {
            message
        } else {
            format!("{message}{text}")
        };
        // Android reserves the rest of the record for the trace marker envelope.
        let limit = label.floor_char_boundary(127.min(label.len()));
        if let Ok(name) = CString::new(&label[..limit]) {
            let token = begin_section(&name);
            if token != 0 {
                end_section(token);
            }
        }
    }
}

struct EventFormat<'a> {
    out: &'a mut String,
    message: String,
}

impl Visit for EventFormat<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            let _ = write!(self.out, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            let _ = write!(self.out, " {}={value:?}", field.name());
        }
    }
}

pub(crate) fn layer<S>() -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    SystemTraceLayer.with_filter(SystemTraceFilter)
}
