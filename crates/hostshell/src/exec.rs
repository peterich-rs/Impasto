//! [`session::MethodExec`] backed by the process runtime and an optional main poster.

use std::sync::Arc;

use session::{MethodExec, PluginResult, Settle, Work};

use crate::command::Command;
use crate::queue::Inbox;

pub type MainThreadPoster = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

pub struct RtExec {
    pub rt: tokio::runtime::Handle,
    pub main: Option<MainThreadPoster>,
}

impl MethodExec for RtExec {
    fn worker(&self, settle: Settle, work: Work) {
        let _ = self.rt.spawn_blocking(move || settle.send(work()));
    }

    fn main_thread(&self, settle: Settle, work: Work) {
        match self.main.clone() {
            Some(post) => post(Box::new(move || settle.send(work()))),
            None => settle.send(PluginResult::Failed {
                message: "module needs a main-thread host".into(),
            }),
        }
    }

    fn now(&self, settle: Settle, result: PluginResult) {
        settle.send(result);
    }
}

pub fn settle_factory(inbox: Arc<Inbox>) -> Arc<dyn Fn(u64) -> Settle + Send + Sync> {
    Arc::new(move |token| {
        let inbox = Arc::clone(&inbox);
        Settle::new(move |result| inbox.post(Command::PluginDone { token, result }))
    })
}
