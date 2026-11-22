//! [`engine::session::MethodExec`] backed by the process runtime and an optional main poster.

use std::sync::Arc;

use engine::session::{IoEvent, MethodExec, PluginResult, Settle, Work};

use crate::command::Command;
use crate::queue::Inbox;

use crate::TaskRunner;

pub struct RtExec {
    pub io: Arc<dyn TaskRunner>,
    pub main: Option<Arc<dyn TaskRunner>>,
}

impl MethodExec for RtExec {
    fn worker(&self, settle: Settle, work: Work) {
        self.io.post(Box::new(move || settle.send(work())));
    }

    fn main_thread(&self, settle: Settle, work: Work) {
        match &self.main {
            Some(main) if main.runs_on_current_thread() => settle.send(work()),
            Some(main) => {
                main.post(Box::new(move || settle.send(work())));
            }
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
        Settle::new(move |result| inbox.post(Command::Io(IoEvent::Plugin { token, result })))
    })
}
