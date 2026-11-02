use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    #[error("impeller is not linked")]
    NotLinked,
    #[error("scene is not drawable")]
    BadScene,
    #[error("frame is not ready")]
    NotReady,
    #[error("present failed")]
    PresentFailed,
    /// The drawable or swapchain image was not acquired. Skip this present.
    /// The surface stays. Consecutive misses rebuild a Vulkan chain.
    #[error("acquire missed")]
    AcquireMiss,
}

/// Consecutive null acquires before the swapchain is dropped and rebuilt.
pub const ACQUIRE_REBUILD_AFTER: u32 = 3;

/// What to do after another null acquire. The chain is not touched on [`Skip`](AcquireFollowup::Skip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcquireFollowup {
    Skip,
    Rebuild,
}

/// `consecutive` is the miss count including the one just observed.
pub fn acquire_followup(consecutive: u32) -> AcquireFollowup {
    if consecutive >= ACQUIRE_REBUILD_AFTER {
        AcquireFollowup::Rebuild
    } else {
        AcquireFollowup::Skip
    }
}

#[cfg(test)]
mod tests {
    use super::{AcquireFollowup, acquire_followup};

    #[test]
    fn a_null_acquire_skips_until_the_chain_is_rebuilt() {
        assert_eq!(acquire_followup(1), AcquireFollowup::Skip);
        assert_eq!(acquire_followup(2), AcquireFollowup::Skip);
        assert_eq!(acquire_followup(3), AcquireFollowup::Rebuild);
    }
}
