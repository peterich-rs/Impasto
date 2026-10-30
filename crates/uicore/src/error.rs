use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Layout(#[from] taffy::TaffyError),
    #[error("node is not in this tree")]
    UnknownNode,
    #[error("the tree has no root")]
    NoRoot,
    #[error("this node cannot have children")]
    NotAParent,
    #[error("node is not a child of this parent")]
    NotAChild,
    #[error("this node is not text")]
    NotText,
    #[error("node is the wrong kind for this operation")]
    WrongKind,
    #[error("node is not focusable")]
    NotFocusable,
    #[error("flex grow must be non-negative")]
    NegativeGrow,
    #[error("numeric value is not usable")]
    BadNumber,
}
