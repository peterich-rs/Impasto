//! What the shell does after one present attempt.
//!
//! A present error keeps the surface. The surface is dropped only when the
//! platform posts [`crate::Command::SurfaceDestroyed`].

use crate::Presented;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterPresent {
    Shown {
        again: bool,
    },
    Stop,
    /// Drawable or display list was not ready, or the list was rejected.
    /// The surface stays.
    Retry,
    /// The engine context is gone.
    Fatal,
}

pub fn after_present(result: Result<Presented, engine::Status>, again: bool) -> AfterPresent {
    match result {
        Ok(Presented::Continue) => AfterPresent::Shown { again },
        Ok(Presented::Stop) => AfterPresent::Stop,
        Err(
            engine::Status::PresentFailed | engine::Status::NotReady | engine::Status::BadScene,
        ) => AfterPresent::Retry,
        Err(engine::Status::NotLinked) => AfterPresent::Fatal,
    }
}

#[cfg(test)]
mod tests {
    use super::{AfterPresent, after_present};
    use crate::Presented;

    #[test]
    fn a_failed_present_retries_and_a_missing_surface_waits() {
        assert_eq!(
            after_present(Err(engine::Status::PresentFailed), false),
            AfterPresent::Retry
        );
        assert_eq!(
            after_present(Err(engine::Status::NotReady), true),
            AfterPresent::Retry
        );
        assert_eq!(
            after_present(Err(engine::Status::BadScene), false),
            AfterPresent::Retry
        );
        assert_eq!(
            after_present(Err(engine::Status::NotLinked), false),
            AfterPresent::Fatal
        );
        assert_eq!(
            after_present(Ok(Presented::Continue), true),
            AfterPresent::Shown { again: true }
        );
    }
}
