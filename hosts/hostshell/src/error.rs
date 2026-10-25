use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Info,
}

#[derive(Debug, Error, Clone)]
pub enum SessionError {
    #[error("quickjs is not linked")]
    NotLinked,
    #[error("quickjs runtime was not created")]
    Runtime,
    #[error("quickjs: {0}")]
    Script(String),
}

#[derive(Debug, Error, Clone)]
pub enum BootError {
    #[error("impeller context was not created: {0}")]
    Engine(engine::Status),
    #[error("font {0} was not provided")]
    Font(String),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("render thread panicked")]
    Panicked,
}

impl BootError {
    /// Process status from the startup table: engine 2, runtime 3, script 4.
    pub fn code(&self) -> u8 {
        match self {
            Self::Engine(_) | Self::Font(_) | Self::Panicked => 2,
            Self::Session(SessionError::NotLinked | SessionError::Runtime) => 3,
            Self::Session(SessionError::Script(_)) => 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BootError, SessionError};

    #[test]
    fn startup_failures_use_the_process_status_table() {
        assert_eq!(BootError::Font("UIFWSans".to_string()).code(), 2);
        assert_eq!(BootError::Engine(engine::Status::NotLinked).code(), 2);
        assert_eq!(BootError::Session(SessionError::NotLinked).code(), 3);
        assert_eq!(BootError::Session(SessionError::Runtime).code(), 3);
        assert_eq!(
            BootError::Session(SessionError::Script("bad".to_string())).code(),
            4
        );
    }
}
