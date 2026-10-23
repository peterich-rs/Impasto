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
}
