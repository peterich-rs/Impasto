//! Layer traversal, visibility, and effect selection.

use std::sync::Arc;

use engine::scene::{Layer, LayerId, LayerKind, PictureId};

use super::{
    Frame, Image, PictureReplay, Scene, Status, add_corner_clip, add_rect_clip, intersect,
    merge_clip, translation,
};
use crate::batch::{self, RectInstance};
use crate::compiler::{self, Affine, Clip, Draw, Effect, Group, IDENTITY, PictureBounds};
use crate::pipelines::BlendKey;

impl Frame<'_> {
    pub(super) fn layer_bounds(
        &self,
        mut current: Option<LayerId>,
        transform: Affine,
    ) -> Result<PictureBounds, Status> {
        let mut bounds = PictureBounds::Empty;
        while let Some(id) = current {
            let layer = self.scene.layer(id).ok_or(Status::BadScene)?;
            current = layer.next;
            if self.scene.slice.is_some_and(|slice| !slice.contains(&id)) {
                continue;
            }
            bounds = bounds.union(self.layer_bound(layer, transform)?);
        }
        Ok(bounds)
    }

    fn layer_bound(&self, layer: &Layer, transform: Affine) -> Result<PictureBounds, Status> {
        let children = layer.first_child;
        Ok(match layer.kind {
            LayerKind::PlatformView { .. } => PictureBounds::Empty,
            LayerKind::Fill { bounds, .. } => PictureBounds::rect(bounds).transform(transform),
            LayerKind::Paint(slot) => PictureBounds::rect(slot.bounds)
                .expand(slot.stroke_width * 0.5 + self.scene.scale.recip())
                .transform(transform),
            LayerKind::Picture { picture } => {
                let picture = self.scene.pictures.get(&picture).ok_or(Status::BadScene)?;
                let origin =
                    compiler::compose(transform, translation(picture.origin.0, picture.origin.1));
                picture.bounds_in(origin, self.scene.scale).0
            }
            LayerKind::Offset { .. } | LayerKind::Transform { .. } => self.layer_bounds(
                children,
                compiler::compose(transform, compiler::layer_transform(&layer.kind)),
            )?,
            LayerKind::CornerClip {
                bounds: [x, y, w, h],
                ..
            } => {
                let child = self.layer_bounds(children, transform)?;
                if let PictureBounds::Rect(clip) =
                    PictureBounds::rect([x, y, w, h]).transform(transform)
                {
                    child.clip(clip)
                } else {
                    child
                }
            }
            LayerKind::Clip { x, y, w, h, .. } => {
                let child = self.layer_bounds(children, transform)?;
                if let PictureBounds::Rect(clip) =
                    PictureBounds::rect([x, y, w, h]).transform(transform)
                {
                    child.clip(clip)
                } else {
                    child
                }
            }
            LayerKind::Blur { sigma, .. } => self
                .layer_bounds(children, transform)?
                .expand(3.0 * sigma.max(0.0) * compiler::effect_scale(transform)),
            LayerKind::BackdropBlur { x, y, w, h, .. } => self
                .layer_bounds(children, transform)?
                .union(PictureBounds::rect([x, y, w, h]).transform(transform)),
            LayerKind::BackdropMaterial { bounds, .. } => self
                .layer_bounds(children, transform)?
                .union(PictureBounds::rect(bounds).transform(transform)),
            _ => self.layer_bounds(children, transform)?,
        })
    }

    fn source_over_layers(&self, mut current: Option<LayerId>) -> bool {
        while let Some(id) = current {
            let Some(layer) = self.scene.layer(id) else {
                return false;
            };
            current = layer.next;
            match layer.kind {
                LayerKind::Picture { picture } => {
                    if !self.scene.pictures.get(&picture).is_some_and(|picture| {
                        picture.groups.is_empty()
                            && picture
                                .scopes
                                .iter()
                                .flat_map(|scope| &scope.draws)
                                .all(Draw::source_over)
                    }) {
                        return false;
                    }
                }
                LayerKind::Clip { .. }
                | LayerKind::CornerClip { .. }
                | LayerKind::Offset { .. }
                | LayerKind::Transform { .. } => {
                    if !self.source_over_layers(layer.first_child) {
                        return false;
                    }
                }
                LayerKind::PlatformView { .. } | LayerKind::Fill { .. } | LayerKind::Paint(_) => {}
                _ => return false,
            }
        }
        true
    }

    pub(super) fn layers(
        &mut self,
        mut current: Option<LayerId>,
        target: &Image,
        transform: Affine,
        clip: &Clip,
    ) -> Result<(), Status> {
        let hidden = self.occluded_siblings(current, transform, clip, target)?;
        while let Some(id) = current {
            let layer = self.scene.layer(id).ok_or(Status::BadScene)?;
            current = layer.next;
            if hidden.contains(&id) || self.scene.slice.is_some_and(|slice| !slice.contains(&id)) {
                continue;
            }
            self.replay_layer(id, layer, target, transform, clip)?;
        }
        Ok(())
    }

    fn replay_layer(
        &mut self,
        id: LayerId,
        layer: &Layer,
        target: &Image,
        transform: Affine,
        clip: &Clip,
    ) -> Result<(), Status> {
        let children = layer.first_child;
        match layer.kind {
            LayerKind::PlatformView { .. } => Ok(()),
            LayerKind::Paint(slot) => {
                self.layer_rect(target, transform, clip, compiler::paint_slot_rect(slot))
            }
            LayerKind::Fill {
                bounds,
                radius,
                corners,
                color,
            } => {
                let rect =
                    (color[3] > 0.0 && bounds[2] > 0.0 && bounds[3] > 0.0).then(|| RectInstance {
                        radius,
                        corners,
                        ..compiler::solid_rect(bounds, color)
                    });
                self.layer_rect(target, transform, clip, rect)
            }
            LayerKind::Picture { picture } => {
                self.picture_layer(target, transform, clip, id, picture)
            }
            LayerKind::Offset { .. } | LayerKind::Transform { .. } => {
                let placed = compiler::compose(transform, compiler::layer_transform(&layer.kind));
                self.layers(children, target, placed, clip)
            }
            LayerKind::CornerClip { bounds, corners } => {
                let mut next = clip.clone();
                add_corner_clip(&mut next, bounds, 0.0, Some(corners), transform);
                self.layers(children, target, transform, &next)
            }
            LayerKind::Clip {
                x,
                y,
                w,
                h,
                corner_radius,
            } => {
                let mut next = clip.clone();
                add_rect_clip(&mut next, [x, y, w, h], corner_radius, transform);
                self.layers(children, target, transform, &next)
            }
            LayerKind::Opacity { alpha, .. } => {
                self.opacity_layer(target, transform, clip, children, alpha)
            }
            LayerKind::Blur { sigma, .. } => {
                self.blur_layer(target, transform, clip, children, sigma)
            }
            LayerKind::ColorFilter { ref matrix } => self.bounded_layer_group(
                target,
                transform,
                clip,
                Effect::ColorFilter(**matrix),
                children,
            ),
            LayerKind::BackdropBlur { sigma, .. } if sigma <= 0.0 => {
                self.layers(children, target, transform, clip)
            }
            LayerKind::BackdropBlur { x, y, w, h, sigma } => {
                let group = Group {
                    effect: Effect::BackdropBlur,
                    opacity: 1.0,
                    sigma,
                    bounds: Some([x, y, w, h]),
                    content: PictureBounds::Empty,
                    transform,
                };
                self.layer_group(target, transform, clip, group, children)
            }
            LayerKind::BackdropMaterial {
                bounds,
                corners,
                ref material,
            } => {
                let group = Group {
                    effect: Effect::BackdropMaterial(**material, corners),
                    opacity: 1.0,
                    sigma: material.sigma,
                    bounds: Some(bounds),
                    content: PictureBounds::Empty,
                    transform,
                };
                self.layer_group(target, transform, clip, group, children)
            }
            LayerKind::Blend { mode, .. } => {
                self.bounded_layer_group(target, transform, clip, Effect::Blend(mode), children)
            }
        }
    }

    fn layer_rect(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        rect: Option<RectInstance>,
    ) -> Result<(), Status> {
        if let Some(rect) = rect {
            let coverage = self.coverage(target, clip)?;
            self.rects(target, transform, &coverage, &[rect], &[])?;
        }
        Ok(())
    }

    fn bounded_layer_group(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        effect: Effect,
        children: Option<LayerId>,
    ) -> Result<(), Status> {
        let group = Group {
            effect,
            opacity: 1.0,
            sigma: 0.0,
            bounds: None,
            content: self.layer_bounds(children, transform)?,
            transform,
        };
        self.layer_group(target, transform, clip, group, children)
    }

    fn picture_layer(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        id: LayerId,
        picture_id: PictureId,
    ) -> Result<(), Status> {
        let picture = Arc::clone(
            self.scene
                .pictures
                .get(&picture_id)
                .ok_or(Status::BadScene)?,
        );
        let local = compiler::compose(transform, translation(picture.origin.0, picture.origin.1));
        let replay = PictureReplay {
            layer: id,
            picture: &picture,
            origin: local,
            group_bounds: (!picture.groups.is_empty())
                .then(|| picture.bounds_in(local, self.scene.scale).1)
                .flatten(),
        };
        self.replay_picture(&replay, target, clip)
    }

    fn opacity_layer(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        children: Option<LayerId>,
        alpha: u8,
    ) -> Result<(), Status> {
        if alpha == 0 {
            return Ok(());
        }
        if alpha == 255 && self.source_over_layers(children) {
            return self.layers(children, target, transform, clip);
        }
        let opacity = f32::from(alpha) / 255.0;
        let group = Group {
            effect: Effect::Opacity,
            opacity,
            sigma: 0.0,
            bounds: None,
            content: self.layer_bounds(children, transform)?,
            transform,
        };
        if let Some(id) = children
            && self.scene.slice.is_none_or(|slice| slice.contains(&id))
            && let Some(child) = self.scene.layer(id)
            && child.next.is_none()
            && let LayerKind::Picture { picture } = child.kind
        {
            let picture = Arc::clone(self.scene.pictures.get(&picture).ok_or(Status::BadScene)?);
            let origin =
                compiler::compose(transform, translation(picture.origin.0, picture.origin.1));
            if self.fold_opacity(group, &picture.scopes, 0, target, origin, clip)? {
                return Ok(());
            }
        }
        self.layer_group(target, transform, clip, group, children)
    }

    fn blur_layer(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        children: Option<LayerId>,
        sigma: f32,
    ) -> Result<(), Status> {
        if sigma <= 0.0 {
            return self.layers(children, target, transform, clip);
        }
        let group = Group {
            effect: Effect::Blur,
            opacity: 1.0,
            sigma,
            bounds: None,
            content: self.layer_bounds(children, transform)?,
            transform,
        };
        self.layer_group(target, transform, clip, group, children)
    }

    fn layer_group(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        group: Group,
        children: Option<LayerId>,
    ) -> Result<(), Status> {
        self.group(target, transform, clip, group, |frame, target| {
            frame.layers(children, target, transform, clip)
        })
    }

    fn occluded_siblings(
        &mut self,
        first: Option<LayerId>,
        transform: Affine,
        clip: &Clip,
        target: &Image,
    ) -> Result<Vec<LayerId>, Status> {
        const MAX_SIBLINGS: usize = 32;
        let mut siblings = [LayerId(0); MAX_SIBLINGS];
        let mut count = 0;
        let mut current = first;
        if self.scene.slice.is_some() || !clip.shapes.is_empty() {
            return Ok(Vec::new());
        }
        while let Some(id) = current {
            let Some(slot) = siblings.get_mut(count) else {
                return Ok(Vec::new());
            };
            *slot = id;
            count += 1;
            current = self.scene.layer(id).and_then(|layer| layer.next);
        }
        let Some((&tail, preceding)) = siblings[..count].split_last() else {
            return Ok(Vec::new());
        };
        if preceding.is_empty() {
            return Ok(Vec::new());
        }
        let scale = self.scene.scale;
        let target = target.area().map(|value| value as f32 / scale);
        let Some(cover) = self.opaque_layer_rect(tail, transform, target) else {
            return Ok(Vec::new());
        };
        // A visible backdrop or shader can sample pixels beyond its own bounds.
        if scene_reads_root(&self.scene, first) {
            return Ok(Vec::new());
        }
        let mut hidden = Vec::new();
        for &id in preceding {
            let layer = self.scene.layer(id).ok_or(Status::BadScene)?;
            if matches!(layer.kind, LayerKind::PlatformView { .. }) {
                continue;
            }
            let bounds = self.layer_bound(layer, transform)?.expand(1.0 / scale);
            if let PictureBounds::Rect(bounds) = bounds
                && intersect(cover, bounds) == bounds
            {
                hidden.push(id);
            }
        }
        self.compositor.render_stats.occluded_siblings += u32::try_from(hidden.len()).unwrap_or(0);
        Ok(hidden)
    }

    fn opaque_layer_rect(
        &self,
        mut id: LayerId,
        mut transform: Affine,
        target: [f32; 4],
    ) -> Option<[f32; 4]> {
        let scale = self.scene.scale;
        let mut limit = target;
        loop {
            let layer = self.scene.layer(id)?;
            match layer.kind {
                LayerKind::Offset { .. } | LayerKind::Transform { .. } => {
                    transform =
                        compiler::compose(transform, compiler::layer_transform(&layer.kind));
                }
                LayerKind::Clip {
                    x,
                    y,
                    w,
                    h,
                    corner_radius: 0.0,
                } if transform[1] == 0.0 && transform[2] == 0.0 => {
                    let PictureBounds::Rect(bounds) =
                        PictureBounds::rect([x, y, w, h]).transform(transform)
                    else {
                        return None;
                    };
                    limit = intersect(limit, bounds);
                }
                LayerKind::Picture { picture } => {
                    let picture = self.scene.pictures.get(&picture)?;
                    if picture.reads_root
                        || picture.has_shader_effect
                        || !picture.groups.is_empty()
                        || !picture
                            .scopes
                            .iter()
                            .flat_map(|scope| &scope.draws)
                            .all(Draw::source_over)
                    {
                        return None;
                    }
                    let origin = compiler::compose(
                        transform,
                        translation(picture.origin.0, picture.origin.1),
                    );
                    return picture.scopes.iter().rev().find_map(|scope| {
                        if !scope.state.clip.shapes.is_empty() {
                            return None;
                        }
                        let transform = compiler::compose(origin, scope.state.transform);
                        let scope_clip = merge_clip(&Clip::default(), &scope.state.clip, origin);
                        if !scope_clip.shapes.is_empty() {
                            return None;
                        }
                        let limit = scope_clip
                            .scissor
                            .map_or(limit, |clip| intersect(limit, clip));
                        scope.draws.iter().rev().find_map(|draw| match draw {
                            Draw::Fill(color) if color[3] == 1.0 => Some(limit),
                            Draw::Rects(run)
                                if transform[1] == 0.0
                                    && transform[2] == 0.0
                                    && transform[0].abs() == transform[3].abs() =>
                            {
                                run.rects.iter().rev().find_map(|rect| {
                                    if rect.kind != batch::RECT_FILL
                                        || rect.color[3] != 1.0
                                        || rect.gradient_b[3] != 1.0
                                        || rect.flags != 0
                                        || rect.corners.is_some()
                                    {
                                        return None;
                                    }
                                    let [x, y, w, h] = rect.rect;
                                    let radius = rect.radius.max(0.0).min(w.min(h) * 0.5);
                                    let inset = radius * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
                                    let interior = PictureBounds::rect([
                                        x + inset,
                                        y + inset,
                                        w - 2.0 * inset,
                                        h - 2.0 * inset,
                                    ])
                                    .transform(transform)
                                    .expand(-1.0 / scale);
                                    let PictureBounds::Rect(interior) = interior else {
                                        return None;
                                    };
                                    Some(intersect(limit, interior))
                                })
                            }
                            _ => None,
                        })
                    });
                }
                _ => return None,
            }
            id = layer.first_child?;
            if self.scene.layer(id)?.next.is_some() {
                return None;
            }
        }
    }
}

pub(super) fn scene_reads_root(scene: &Scene<'_>, mut current: Option<LayerId>) -> bool {
    use engine::scene::BlendMode;
    while let Some(id) = current {
        let Some(layer) = scene.layer(id) else {
            return false;
        };
        current = layer.next;
        if scene.slice.is_some_and(|slice| !slice.contains(&id)) {
            continue;
        }
        match layer.kind {
            LayerKind::BackdropBlur { .. }
            | LayerKind::BackdropMaterial { .. }
            | LayerKind::Blend {
                mode: BlendMode::Multiply,
                ..
            } => return true,
            LayerKind::Picture { picture } => {
                if scene
                    .pictures
                    .get(&picture)
                    .is_some_and(|picture| picture.reads_root)
                {
                    return true;
                }
            }
            LayerKind::Clip { .. }
            | LayerKind::CornerClip { .. }
            | LayerKind::Offset { .. }
            | LayerKind::Transform { .. }
                if scene_reads_root(scene, layer.first_child) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

pub(super) fn opaque_tail(scene: &Scene<'_>, size: [u32; 2]) -> (LayerId, u32) {
    let root = scene.tree.root;
    if scene.slice.is_some() {
        return (root, 0);
    }
    let mut tail = root;
    let mut hidden = 0u32;
    while let Some(next) = scene.layer(tail).and_then(|layer| layer.next) {
        tail = next;
        hidden += 1;
    }
    if hidden == 0 {
        return (root, 0);
    }
    let covered = scene.layer(tail).is_some_and(|layer| {
        let LayerKind::Picture { picture } = layer.kind else {
            return false;
        };
        let Some(picture) = scene.pictures.get(&picture) else {
            return false;
        };
        if !picture.groups.is_empty() || picture.reads_root || picture.has_shader_effect {
            return false;
        }
        let [width, height] = size.map(|value| value as f32 / scene.scale);
        picture.scopes.iter().any(|scope| {
            let state = &scope.state;
            if state.clip.scissor.is_some()
                || state.clip.hard_scissor.is_some()
                || !state.clip.shapes.is_empty()
                || state.transform != IDENTITY
            {
                return false;
            }
            scope.draws.iter().any(|draw| match draw {
                Draw::Fill(color) => color[3] == 1.0,
                Draw::Rects(run) if run.blend == BlendKey::SrcOver => {
                    run.rects.iter().any(|rect| {
                        let [x, y, w, h] = rect.rect;
                        let x = x + picture.origin.0;
                        let y = y + picture.origin.1;
                        rect.kind == batch::RECT_FILL
                            && rect.color[3] == 1.0
                            && rect.gradient_b[3] == 1.0
                            && rect.flags == 0
                            && rect.radius == 0.0
                            && rect.corners.is_none()
                            && x <= 0.0
                            && y <= 0.0
                            && x + w >= width
                            && y + h >= height
                    })
                }
                _ => false,
            })
        }) && picture
            .scopes
            .iter()
            .flat_map(|scope| &scope.draws)
            .all(Draw::source_over)
    });
    if covered { (tail, hidden) } else { (root, 0) }
}
