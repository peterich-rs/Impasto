//! Compositing layers. Pictures keep commands relative to their own origin.

use std::sync::Arc;

use crate::draw_list::{BlendMode, Command, DrawList};

/// Identity of one picture. Child pictures use a boundary node id.
/// A node's own commands use a different id so the two do not share a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PictureId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct LayerTree {
    pub layers: Vec<Layer>,
    pub pictures: Vec<Arc<Picture>>,
    pub root: LayerId,
    pub epoch: u64,
}

impl LayerTree {
    pub fn empty() -> Self {
        Self {
            layers: Vec::new(),
            pictures: Vec::new(),
            root: LayerId(0),
            epoch: 0,
        }
    }

    pub fn picture(&self, id: PictureId) -> Option<&Picture> {
        self.pictures
            .iter()
            .find(|picture| picture.id == id)
            .map(Arc::as_ref)
    }

    /// Image sources that will be drawn into some picture, including ones a
    /// viewport clip would drop from a flattened list.
    pub fn image_requirements(&self) -> Vec<(Arc<str>, bool)> {
        let mut commands = Vec::new();
        for picture in &self.pictures {
            commands.extend(picture.commands.iter().cloned());
        }
        DrawList { commands }.image_requirements()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub kind: LayerKind,
    pub parent: Option<LayerId>,
    pub first_child: Option<LayerId>,
    pub next: Option<LayerId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerKind {
    Offset {
        x: f32,
        y: f32,
    },
    Transform {
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale_x: f32,
        scale_y: f32,
    },
    Clip {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
    },
    Opacity {
        alpha: u8,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Blur {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sigma: f32,
    },
    BackdropBlur {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sigma: f32,
    },
    Blend {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mode: BlendMode,
    },
    Picture {
        picture: PictureId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub id: PictureId,
    pub generation: u64,
    /// Where the picture's local origin sits before this node's own scroll offset.
    /// `(0, 0)` means the commands are already in the parent layer's space.
    pub origin: (f32, f32),
    pub commands: Arc<[Command]>,
}
