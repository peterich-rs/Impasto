use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::BootError;
use crate::command::Command;
use crate::host::UiHost;
use crate::input::push_coalesced;

pub(crate) type Completion = Box<dyn FnOnce(&mut UiHost) -> Result<bool, BootError> + Send>;

type Notify = Arc<dyn Fn() + Send + Sync>;

pub struct Inbox {
    state: Mutex<State>,
}

struct State {
    commands: VecDeque<Command>,
    completions: VecDeque<Completion>,
    closed: bool,
    notify: Option<Notify>,
}

impl Inbox {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                commands: VecDeque::new(),
                completions: VecDeque::new(),
                closed: false,
                notify: None,
            }),
        }
    }

    pub fn bind(&self, notify: Notify) {
        let pending = {
            let mut state = lock(&self.state);
            state.notify = Some(Arc::clone(&notify));
            !state.commands.is_empty() || !state.completions.is_empty()
        };
        if pending {
            notify();
        }
    }

    pub fn post(&self, command: Command) {
        self.post_if(command, |_| true);
    }

    pub(crate) fn post_plugin_event(&self, event: impasto::abi::HostEvent) -> bool {
        let bytes = event
            .module
            .len()
            .saturating_add(event.event.len())
            .saturating_add(event.payload.payload_bytes());
        bytes <= 256 * 1024
            && self.post_if(
                Command::Io(engine::session::IoEvent::Host(event)),
                |state| {
                    state
                        .commands
                        .iter()
                        .filter(|command| {
                            matches!(command, Command::Io(engine::session::IoEvent::Host(_)))
                        })
                        .take(64)
                        .count()
                        < 64
                },
            )
    }

    fn post_if(&self, command: Command, accept: impl FnOnce(&State) -> bool) -> bool {
        let notify = {
            let mut state = lock(&self.state);
            if state.closed || !accept(&state) {
                return false;
            }
            let notify = (state.commands.is_empty() && state.completions.is_empty())
                .then(|| state.notify.clone())
                .flatten();
            push_coalesced(&mut state.commands, command);
            notify
        };
        if let Some(notify) = notify {
            notify();
        }
        true
    }

    pub(crate) fn complete(&self, completion: Completion) {
        let notify = {
            let mut state = lock(&self.state);
            if state.closed {
                return;
            }
            state.completions.push_back(completion);
            state.notify.clone()
        };
        if let Some(notify) = notify {
            notify();
        }
    }

    pub(crate) fn take_completion(&self) -> Option<Completion> {
        lock(&self.state).completions.pop_front()
    }

    pub fn close(&self) {
        let commands = {
            let mut state = lock(&self.state);
            state.closed = true;
            state.notify = None;
            (
                std::mem::take(&mut state.commands),
                std::mem::take(&mut state.completions),
            )
        };
        drop(commands);
    }

    pub fn discard_surface(&self, id: crate::SurfaceId) {
        lock(&self.state).commands.retain(|command| match command {
            Command::SurfaceReady(target) => target.id != id,
            Command::SurfaceChanged { id: queued, .. } => *queued != id,
            _ => true,
        });
    }

    pub fn pop(&self) -> Option<Command> {
        lock(&self.state).commands.pop_front()
    }

    pub fn has_pending(&self) -> bool {
        let state = lock(&self.state);
        !state.commands.is_empty() || !state.completions.is_empty()
    }
}

fn lock(mutex: &Mutex<State>) -> MutexGuard<'_, State> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
