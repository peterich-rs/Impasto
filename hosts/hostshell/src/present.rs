//! What the shell does after one present attempt.
//!
//! [`engine::Status::PresentFailed`] keeps the surface and asks for another
//! vsync. [`engine::Status::NotReady`] waits until the platform posts a new surface.

use crate::Presented;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterPresent {
    Shown { again: bool },
    Stop,
    Retry,
    WaitForSurface,
}

pub fn after_present(result: Result<Presented, engine::Status>, again: bool) -> AfterPresent {
    match result {
        Ok(Presented::Continue) => AfterPresent::Shown { again },
        Ok(Presented::Stop) => AfterPresent::Stop,
        Err(engine::Status::PresentFailed) => AfterPresent::Retry,
        Err(engine::Status::NotReady | engine::Status::NotLinked | engine::Status::BadScene) => {
            AfterPresent::WaitForSurface
        }
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
            AfterPresent::WaitForSurface
        );
        assert_eq!(
            after_present(Ok(Presented::Continue), true),
            AfterPresent::Shown { again: true }
        );
    }
}
