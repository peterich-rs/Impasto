//! Compositing layers. Pictures keep commands relative to their own origin.

use std::sync::Arc;

use super::{BlendMode, Command, image};

/// Distinguishes a node’s own picture from its child picture.
pub const SELF_PICTURE: u64 = 1 << 63;

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
    pub tracks: Arc<[super::CompositorTrack]>,
    pub root: LayerId,
    pub epoch: u64,
}

impl LayerTree {
    pub fn empty() -> Self {
        Self {
            layers: Vec::new(),
            pictures: Vec::new(),
            tracks: Arc::default(),
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
        image::dedup(self.pictures.iter().flat_map(|picture| {
            picture
                .images()
                .iter()
                .map(|(source, required)| (source, *required))
        }))
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
    CornerClip {
        bounds: [f32; 4],
        corners: super::CornerShape,
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
    /// A native view interleaves the pictures before and after this node.
    PlatformView {
        node: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub id: PictureId,
    pub generation: u64,
    /// Where the picture's local origin sits before this node's own scroll offset.
    /// `(0, 0)` means the commands are already in the parent layer's space.
    pub origin: (f32, f32),
    commands: Arc<[Command]>,
    images: Vec<(Arc<str>, bool)>,
}

impl Picture {
    pub fn new(
        id: PictureId,
        generation: u64,
        origin: (f32, f32),
        commands: Arc<[Command]>,
    ) -> Self {
        let images = image::requirements(commands.iter());
        Self {
            id,
            generation,
            origin,
            commands,
            images,
        }
    }

    pub fn commands(&self) -> &Arc<[Command]> {
        &self.commands
    }

    pub fn images(&self) -> &[(Arc<str>, bool)] {
        &self.images
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, LayerTree, Picture, PictureId};
    use crate::scene::{CanvasBody, CanvasImageRef, CanvasIsolation, CanvasList, ContentMode};
    use std::sync::Arc;

    #[test]
    fn image_requirements_merge_canvas_and_required_picture_images() {
        let shared = Arc::<str>::from("shared.png");
        let canvas_only = Arc::<str>::from("canvas.png");
        let image_only = Arc::<str>::from("image.png");
        let canvas = Command::Canvas {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            list: CanvasList {
                body: Arc::new(CanvasBody {
                    ops: Box::new([]),
                    images: Box::new([
                        CanvasImageRef {
                            source: Arc::clone(&shared),
                        },
                        CanvasImageRef {
                            source: Arc::clone(&canvas_only),
                        },
                    ]),
                    isolation: CanvasIsolation::Shared,
                    bytes: 0,
                }),
            },
        };
        let image = |source, required| Command::Image {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            source,
            mode: ContentMode::Fill,
            required,
        };
        let first = Arc::new(Picture::new(
            PictureId(1),
            1,
            (0.0, 0.0),
            Arc::new([canvas, image(Arc::clone(&shared), false)]),
        ));
        let second = Arc::new(Picture::new(
            PictureId(2),
            1,
            (0.0, 0.0),
            Arc::new([
                image(Arc::clone(&shared), true),
                image(Arc::clone(&image_only), true),
            ]),
        ));
        assert_eq!(
            first.images(),
            &[
                (Arc::clone(&shared), false),
                (Arc::clone(&canvas_only), false)
            ]
        );
        let tree = LayerTree {
            pictures: vec![first, second],
            ..LayerTree::empty()
        };
        assert_eq!(
            tree.image_requirements(),
            vec![(shared, true), (canvas_only, false), (image_only, true)]
        );
    }
}
