//! Scene preparation and atomic installation on the raster queue.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use engine::scene::{LayerId, LayerKind, LayerTree, Picture, PictureId};

use crate::compiler::{
    self, Affine, CompileContext, CompiledPicture, IDENTITY, compose, layer_transform,
};
use crate::compositor::{GlyphChanges, GlyphScene};
use crate::{
    DamageRect, Frame, ImagePixels, Rasterizer, Status, SubmitInfo, SubmitStats, batch, damage,
    prepare,
};

impl Rasterizer {
    /// Validates and installs a complete scene. On failure the previous frame stays intact.
    pub fn submit_layers<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<SubmitInfo, Status> {
        self.submit_shared_layers(Arc::new(tree.clone()), point_scale, resolve)
    }

    /// Installs a scene already shared by the UI and raster queues.
    pub fn submit_shared_layers<'pixels>(
        &mut self,
        tree: Arc<LayerTree>,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<SubmitInfo, Status> {
        self.submit_scene(tree, point_scale, None, resolve)
    }

    /// Installs a checkpoint with an optional incremental damage description.
    pub fn submit_scene<'pixels>(
        &mut self,
        tree: Arc<LayerTree>,
        point_scale: f32,
        delta: Option<&engine::scene::SceneDelta>,
        mut resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<SubmitInfo, Status> {
        if !point_scale.is_finite() || point_scale <= 0.0 {
            return Err(Status::BadScene);
        }
        let started = Instant::now();
        let _prepare_span =
            tracing::debug_span!(target: "impasto::render", "prepare_pictures").entered();
        let order = paint_order(&tree)?;
        let scales = picture_scales(&tree, &order, point_scale)?;
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        if device.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        let mut textures = device.textures.begin_frame();
        let previous = self.frame.as_ref().filter(|_| self.reuse_pictures);
        let previous_revision = self.frame.as_ref().map_or(0, |frame| frame.revision);
        let mut context = CompileContext {
            gpu: &device.gpu,
            device_generation: device.generation,
            textures: &mut textures,
            resolve: &mut resolve,
            point_scale,
            stream_mesh: false,
        };
        let PreparedPictures {
            pictures,
            mut stats,
            has_glyphs,
            pending_images,
            has_images,
            rebuilt_atlas_pictures,
        } = prepare_pictures(
            &tree,
            &scales,
            previous,
            self.parallel_prepare,
            &mut context,
        )?;
        let mut views = Vec::new();
        #[cfg(target_os = "ios")]
        let mut slices = vec![HashSet::new()];
        for &id in &order {
            let layer = &tree.layers[id.0 as usize];
            match layer.kind {
                LayerKind::PlatformView { node } => {
                    views.push(node);
                    #[cfg(target_os = "ios")]
                    slices.push(HashSet::new());
                }
                LayerKind::Picture { picture } if !pictures.contains_key(&picture) => {
                    return Err(Status::BadScene);
                }
                #[cfg(target_os = "ios")]
                LayerKind::Picture { .. } | LayerKind::Fill { .. } | LayerKind::Paint(_) => {
                    if let Some(slice) = slices.last_mut() {
                        let mut ancestor = Some(id);
                        while let Some(parent) = ancestor {
                            if !slice.insert(parent) {
                                break;
                            }
                            ancestor = tree.layers[parent.0 as usize].parent;
                        }
                    }
                }
                _ => {}
            }
        }
        if device.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        let platform_views: Arc<[u64]> = views.into();
        let gpu_view_requests = self.gpu_views.requests(&tree, &scales, point_scale);
        let complex_damage = !platform_views.is_empty()
            || pictures
                .values()
                .any(|picture| picture.has_shader_effect || picture.reads_root)
            || tree.layers.iter().any(|layer| {
                matches!(
                    layer.kind,
                    LayerKind::BackdropBlur { .. }
                        | LayerKind::BackdropMaterial { .. }
                        | LayerKind::Blend {
                            mode: engine::scene::BlendMode::Multiply,
                            ..
                        }
                )
            });
        let mut frame = Frame {
            shader_uniforms: tree.shader_uniforms.clone(),
            tree,
            order,
            sampled_layers: HashMap::new(),
            scale: point_scale,
            pictures,
            views: Arc::clone(&platform_views),
            gpu_view_requests,
            damage: DamageRect::Full,
            complex_damage,
            pending_images,
            has_images,
            has_glyphs,
            revision: previous_revision.wrapping_add(1),
            previous_revision,
            #[cfg(target_os = "ios")]
            slices,
        };
        let gpu = &device.gpu;
        let (damage, upload) = {
            let damage = match (previous, delta) {
                (Some(old), Some(delta))
                    if old.tree.epoch == delta.base_epoch
                        && old.scale == point_scale
                        && !old.has_images
                        && !frame.has_images =>
                {
                    damage::changed_layers(
                        old,
                        &frame.tree,
                        |id| frame.pictures.get(&id).map(Arc::as_ref),
                        frame.complex_damage,
                        delta,
                    )
                }
                _ => damage::between(previous, &frame, &textures),
            };
            let upload = textures.commit(gpu)?;
            (damage, upload)
        };
        let glyph_changes = match (previous, delta) {
            (Some(old), Some(delta))
                if old.tree.epoch == delta.base_epoch && !delta.inserted_or_changed.is_empty() =>
            {
                Some(GlyphChanges::Layers(&delta.inserted_or_changed))
            }
            (Some(old), _) if old.tree.epoch != frame.tree.epoch => Some(GlyphChanges::All),
            _ if !rebuilt_atlas_pictures.is_empty() => {
                Some(GlyphChanges::Rebuilt(&rebuilt_atlas_pictures))
            }
            _ => None,
        };
        if self.parallel_prepare
            && let Some(changes) = glyph_changes
        {
            stats.glyph_prepare = device.compositor.prepare_glyphs(
                gpu,
                GlyphScene {
                    tree: &frame.tree,
                    pictures: &frame.pictures,
                    fonts: &self.fonts,
                    scale: frame.scale,
                    notify: self.resources_ready.as_ref(),
                    prepare_deadline: self.resource_prepare_deadline,
                },
                changes,
            );
        }
        frame.damage = damage;
        stats.damage = damage;
        stats.image_upload = upload;
        device.compositor.evict_images(device.textures.evictions());
        stats.picture_prepare = started.elapsed();
        stats.layer_count = u32::try_from(frame.tree.layers.len()).unwrap_or(u32::MAX);
        stats.pictures = u32::try_from(frame.pictures.len()).unwrap_or(u32::MAX);
        stats.textures = u32::try_from(device.textures.len()).unwrap_or(u32::MAX);
        self.frame = Some(frame);
        self.reuse_pictures = true;
        #[cfg(target_os = "ios")]
        {
            self.frame_number = self.frame_number.wrapping_add(1);
        }
        self.prepared_buffers = stats.render.buffers_created;
        self.prepared_mesh_bytes = stats.render.mesh_upload_bytes;
        self.stats = stats;
        self.fonts.sweep();
        Ok(SubmitInfo {
            stats,
            platform_views,
        })
    }

    /// Installs layer values and picture replacements over a matching checkpoint.
    ///
    /// Returns `None` when the base is stale or the changes require full picture
    /// preparation. The caller must then submit the full snapshot.
    pub fn submit_layer_delta(
        &mut self,
        tree: &Arc<LayerTree>,
        scale: f32,
        delta: &engine::scene::SceneDelta,
    ) -> Result<Option<SubmitInfo>, Status> {
        use engine::scene::SceneChanges;
        let started = Instant::now();
        let updates = match &delta.changes {
            SceneChanges::LayerValues => &[][..],
            SceneChanges::Pictures(updates) => updates,
            SceneChanges::Topology => return Ok(None),
        };
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        if device.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        let Some(frame) = self.frame.as_mut().filter(|frame| {
            self.reuse_pictures
                && frame.tree.epoch == delta.base_epoch
                && frame.scale == scale
                && frame.tree.layers.len() == tree.layers.len()
                && frame.tree.tracks.is_empty()
                && tree.tracks.is_empty()
                && frame.gpu_view_requests.is_empty()
                && !frame.pending_images
                && delta.removed_or_changed == delta.inserted_or_changed
        }) else {
            return Ok(None);
        };
        if delta.inserted_or_changed.iter().any(|id| {
            let index = id.0 as usize;
            match (frame.tree.layers.get(index), tree.layers.get(index)) {
                (Some(before), Some(after)) => {
                    before.parent != after.parent
                        || before.next != after.next
                        || before.first_child != after.first_child
                        || !patch_preserves_picture_scale(&before.kind, &after.kind)
                }
                _ => true,
            }
        }) {
            return Ok(None);
        }
        let mut stats = SubmitStats::default();
        let mut prepared = HashMap::with_capacity(updates.len());
        let pool = self.parallel_prepare.then(prepare::pool).flatten();
        for update in updates {
            let Some(picture) = tree.pictures.get(update.index) else {
                return Ok(None);
            };
            let Some(before) = frame.pictures.get(&update.id) else {
                return Ok(None);
            };
            if picture.id != update.id || !picture.images().is_empty() || !before.images.is_empty()
            {
                return Ok(None);
            }
            let compiled = compiler::prepare(picture, before.scale, pool, &mut |_| {
                crate::textures::Load::Failed
            })?
            .install(&device.gpu, device.generation, true)?;
            if let Some(mesh) = compiled
                .mesh_buffer
                .get()
                .and_then(batch::MeshBuffer::resident)
            {
                stats.render.buffers_created += 2;
                stats.render.mesh_upload_bytes += mesh.vertices.size() + mesh.indices.size();
            }
            prepared.insert(picture.id, Arc::new(compiled));
        }
        let complex_damage = frame.complex_damage
            || prepared
                .values()
                .any(|picture| picture.has_shader_effect || picture.reads_root);
        let damage = damage::changed_layers(
            frame,
            tree,
            |id| {
                prepared
                    .get(&id)
                    .or_else(|| frame.pictures.get(&id))
                    .map(Arc::as_ref)
            },
            complex_damage,
            delta,
        );
        frame.complex_damage = complex_damage;
        frame.has_glyphs |= prepared.values().any(|picture| picture.has_glyphs);
        frame.pictures.extend(prepared);
        frame.shader_uniforms.clone_from(&tree.shader_uniforms);
        frame.tree = Arc::clone(tree);
        frame.sampled_layers.clear();
        frame.damage = damage;
        frame.previous_revision = frame.revision;
        frame.revision = frame.revision.wrapping_add(1);
        if self.parallel_prepare {
            stats.glyph_prepare = device.compositor.prepare_glyphs(
                &device.gpu,
                GlyphScene {
                    tree: &frame.tree,
                    pictures: &frame.pictures,
                    fonts: &self.fonts,
                    scale: frame.scale,
                    notify: self.resources_ready.as_ref(),
                    prepare_deadline: self.resource_prepare_deadline,
                },
                GlyphChanges::Layers(&delta.inserted_or_changed),
            );
        }
        let pictures = u32::try_from(frame.pictures.len()).unwrap_or(u32::MAX);
        stats.damage = damage;
        stats.pictures = pictures;
        stats.picture_rebuilt = u32::try_from(updates.len()).unwrap_or(u32::MAX);
        stats.picture_reused = pictures.saturating_sub(stats.picture_rebuilt);
        stats.layer_count = u32::try_from(tree.layers.len()).unwrap_or(u32::MAX);
        stats.textures = u32::try_from(device.textures.len()).unwrap_or(u32::MAX);
        stats.picture_prepare = started.elapsed();
        self.stats = stats;
        self.prepared_buffers = stats.render.buffers_created;
        self.prepared_mesh_bytes = stats.render.mesh_upload_bytes;
        #[cfg(target_os = "ios")]
        {
            self.frame_number = self.frame_number.wrapping_add(1);
        }
        Ok(Some(SubmitInfo {
            stats: self.stats,
            platform_views: Arc::clone(&frame.views),
        }))
    }
}

struct PreparedPictures {
    pictures: HashMap<PictureId, Arc<CompiledPicture>>,
    stats: SubmitStats,
    has_glyphs: bool,
    pending_images: bool,
    has_images: bool,
    rebuilt_atlas_pictures: HashSet<PictureId>,
}

fn prepare_pictures(
    tree: &LayerTree,
    scales: &HashMap<PictureId, f32>,
    previous: Option<&Frame>,
    parallel_prepare: bool,
    context: &mut CompileContext<'_, '_, '_>,
) -> Result<PreparedPictures, Status> {
    let point_scale = context.point_scale;
    let compile_scale = |picture| scales.get(&picture).copied().unwrap_or(point_scale);
    let (pool, mut prepared) = if parallel_prepare {
        prepare::pictures(
            tree.pictures.iter().filter_map(|picture| {
                let scale = compile_scale(picture.id);
                let cached = previous
                    .and_then(|frame| frame.pictures.get(&picture.id))
                    .is_some_and(|old| {
                        same_picture(old, picture, scale, context) && old.waiting.is_empty()
                    });
                (!cached).then_some((picture.as_ref(), scale))
            }),
            |source| {
                context
                    .textures
                    .get(source)
                    .map(|image| (image.width(), image.height()))
            },
        )
    } else {
        (None, HashMap::new())
    };
    let mut stats = SubmitStats {
        picture_workers: pool.map_or(0, |pool| {
            u32::try_from(pool.current_num_threads()).unwrap_or(u32::MAX)
        }),
        ..SubmitStats::default()
    };
    let mut pictures = HashMap::with_capacity(tree.pictures.len());
    let mut has_glyphs = false;
    let mut pending_images = false;
    let mut has_images = false;
    let mut rebuilt_atlas_pictures = HashSet::new();
    for picture in &tree.pictures {
        let compile_scale = compile_scale(picture.id);
        let cached = previous
            .and_then(|frame| frame.pictures.get(&picture.id))
            .filter(|cached| {
                same_picture(cached, picture, compile_scale, context)
                    && cached.waiting.iter().all(|source| {
                        context.textures.get(source).is_none()
                            && (context.resolve)(source).is_none()
                    })
            });
        let compiled = if let Some(cached) = cached {
            for (source, _) in &cached.images {
                context.textures.touch(source);
            }
            stats.picture_reused = stats.picture_reused.saturating_add(1);
            Arc::clone(cached)
        } else {
            let _compile_span = tracing::trace_span!(
                target: "impasto::render",
                "compile_picture",
                id = picture.id.0
            )
            .entered();
            let stream_mesh = previous
                .and_then(|frame| frame.pictures.get(&picture.id))
                .is_some_and(|old| old.generation != picture.generation);
            context.point_scale = compile_scale;
            context.stream_mesh = stream_mesh;
            let result = if let Some(prepared) = prepared.remove(&picture.id) {
                let compiled = prepared?.install(
                    context.gpu,
                    context.device_generation,
                    context.stream_mesh,
                )?;
                for (source, _) in picture.images() {
                    context.textures.touch(source);
                }
                compiled
            } else {
                compiler::compile(picture, context, pool)?
            };
            stats.picture_rebuilt = stats.picture_rebuilt.saturating_add(1);
            if parallel_prepare {
                rebuilt_atlas_pictures.insert(picture.id);
            }
            if let Some(mesh) = result
                .mesh_buffer
                .get()
                .and_then(batch::MeshBuffer::resident)
            {
                stats.render.buffers_created += 2;
                stats.render.mesh_upload_bytes += mesh.vertices.size() + mesh.indices.size();
            }
            Arc::new(result)
        };
        has_glyphs |= compiled.has_glyphs;
        pending_images |= !compiled.waiting.is_empty();
        has_images |= !picture.images().is_empty();
        if pictures.insert(picture.id, compiled).is_some() {
            return Err(Status::BadScene);
        }
    }
    Ok(PreparedPictures {
        pictures,
        stats,
        has_glyphs,
        pending_images,
        has_images,
        rebuilt_atlas_pictures,
    })
}

fn same_picture(
    cached: &CompiledPicture,
    picture: &Picture,
    scale: f32,
    context: &CompileContext<'_, '_, '_>,
) -> bool {
    cached.device == context.device_generation
        && cached.generation == picture.generation
        && cached.scale == scale
        && cached.origin == picture.origin
        && cached
            .images
            .iter()
            .all(|(source, _)| context.textures.same_geometry(source))
}

fn patch_preserves_picture_scale(before: &LayerKind, after: &LayerKind) -> bool {
    match (before, after) {
        (LayerKind::Transform { .. }, LayerKind::Transform { .. }) => {
            let [a, b, c, d, ..] = layer_transform(before);
            let [x, y, z, w, ..] = layer_transform(after);
            [a, b, c, d] == [x, y, z, w]
        }
        (LayerKind::Picture { picture: a }, LayerKind::Picture { picture: b }) => a == b,
        (LayerKind::Paint(_), LayerKind::Paint(_))
        | (LayerKind::Opacity { .. }, LayerKind::Opacity { .. })
        | (
            LayerKind::Clip { .. } | LayerKind::CornerClip { .. },
            LayerKind::Clip { .. } | LayerKind::CornerClip { .. },
        )
        | (LayerKind::Offset { .. }, LayerKind::Offset { .. }) => true,
        _ => false,
    }
}

fn picture_scales(
    tree: &LayerTree,
    order: &[LayerId],
    scale: f32,
) -> Result<HashMap<PictureId, f32>, Status> {
    let mut transforms: Vec<Affine> = vec![IDENTITY; tree.layers.len()];
    let mut pictures = HashMap::<PictureId, f32>::new();
    for id in order {
        let index = id.0 as usize;
        let layer = &tree.layers[index];
        let parent = layer
            .parent
            .map_or(IDENTITY, |parent| transforms[parent.0 as usize]);
        let transform = compose(parent, layer_transform(&layer.kind));
        if !transform.iter().all(|value| value.is_finite()) {
            return Err(Status::BadScene);
        }
        transforms[index] = transform;
        if let LayerKind::Picture { picture } = layer.kind {
            let [a, b, c, d, ..] = transform;
            // Half-octave buckets keep scrolling and small transform updates
            // from rebuilding tessellation while bounding pixel error.
            let magnitude = (scale * a.hypot(b).max(c.hypot(d))).clamp(0.25, 128.0);
            let bucket = (magnitude.log2() * 2.0).ceil() * 0.5;
            let scale = bucket.exp2();
            pictures
                .entry(picture)
                .and_modify(|previous| *previous = previous.max(scale))
                .or_insert(scale);
        }
    }
    Ok(pictures)
}

fn paint_order(tree: &LayerTree) -> Result<Vec<LayerId>, Status> {
    if tree.layers.is_empty() {
        return Ok(Vec::new());
    }
    let mut pending = vec![(tree.root, None, 0)];
    let mut seen = vec![false; tree.layers.len()];
    let mut ordered = Vec::with_capacity(tree.layers.len());
    while let Some((id, parent, depth)) = pending.pop() {
        let index = id.0 as usize;
        let layer = tree.layers.get(index).ok_or(Status::BadScene)?;
        if seen[index] || layer.parent != parent || depth > 256 {
            return Err(Status::BadScene);
        }
        seen[index] = true;
        ordered.push(id);
        if let Some(next) = layer.next {
            pending.push((next, parent, depth));
        }
        if let Some(child) = layer.first_child {
            if matches!(
                layer.kind,
                LayerKind::Picture { .. }
                    | LayerKind::PlatformView { .. }
                    | LayerKind::Fill { .. }
                    | LayerKind::Paint(_)
            ) {
                return Err(Status::BadScene);
            }
            pending.push((child, Some(id), depth + 1));
        }
    }
    if ordered.len() != tree.layers.len() {
        return Err(Status::BadScene);
    }
    Ok(ordered)
}
