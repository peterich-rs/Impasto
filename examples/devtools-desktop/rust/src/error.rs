use std::{fmt, io};

use impasto::ImpastoError;

#[derive(Debug)]
pub struct DesktopError {
    code: &'static str,
    message: String,
}

impl DesktopError {
    pub(crate) fn new(error: impl ToString) -> Self {
        Self::with_code("invalid_input", error)
    }

    pub(crate) fn with_code(code: &'static str, error: impl ToString) -> Self {
        Self {
            code,
            message: error.to_string(),
        }
    }

    pub(crate) fn disconnected() -> Self {
        Self::with_code("disconnected", "No live target is connected")
    }
}

impl fmt::Display for DesktopError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.message)
    }
}

impl std::error::Error for DesktopError {}

impl ImpastoError for DesktopError {
    fn scope(&self) -> &'static str {
        "impasto:devtools-desktop"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}

impl From<io::Error> for DesktopError {
    fn from(error: io::Error) -> Self {
        Self::with_code("io", error)
    }
}

impl From<serde_json::Error> for DesktopError {
    fn from(error: serde_json::Error) -> Self {
        Self::with_code("protocol", error)
    }
}

impl From<impasto_file_system::FileError> for DesktopError {
    fn from(error: impasto_file_system::FileError) -> Self {
        Self::with_code("io", error)
    }
}
