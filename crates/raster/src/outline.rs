//! Device-local tessellated glyph outlines for large or scaled text.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use engine::scene::{CanvasComposite, CanvasPath, FillRule, FontFaceId, PathVerb};
use swash::{
    FontRef,
    scale::ScaleContext,
    zeno::{Angle, Command, PathData, Transform},
};

use crate::{
    FontLibrary, Status,
    batch::{Brush, GpuMeshBuffer, Mesh},
    compiler::{GlyphDraw, mesh_bounds, tessellate},
    gpu::Gpu,
    pipelines,
};

pub const REFERENCE_PPEM: f32 = 128.0;
const CACHE_LIMIT: usize = 512;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    face: FontFaceId,
    glyph: u16,
    coords: Option<Arc<[i16]>>,
    embolden: bool,
    skew: Option<u32>,
}

#[derive(Default)]
pub struct Outlines {
    cache: HashMap<Key, Option<Mesh>>,
    scale: ScaleContext,
    #[cfg(test)]
    disabled: bool,
}

impl Outlines {
    #[cfg(test)]
    pub fn disable(&mut self) {
        self.disabled = true;
    }

    pub fn glyph(
        &mut self,
        gpu: &Gpu,
        library: &FontLibrary,
        glyph: &GlyphDraw,
    ) -> Result<(Option<Mesh>, u64), Status> {
        #[cfg(test)]
        if self.disabled {
            return Ok((None, 0));
        }
        let key = Key {
            face: *glyph.face,
            glyph: glyph.glyph,
            coords: glyph.coords.clone(),
            embolden: glyph.embolden,
            skew: glyph.skew.map(f32::to_bits),
        };
        if let Some(mesh) = self.cache.get(&key) {
            return Ok((mesh.clone(), 0));
        }
        let mesh = library
            .with_face(&key.face, |bytes, index| {
                self.path(bytes, index, &key)
                    .map(|path| upload(gpu, &path))
                    .transpose()
            })
            .transpose()?
            .flatten()
            .flatten();
        let bytes = mesh
            .as_ref()
            .and_then(|mesh| mesh.buffer.get())
            .map_or(0, |buffer| buffer.vertices.size() + buffer.indices.size());
        if self.cache.len() >= CACHE_LIMIT
            && let Some(old) = self.cache.keys().next().cloned()
        {
            self.cache.remove(&old);
        }
        self.cache.insert(key, mesh.clone());
        Ok((mesh, bytes))
    }

    fn path(&mut self, bytes: &[u8], index: u32, key: &Key) -> Option<CanvasPath> {
        let font = FontRef::from_index(bytes, index as usize)?;
        let mut scaler = self
            .scale
            .builder(font)
            .size(REFERENCE_PPEM)
            .normalized_coords(key.coords.as_deref().unwrap_or_default().iter().copied())
            .build();
        if scaler.has_color_bitmaps() || scaler.has_color_outlines() {
            return None;
        }
        let mut outline = scaler.scale_outline(key.glyph)?;
        if key.embolden {
            outline.embolden(REFERENCE_PPEM / 24.0, REFERENCE_PPEM / 24.0);
        }
        if let Some(skew) = key.skew.map(f32::from_bits) {
            outline.transform(&Transform::skew(Angle::from_degrees(-skew), Angle::ZERO));
        }
        let verbs = outline
            .path()
            .commands()
            .map(|command| match command {
                Command::MoveTo(p) => PathVerb::Move(p.x, -p.y),
                Command::LineTo(p) => PathVerb::Line(p.x, -p.y),
                Command::QuadTo(c, p) => PathVerb::Quad(c.x, -c.y, p.x, -p.y),
                Command::CurveTo(c1, c2, p) => PathVerb::Cubic(c1.x, -c1.y, c2.x, -c2.y, p.x, -p.y),
                Command::Close => PathVerb::Close,
            })
            .collect();
        Some(CanvasPath {
            verbs,
            fill: FillRule::NonZero,
        })
    }
}

fn upload(gpu: &Gpu, path: &CanvasPath) -> Result<Option<Mesh>, Status> {
    let Ok(geometry) = tessellate(path, 0.035) else {
        return Ok(None);
    };
    if geometry.indices.is_empty() {
        return Ok(None);
    }
    let bounds = mesh_bounds(&geometry.vertices);
    let count = u32::try_from(geometry.indices.len()).map_err(|_| Status::OutOfMemory)?;
    let buffers = gpu.validation(|| {
        Ok(GpuMeshBuffer {
            vertices: pipelines::buffer_from(
                gpu,
                "glyph outline vertices",
                bytemuck::cast_slice(&geometry.vertices),
                wgpu::BufferUsages::VERTEX,
            )?,
            indices: pipelines::buffer_from(
                gpu,
                "glyph outline indices",
                bytemuck::cast_slice(&geometry.indices),
                wgpu::BufferUsages::INDEX,
            )?,
        })
    })?;
    Ok(Some(Mesh {
        buffer: Arc::new(OnceLock::from(buffers)),
        indices: 0..count,
        bounds,
        brush: Brush::Solid([1.0; 4]),
        composite: CanvasComposite::SourceOver,
    }))
}
