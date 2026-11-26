//! Optional partial replay when a working texture is already required.

use engine::scene::LayerKind;

use super::{BlendKey, Clip, Coverage, Frame, Image, OutputFormat, POOL_BYTES, Scene, Status};
use crate::compiler::{IDENTITY, solid_rect};
use crate::{DamageRect, PixelSize};

const PARTIAL_LIMIT: f64 = 0.15;

#[derive(Clone)]
pub(super) struct RetainedRoot {
    pub image: Image,
    pub output: OutputFormat,
    pub revision: u64,
    pub submitted: bool,
}

impl RetainedRoot {
    pub(super) fn eligible(scene: &Scene<'_>, size: [u32; 2], format: wgpu::TextureFormat) -> bool {
        let texel = if format == wgpu::TextureFormat::Rgba16Float {
            8
        } else {
            4
        };
        let bytes = u64::from(size[0]) * u64::from(size[1]) * texel;
        bytes <= POOL_BYTES
            && scene.slice.is_none()
            && !scene.tree.has_renderer_work()
            && scene.tree.layers.iter().all(|layer| {
                matches!(
                    layer.kind,
                    LayerKind::Offset { .. }
                        | LayerKind::Transform { .. }
                        | LayerKind::Clip { .. }
                        | LayerKind::CornerClip { .. }
                        | LayerKind::Opacity { .. }
                        | LayerKind::Picture { .. }
                )
            })
            && scene
                .pictures
                .values()
                .all(|picture| picture.supports_partial)
    }

    pub(super) fn is_small(damage: DamageRect, size: [u32; 2]) -> bool {
        damage.fraction(PixelSize::new(size[0], size[1])) < PARTIAL_LIMIT
    }

    pub(super) fn damage(&self, scene: &Scene<'_>) -> DamageRect {
        if self.revision == scene.previous_revision && Self::is_small(scene.damage, self.image.size)
        {
            scene.damage
        } else {
            DamageRect::Full
        }
    }
}

impl Frame<'_> {
    pub(super) fn clear_damage(&mut self, target: &Image, area: [u32; 4]) -> Result<Clip, Status> {
        let scale = self.scene.scale;
        let [width, height] = target.size.map(|value| value as f32 / scale);
        // A scissored draw replaces the damaged pixels while retaining the
        // contents around them. The solid quad covers the entire scissor.
        self.rect_batch_from_paint(
            target,
            IDENTITY,
            &Coverage {
                scissor: Some(area),
                ..Coverage::default()
            },
            BlendKey::DstOut,
            (
                &[solid_rect([0.0, 0.0, width, height], [1.0; 4])],
                &[],
                None,
            ),
        )?;
        self.damage_scissor = Some(area);
        Ok(Clip::default())
    }
}
