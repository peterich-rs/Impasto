use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard};

use ui_core::Phase;

use crate::command::Command;

pub struct Inbox {
    state: Mutex<State>,
    cv: Condvar,
}

struct State {
    commands: VecDeque<Command>,
}

impl Inbox {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                commands: VecDeque::new(),
            }),
            cv: Condvar::new(),
        }
    }

    pub fn post(&self, command: Command) {
        let mut state = lock(&self.state);
        push_coalesced(&mut state.commands, command);
        self.cv.notify_one();
    }

    pub fn recv(&self) -> Option<Command> {
        let mut state = lock(&self.state);
        loop {
            if let Some(command) = state.commands.pop_front() {
                return Some(command);
            }
            state = self
                .cv
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

pub fn push_coalesced(commands: &mut VecDeque<Command>, command: Command) {
    if let Command::Pointer(next) = command {
        if next.phase == Phase::Move {
            if let Some(Command::Pointer(previous)) = commands.back_mut() {
                if previous.phase == Phase::Move && previous.pointer == next.pointer {
                    *previous = next;
                    return;
                }
            }
        }
    }
    commands.push_back(command);
}

fn lock(mutex: &Mutex<State>) -> MutexGuard<'_, State> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_core::{Phase, PointerEvent};

    fn move_event(x: f32, time_ms: f64) -> PointerEvent {
        PointerEvent::new(Phase::Move, x, 0.0, time_ms)
    }

    #[test]
    fn adjacent_moves_collapse_to_the_latest() {
        let mut commands = VecDeque::new();
        push_coalesced(&mut commands, Command::Pointer(move_event(1.0, 10.0)));
        push_coalesced(&mut commands, Command::Pointer(move_event(2.0, 18.0)));
        assert_eq!(commands.len(), 1);
        let Some(Command::Pointer(event)) = commands.front() else {
            panic!("expected a pointer");
        };
        assert_eq!(event.x, 2.0);
        assert_eq!(event.time_ms, 18.0);
    }

    #[test]
    fn a_move_after_a_down_stays_in_order() {
        let mut commands = VecDeque::new();
        push_coalesced(
            &mut commands,
            Command::Pointer(PointerEvent::new(Phase::Down, 0.0, 0.0, 0.0)),
        );
        push_coalesced(&mut commands, Command::Pointer(move_event(3.0, 4.0)));
        assert_eq!(commands.len(), 2);
    }

    #[test]
    fn wheels_stay_in_order() {
        let mut commands = VecDeque::new();
        push_coalesced(
            &mut commands,
            Command::Pointer(PointerEvent::wheel(1.0, 2.0, 0.0, 0.0, 4.0)),
        );
        push_coalesced(
            &mut commands,
            Command::Pointer(PointerEvent::wheel(1.0, 2.0, 8.0, 0.0, 6.0)),
        );
        assert_eq!(commands.len(), 2);
    }
}
