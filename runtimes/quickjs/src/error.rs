use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("quickjs is not linked; run runtimes/quickjs/fetch-quickjs.sh")]
    NotLinked,
    #[error("quickjs runtime was not created")]
    Runtime,
    #[error("quickjs: {0}")]
    Script(String),
}
