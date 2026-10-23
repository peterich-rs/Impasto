//! Primary-pointer filter. Extra fingers and historical samples stay out of the queue.

use core::Phase;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Down,
    Move,
    Up,
    Cancel,
    PointerDown,
    PointerUp,
    Other,
}

/// `PointerDown` and `PointerUp` are dropped whole, including index 0.
pub fn phase_for(action: Action) -> Option<Phase> {
    match action {
        Action::Down => Some(Phase::Down),
        Action::Move => Some(Phase::Move),
        Action::Up => Some(Phase::Up),
        Action::Cancel => Some(Phase::Cancel),
        Action::PointerDown | Action::PointerUp | Action::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_primary_phases_pass() {
        assert_eq!(phase_for(Action::Down), Some(Phase::Down));
        assert_eq!(phase_for(Action::Move), Some(Phase::Move));
        assert_eq!(phase_for(Action::Up), Some(Phase::Up));
        assert_eq!(phase_for(Action::Cancel), Some(Phase::Cancel));
        assert_eq!(phase_for(Action::PointerDown), None);
        assert_eq!(phase_for(Action::PointerUp), None);
        assert_eq!(phase_for(Action::Other), None);
    }
}
