use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::BootError;
use crate::command::Command;
use crate::host::UiHost;
use crate::input::push_coalesced;

pub(crate) enum Work {
    Completions(Vec<Completion>),
    Command(Command),
}

pub(crate) type Completion = Box<dyn FnOnce(&mut UiHost) -> Result<bool, BootError> + Send>;

type Notify = Arc<dyn Fn() + Send + Sync>;

pub struct Inbox {
    state: Mutex<State>,
    retired_surface: Arc<AtomicU64>,
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
            retired_surface: Arc::new(AtomicU64::new(0)),
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

    // The platform is already executing the only consumer. It drains the
    // commands queued before this tick together with the tick itself.
    pub(crate) fn post_inline_frame(&self, command: Command) {
        let mut state = lock(&self.state);
        if !state.closed {
            push_coalesced(&mut state.commands, command);
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn post_devtools(&self, command: Command) -> bool {
        const QUERY_LIMIT: usize = 32;
        self.post_if(command, |state| {
            state
                .commands
                .iter()
                .filter(|command| matches!(command, Command::Devtools { .. }))
                .take(QUERY_LIMIT)
                .count()
                < QUERY_LIMIT
        })
    }

    #[cfg(debug_assertions)]
    pub(crate) fn post_development(&self, command: Command) -> bool {
        self.post_if(command, |state| {
            !state
                .commands
                .iter()
                .any(|command| matches!(command, Command::Development { .. }))
        })
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

    pub(crate) fn take_work(&self, limit: usize) -> Option<Work> {
        let mut state = lock(&self.state);
        let count = state.completions.len().min(limit).min(8);
        if count != 0 {
            return Some(Work::Completions(
                state.completions.drain(..count).collect(),
            ));
        }
        state.commands.pop_front().map(Work::Command)
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

    pub(crate) fn surface_gate(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.retired_surface)
    }

    pub(crate) fn admits_surface(&self, id: crate::SurfaceId) -> bool {
        id.0 > self.retired_surface.load(Ordering::Acquire)
    }

    pub fn discard_surface(&self, id: crate::SurfaceId) {
        self.retired_surface.fetch_max(id.0, Ordering::AcqRel);
        lock(&self.state).commands.retain(|command| match command {
            Command::SurfaceReady(target) => target.id != id,
            Command::SurfaceChanged { id: queued, .. } => *queued != id,
            _ => true,
        });
    }

    #[cfg(test)]
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
