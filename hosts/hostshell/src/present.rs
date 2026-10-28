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

/// Failures before this count retry on the next vsync. At and after it, the shell
/// waits [`present_retry_delay_ms`] and logs once until a present succeeds.
pub const PRESENT_BACKOFF_AFTER: u32 = 3;

/// Delay before the next present attempt. `None` means the next vsync.
pub fn present_retry_delay_ms(failures: u32) -> Option<i64> {
    if failures < PRESENT_BACKOFF_AFTER {
        return None;
    }
    let shift = (failures - PRESENT_BACKOFF_AFTER).min(6);
    Some(16_i64.saturating_mul(1_i64 << shift).min(1_000))
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

    #[test]
    fn present_backoff_starts_at_the_third_failure_and_caps() {
        assert_eq!(super::present_retry_delay_ms(1), None);
        assert_eq!(super::present_retry_delay_ms(2), None);
        assert_eq!(super::present_retry_delay_ms(3), Some(16));
        assert_eq!(super::present_retry_delay_ms(4), Some(32));
        assert_eq!(super::present_retry_delay_ms(40), Some(1_000));
    }
}
