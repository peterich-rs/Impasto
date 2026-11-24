//! Compositing layers. Pictures keep commands relative to their own origin.

use std::{collections::HashMap, sync::Arc};

use super::{
    BackdropMaterial, BlendMode, ColorMatrix, Command, ShaderProgram, ShaderProgramId, image,
};

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
    pub shader_uniforms:
        HashMap<LayerId, [Option<f32>; super::MAX_SHADER_UNIFORM_BYTES / size_of::<f32>()]>,
    pub root: LayerId,
    pub epoch: u64,
    /// Programs in the session that recorded this tree.
    pub shader_programs: Arc<[ShaderProgram]>,
}

impl LayerTree {
    pub fn empty() -> Self {
        Self {
            layers: Vec::new(),
            pictures: Vec::new(),
            tracks: Arc::default(),
            shader_uniforms: HashMap::new(),
            root: LayerId(0),
            epoch: 0,
            shader_programs: Arc::from([]),
        }
    }

    pub fn picture(&self, id: PictureId) -> Option<&Picture> {
        self.pictures
            .iter()
            .find(|picture| picture.id == id)
            .map(Arc::as_ref)
    }

    /// Resolves a program within this snapshot's session.
    pub fn shader_program(&self, id: ShaderProgramId) -> Option<&ShaderProgram> {
        self.shader_programs.iter().find(|program| program.id == id)
    }

    /// Returns whether one of the pictures uses an effect driven by the frame clock.
    pub fn has_animated_effects(&self) -> bool {
        self.shader_programs.iter().any(|program| program.animated)
            && self.pictures.iter().any(|picture| {
                picture.commands().iter().any(|command| {
                    matches!(command, Command::ShaderEffect { shader, .. }
                        if self.shader_program(*shader).is_some_and(|program| program.animated))
                })
            })
    }

    /// Returns whether any picture samples a texture from a same-device producer.
    pub fn has_gpu_views(&self) -> bool {
        self.pictures.iter().any(|picture| {
            picture
                .images()
                .iter()
                .any(|(source, _)| source.starts_with(image::GPU_VIEW_TEXTURE_PREFIX))
        })
    }

    /// Returns whether drawing may change on vsync without a new UI tree.
    pub fn has_renderer_work(&self) -> bool {
        self.has_animated_effects() || self.has_gpu_views()
    }

    /// Destination sizes in scene points for sources that can be decoded smaller.
    /// Canvas, shaders and pixel-addressed images require the original source.
    pub fn image_decode_extents(&self) -> HashMap<Arc<str>, [f32; 2]> {
        let mut scales = HashMap::<PictureId, f32>::new();
        let mut pending = vec![(self.root, 1.0_f32)];
        let mut visited = vec![false; self.layers.len()];
        while let Some((id, scale)) = pending.pop() {
            let Some(layer) = self.layers.get(id.0 as usize) else {
                return HashMap::new();
            };
            if std::mem::replace(&mut visited[id.0 as usize], true) {
                return HashMap::new();
            }
            if let Some(next) = layer.next {
                pending.push((next, scale));
            }
            let scale = match layer.kind {
                LayerKind::Transform {
                    scale_x, scale_y, ..
                } => scale * scale_x.abs().max(scale_y.abs()).max(1.0),
                LayerKind::Picture { picture } => {
                    scales
                        .entry(picture)
                        .and_modify(|value| *value = value.max(scale))
                        .or_insert(scale);
                    scale
                }
                _ => scale,
            };
            if let Some(child) = layer.first_child {
                pending.push((child, scale));
            }
        }
        let mut extents = HashMap::<Arc<str>, Option<[f32; 2]>>::new();
        for picture in &self.pictures {
            let Some(&scale) = scales.get(&picture.id) else {
                continue;
            };
            let can_resize = !picture.commands.iter().any(|command| {
                matches!(
                    command, Command::PushTransform { scale_x, scale_y, .. }
                    if scale_x.abs() > 1.0 || scale_y.abs() > 1.0
                )
            });
            for command in &*picture.commands {
                for (source, _) in image::uses(command) {
                    let size = match command {
                        Command::Image {
                            w: width,
                            h: height,
                            mode,
                            ..
                        } if can_resize
                            && !matches!(mode, image::ContentMode::NineSlice { .. })
                            && width.is_finite()
                            && height.is_finite()
                            && *width > 0.0
                            && *height > 0.0 =>
                        {
                            Some([width * scale, height * scale])
                        }
                        _ => None,
                    };
                    extents
                        .entry(Arc::clone(source))
                        .and_modify(|existing| {
                            *existing = existing
                                .zip(size)
                                .map(|([a, b], [c, d])| [a.max(c), b.max(d)]);
                        })
                        .or_insert(size);
                }
            }
        }
        extents
            .into_iter()
            .filter_map(|(key, extent)| extent.map(|extent| (key, extent)))
            .collect()
    }

    /// Image sources that will be drawn into some picture, including ones a
    /// viewport clip would drop from a flattened list.
    pub fn image_requirements(&self) -> Vec<(Arc<str>, bool)> {
        image::dedup(self.pictures.iter().flat_map(|picture| {
            picture
                .images()
                .iter()
                .filter(|(source, _)| !image::is_inline_source(source))
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
    /// A solid background with a premultiplied linear color.
    Fill {
        bounds: [f32; 4],
        radius: f32,
        corners: Option<super::CornerShape>,
        color: [f32; 4],
    },
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
    ColorFilter {
        matrix: Arc<ColorMatrix>,
    },
    BackdropBlur {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sigma: f32,
    },
    BackdropMaterial {
        bounds: [f32; 4],
        corners: Option<super::CornerShape>,
        material: Arc<BackdropMaterial>,
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
