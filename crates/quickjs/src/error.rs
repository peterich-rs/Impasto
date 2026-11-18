use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("quickjs runtime was not created")]
    Runtime,
    #[error("quickjs: {0}")]
    Script(String),
}
