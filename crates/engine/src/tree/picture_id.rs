use super::NodeId;
use crate::scene::{PictureId, SELF_PICTURE};

pub fn child_picture_id(id: NodeId) -> PictureId {
    PictureId(id.to_raw())
}

pub fn self_picture_id(id: NodeId) -> PictureId {
    PictureId(id.to_raw() | SELF_PICTURE)
}
