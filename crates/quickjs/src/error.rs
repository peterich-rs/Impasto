use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("quickjs is not linked; run crates/quickjs/fetch-quickjs.sh")]
    NotLinked,
    #[error("quickjs runtime was not created")]
    Runtime,
    #[error("quickjs: {0}")]
    Script(String),
}
