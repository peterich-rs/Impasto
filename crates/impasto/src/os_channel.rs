//! One-shot requests to plugins registered on the platform window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use futures_channel::oneshot;

type ResultValue = Result<Option<String>, String>;
type Notification = Box<dyn FnOnce(ResultValue) + Send>;
type Dispatch = dyn Fn(u64, &str, &str) -> bool + Send + Sync;

enum Reply {
    Request(oneshot::Sender<ResultValue>),
    Notification(Notification),
}

/// Routes one-shot calls to window-specific native plugins.
#[derive(Clone)]
pub struct NativeChannel {
    dispatch: Arc<Dispatch>,
    state: Arc<Mutex<Pending>>,
}

#[derive(Default)]
struct Pending {
    next: u64,
    closed: bool,
    requests: HashMap<u64, Reply>,
}

struct Request<'a> {
    channel: &'a NativeChannel,
    id: u64,
}

impl Drop for Request<'_> {
    fn drop(&mut self) {
        let removed = self.channel.state().requests.remove(&self.id);
        drop(removed);
    }
}

impl NativeChannel {
    /// Creates a channel over the window's main-queue dispatch mechanism.
    #[doc(hidden)]
    pub fn new(dispatch: impl Fn(u64, &str, &str) -> bool + Send + Sync + 'static) -> Self {
        Self {
            dispatch: Arc::new(dispatch),
            state: Arc::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, Pending> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Calls a native plugin operation. Dropping the future retires its reply ID.
    pub async fn request(&self, operation: &str, payload: &str) -> ResultValue {
        let (sender, receiver) = oneshot::channel();
        let id = self.reserve(Reply::Request(sender))?;
        let _request = Request { channel: self, id };
        if !(self.dispatch)(id, operation, payload) {
            return Err("the platform window is unavailable".into());
        }
        receiver
            .await
            .map_err(|_| "the platform window detached".to_owned())?
    }

    fn reserve(&self, reply: Reply) -> Result<u64, String> {
        let mut state = self.state();
        if state.closed {
            return Err("the platform session is closed".into());
        }
        let id = state
            .next
            .checked_add(1)
            .ok_or("native request identifiers are exhausted")?;
        state.next = id;
        state.requests.insert(id, reply);
        Ok(id)
    }

    pub(crate) fn notify(
        &self,
        operation: &str,
        payload: &str,
        reply: Notification,
    ) -> Result<(), String> {
        let id = self.reserve(Reply::Notification(reply))?;
        if !(self.dispatch)(id, operation, payload) {
            let _guard = Request { channel: self, id };
            return Err("the platform window is unavailable".into());
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn contains(&self, id: u64) -> bool {
        self.state().requests.contains_key(&id)
    }

    #[doc(hidden)]
    pub fn complete(&self, id: u64, result: ResultValue) -> bool {
        let pending = self.state().requests.remove(&id);
        match pending {
            Some(Reply::Request(sender)) => sender.send(result).is_ok(),
            Some(Reply::Notification(notify)) => {
                notify(result);
                true
            }
            None => false,
        }
    }

    #[doc(hidden)]
    pub fn cancel_pending(&self) {
        let pending = std::mem::take(&mut self.state().requests);
        Self::detach(pending);
    }

    #[doc(hidden)]
    pub fn close(&self) {
        let pending = {
            let mut state = self.state();
            state.closed = true;
            std::mem::take(&mut state.requests)
        };
        Self::detach(pending);
    }

    fn detach(pending: HashMap<u64, Reply>) {
        for reply in pending.into_values() {
            if let Reply::Notification(notify) = reply {
                notify(Err("the platform window detached".into()));
            }
        }
    }
}
