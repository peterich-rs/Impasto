use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("buffer is shorter than the encoded scene")]
    ShortBuffer,
    #[error("parent is not in the scene")]
    UnknownParent,
    #[error("text is longer than a protocol string")]
    TextTooLong,
    #[error("scene has too many nodes")]
    TooManyNodes,
    #[error("scene is shorter than a header")]
    Truncated,
    #[error("magic is not UIFW")]
    BadMagic,
    #[error("version is not 1")]
    BadVersion,
    #[error("node kind is not valid")]
    BadKind,
    #[error("node id is not creation order")]
    BadId,
    #[error("bytes remain after the last node")]
    Trailing,
    #[error("string is not utf-8")]
    BadUtf8,
}
