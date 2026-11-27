//! Records node effects and placement in the layer tree and flat command stream.

use std::sync::Arc;

use super::retained::{clip_kind, rect_slot};
use super::{
    FlatList, NodeId, PaintClip, PaintKind, RunScope, Tree, intersect, paint_kind, positive_sigma,
};
use crate::scene::{
    BackdropMaterial, BlendMode, ColorMatrix, Command, CompositorTransform, CornerShape,
    MAX_SHADER_UNIFORM_BYTES, ScrollTargetProp, ScrollValue,
};

struct LayerPaint {
    abs: (f32, f32),
    at: (f32, f32),
    size: [f32; 2],
    effect: PaintKind,
    radius: f32,
    corners: Option<CornerShape>,
    alpha: Option<u8>,
    scroll: (f32, f32),
    transform: (f32, f32, f32, f32, f32),
    blur: Option<f32>,
    backdrop: Option<f32>,
    fill: Option<[f32; 4]>,
    uniforms: [Option<f32>; MAX_SHADER_UNIFORM_BYTES / size_of::<f32>()],
    backdrop_material: Option<Arc<BackdropMaterial>>,
    color_filter: Option<Arc<ColorMatrix>>,
    blend: Option<BlendMode>,
    caches: bool,
    pushes_opacity: bool,
    pushes_matrix: bool,
    track_base: CompositorTransform,
}

impl LayerPaint {
    fn push_flat_effects(&self, out: &mut FlatList<'_>) -> usize {
        let [w, h] = self.size;
        let mut pushed = 0;
        if self.pushes_matrix {
            pushed += 1;
            out.push(Command::PushTransform {
                translate_x: self.transform.0,
                translate_y: self.transform.1,
                rotate_rad: self.transform.2,
                scale_x: self.transform.3,
                scale_y: self.transform.4,
            });
        }
        if self.pushes_opacity {
            pushed += 1;
            out.push(Command::PushOpacity {
                x: self.at.0,
                y: self.at.1,
                w,
                h,
                alpha: self.alpha.unwrap_or(255),
            });
        }
        if let Some(mode) = self.blend {
            pushed += 1;
            out.push(Command::PushBlend {
                x: self.at.0,
                y: self.at.1,
                w,
                h,
                mode,
            });
        }
        if let Some(sigma) = self.backdrop {
            pushed += 1;
            out.push(Command::PushBackdropBlur {
                x: self.at.0,
                y: self.at.1,
                w,
                h,
                sigma,
            });
        }
        if let Some(matrix) = &self.color_filter {
            pushed += 1;
            out.push(Command::PushColorFilter {
                matrix: Arc::clone(matrix),
            });
        }
        if let Some(sigma) = self.blur {
            pushed += 1;
            out.push(Command::PushBlur {
                x: self.at.0,
                y: self.at.1,
                w,
                h,
                sigma,
            });
        }
        pushed
    }
}

impl Tree {
    pub(super) fn emit_node(
        &mut self,
        id: NodeId,
        out: &mut FlatList<'_>,
        parent_abs: (f32, f32),
        ancestor_scroll: (f32, f32),
        clip: Option<PaintClip>,
    ) {
        self.layer_record_visits = self.layer_record_visits.saturating_add(1);
        let Some(node) = self.get(id) else {
            return;
        };
        let transition_opacity = self
            .motion_layers
            .get(&id)
            .is_some_and(|layers| layers.transition_opacity);
        if node
            .flags
            .contains(crate::tree::node::NodeFlags::CULL_PAINT)
            || node.flags.contains(crate::tree::node::NodeFlags::HIDDEN)
            || node.alpha() == Some(0) && !transition_opacity
        {
            return;
        }
        let paint = paint_kind(node);
        let alpha = node.alpha();
        let abs = (parent_abs.0 + node.x, parent_abs.1 + node.y);
        let child = node.first_child;
        if paint.opacity && alpha == Some(0) && !transition_opacity {
            self.capture(id, parent_abs, out, clip);
            return;
        }
        if paint.clip && !self.renders_layer_node(id) {
            self.emit_settled_clip(id, out, parent_abs, ancestor_scroll, clip, child);
            return;
        }
        if self.renders_layer_node(id) {
            self.emit_layer(id, out, parent_abs, ancestor_scroll, clip);
            return;
        }
        if self.platform_views.is_empty() && self.is_picture_owner(id) && self.subtree_flat(id) {
            self.emit_picture_owner(id, out, parent_abs, ancestor_scroll, clip);
            return;
        }
        self.capture(id, parent_abs, out, clip);
        self.emit_platform_view(id);
        self.emit_children(id, out, abs, ancestor_scroll, clip);
    }

    fn emit_children(
        &mut self,
        id: NodeId,
        out: &mut FlatList<'_>,
        abs: (f32, f32),
        scroll: (f32, f32),
        clip: Option<PaintClip>,
    ) {
        if let Some(children) = self.ordered_children(id, false) {
            for child in children {
                self.emit_node(child, out, abs, scroll, clip);
            }
        } else {
            let mut child = self.get(id).and_then(|node| node.first_child);
            while let Some(current) = child {
                child = self.get(current).and_then(|node| node.next);
                self.emit_node(current, out, abs, scroll, clip);
            }
        }
    }

    fn emit_settled_clip(
        &mut self,
        id: NodeId,
        out: &mut FlatList<'_>,
        parent_abs: (f32, f32),
        ancestor_scroll: (f32, f32),
        clip: Option<PaintClip>,
        _child: Option<NodeId>,
    ) {
        let Some(node) = self.get(id) else {
            return;
        };
        let (w, h) = (node.w, node.h);
        let radius = node.corner_radius();
        let corners = node.corners();
        let abs = (parent_abs.0 + node.x, parent_abs.1 + node.y);
        self.capture(id, parent_abs, out, clip);
        self.emit_platform_view(id);
        let paint = (abs.0 - ancestor_scroll.0, abs.1 - ancestor_scroll.1);
        let local = self.local_xy(abs);
        self.push_run_bracket(
            Some(id),
            Command::clip([local.0, local.1, w, h], radius, corners),
            out,
        );
        let child_clip = intersect(clip, paint.0, paint.1, w, h);
        self.emit_children(id, out, abs, ancestor_scroll, child_clip);
        self.push_run_bracket(None, Command::Pop, out);
    }

    fn emit_layer(
        &mut self,
        id: NodeId,
        out: &mut FlatList<'_>,
        parent_abs: (f32, f32),
        ancestor_scroll: (f32, f32),
        clip: Option<PaintClip>,
    ) {
        let Some(paint) = self.layer_paint(id, parent_abs, ancestor_scroll) else {
            return;
        };
        let LayerPaint {
            abs,
            at,
            size: [w, h],
            effect,
            radius,
            corners,
            alpha,
            scroll,
            transform,
            blur,
            backdrop,
            fill,
            uniforms,
            ref backdrop_material,
            ref color_filter,
            blend,
            caches,
            track_base,
            ..
        } = paint;
        self.close_run();
        let Some(saved) = self.run.scope else {
            return;
        };
        self.run.scope = Some(RunScope {
            owner: id,
            base: abs,
            mirror: at,
            segment: if saved.owner == id { saved.segment } else { 0 },
        });
        let mut pushed = self.push_effect_layers(
            id,
            effect,
            abs,
            w,
            h,
            alpha,
            transform,
            blur,
            backdrop,
            blend,
            color_filter.as_ref(),
            track_base,
        );
        let mut flat_pushed = paint.push_flat_effects(out);
        if let Some(material) = backdrop_material {
            let corners = corners.or_else(|| {
                (radius > 0.0).then_some(CornerShape {
                    radii: [radius; 4],
                    curve: 2.0,
                })
            });
            self.push_layer(
                id,
                crate::scene::LayerKind::BackdropMaterial {
                    bounds: [abs.0, abs.1, w, h],
                    corners,
                    material: Arc::clone(material),
                },
            );
            pushed += 1;
            flat_pushed += 1;
            out.push(Command::PushBackdropMaterial {
                bounds: [at.0, at.1, w, h],
                corners,
                material: Arc::clone(material),
            });
        }
        if let Some(color) = fill {
            let layer = self.note_layer(
                id,
                crate::scene::LayerKind::Fill {
                    bounds: [abs.0, abs.1, w, h],
                    radius,
                    corners,
                    color,
                },
            );
            if let Some(links) = self.motion_layers.get_mut(&id) {
                links.fill = Some(layer);
            }
            out.push(
                crate::scene::RectPaint {
                    color: crate::scene::linear_color_bytes(color),
                    stroke_width: 0.0,
                    stroke_color: [0; 4],
                    gradient: None,
                    shadow: None,
                    box_shadows: Arc::from([]),
                }
                .command([at.0, at.1, w, h], radius, corners),
            );
        }
        if self.uses_paint_slot(id)
            && let Some(node) = self.get(id)
        {
            let slot = rect_slot(node, [abs.0, abs.1, w, h]);
            let layer = self.note_layer(id, crate::scene::LayerKind::Paint(slot));
            self.retained_layers.paint.insert(id, layer);
            if slot.bounds[2] > 0.0
                && slot.bounds[3] > 0.0
                && (slot.color[3] > 0 || slot.stroke_width > 0.0)
            {
                out.push(
                    crate::scene::RectPaint {
                        color: slot.color,
                        stroke_width: slot.stroke_width,
                        stroke_color: slot.stroke_color,
                        gradient: None,
                        shadow: None,
                        box_shadows: Arc::from([]),
                    }
                    .command([at.0, at.1, w, h], radius, corners),
                );
            }
        }
        self.capture(id, parent_abs, out, clip);
        if uniforms.iter().any(Option::is_some) {
            self.close_run();
            if let Some(layer) = self.scroll_shader_layer(id) {
                self.layers.shader_uniforms.insert(layer, uniforms);
            }
        }
        self.emit_platform_view(id);
        if effect.clip || effect.scroll {
            flat_pushed += 1;
            out.push(if effect.scroll {
                Command::clip([at.0, at.1, w, h], 0.0, None)
            } else {
                Command::clip([at.0, at.1, w, h], radius, corners)
            });
        }
        self.close_run();
        let next_scroll = if effect.scroll {
            (ancestor_scroll.0 + scroll.0, ancestor_scroll.1 + scroll.1)
        } else {
            ancestor_scroll
        };
        let child_clip = if effect.scroll || effect.clip {
            intersect(clip, at.0, at.1, w, h)
        } else if effect.transform {
            None
        } else {
            clip
        };
        if let Some(scope) = self.run.scope.as_mut() {
            scope.mirror = (abs.0 - next_scroll.0, abs.1 - next_scroll.1);
        }
        pushed +=
            self.push_content_layers(id, effect, [abs.0, abs.1, w, h], radius, corners, scroll);
        let hosts_rows = self
            .subtree_layers
            .get(&id)
            .is_some_and(|value| value.picture_owners);
        if self.platform_views.is_empty()
            && caches
            && !hosts_rows
            && (self.picture_fresh(id) || self.subtree_flat(id))
        {
            if !self.picture_fresh(id) {
                self.fill_children(id, abs);
            } else if let Some(picture) = self.pictures.get(&id).cloned() {
                self.adopt_picture(picture);
            }
            let dx = abs.0 - next_scroll.0;
            let dy = abs.1 - next_scroll.1;
            self.blit(id, out, dx, dy, child_clip);
        } else {
            self.pictures.remove(&id);
            self.emit_children(id, out, abs, next_scroll, child_clip);
        }
        self.close_run();
        if saved.owner != id {
            self.leave_owner(id);
            self.run.scope = Some(saved);
        } else if let Some(scope) = self.run.scope.as_mut() {
            scope.base = saved.base;
            scope.mirror = saved.mirror;
        }
        self.run.commands.clear();
        self.run.first = None;
        self.pop_layers(pushed);
        for _ in 0..flat_pushed {
            out.push(Command::Pop);
        }
    }

    fn layer_paint(
        &self,
        id: NodeId,
        parent_abs: (f32, f32),
        ancestor_scroll: (f32, f32),
    ) -> Option<LayerPaint> {
        let node = self.get(id)?;
        let abs = (parent_abs.0 + node.x, parent_abs.1 + node.y);
        let at = (abs.0 - ancestor_scroll.0, abs.1 - ancestor_scroll.1);
        let (w, h) = (node.w, node.h);
        let mut effect = paint_kind(node);
        let radius = node.corner_radius();
        let corners = node.corners();
        let mut alpha = node.alpha();
        let scroll = node.scroll_paint();
        let base = self
            .effective_transform(id)
            .unwrap_or_else(|| node.transform());
        effect.transform |= !base.is_identity();
        let geom = base.placed(abs.0, abs.1, w, h);
        let transform = (
            geom.translate_x,
            geom.translate_y,
            geom.rotate_rad,
            geom.scale_x,
            geom.scale_y,
        );
        let mut blur = positive_sigma(node.blur());
        let mut backdrop = positive_sigma(node.backdrop());
        let mut fill = None;
        let mut uniforms = [None; MAX_SHADER_UNIFORM_BYTES / size_of::<f32>()];
        for (prop, value) in self.binding_values(id) {
            match (prop, value) {
                (ScrollTargetProp::Opacity, ScrollValue::Alpha(value)) => {
                    effect.opacity = true;
                    alpha = Some(value);
                }
                (ScrollTargetProp::BlurSigma, ScrollValue::Scalar(value)) => blur = Some(value),
                (ScrollTargetProp::BackdropBlurSigma, ScrollValue::Scalar(value)) => {
                    backdrop = Some(value)
                }
                (ScrollTargetProp::BackgroundColor, ScrollValue::Color(value)) => {
                    fill = Some(value)
                }
                (ScrollTargetProp::ShaderUniform(index), ScrollValue::Scalar(value)) => {
                    if let Some(slot) = uniforms.get_mut(usize::from(index)) {
                        *slot = Some(value);
                    }
                }
                _ => {}
            }
        }
        let backdrop_material = node.backdrop_material().cloned();
        let color_filter = node.color_filter().cloned();
        let blend = node.blend();
        let blend = (blend != BlendMode::SrcOver).then_some(blend);
        let caches = node.caches_picture();
        let track_base = base.compositor_transform(abs.0, abs.1, w, h);
        let pushes_matrix = effect.transform && !geom.is_identity();
        let pushes_opacity = effect.opacity && alpha.unwrap_or(255) != 255;
        Some(LayerPaint {
            abs,
            at,
            size: [w, h],
            effect,
            radius,
            corners,
            alpha,
            scroll,
            transform,
            blur,
            backdrop,
            fill,
            uniforms,
            backdrop_material,
            color_filter,
            blend,
            caches,
            pushes_opacity,
            pushes_matrix,
            track_base,
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is one effect the layer stack may push for this node"
    )]
    fn push_effect_layers(
        &mut self,
        node: NodeId,
        effect: PaintKind,
        abs: (f32, f32),
        w: f32,
        h: f32,
        alpha: Option<u8>,
        transform: (f32, f32, f32, f32, f32),
        blur: Option<f32>,
        backdrop: Option<f32>,
        blend: Option<BlendMode>,
        color_filter: Option<&Arc<ColorMatrix>>,
        track_base: CompositorTransform,
    ) -> usize {
        let mut pushed = 0;
        let identity = transform.0 == 0.0
            && transform.1 == 0.0
            && transform.2 == 0.0
            && transform.3 == 1.0
            && transform.4 == 1.0;
        if !identity && effect.transform
            || self
                .motion_layers
                .get(&node)
                .is_some_and(|layers| layers.linked_transform)
        {
            let layer = self.push_layer(
                node,
                crate::scene::LayerKind::Transform {
                    translate_x: transform.0,
                    translate_y: transform.1,
                    rotate_rad: transform.2,
                    scale_x: transform.3,
                    scale_y: transform.4,
                },
            );
            self.retained_layers
                .transforms
                .insert(node, (layer, [abs.0, abs.1, w, h]));
            if let Some(links) = self.motion_layers.get_mut(&node) {
                links.transform = Some((layer, track_base));
            }
            pushed += 1;
        }
        if effect.opacity && alpha.unwrap_or(255) != 255
            || self
                .motion_layers
                .get(&node)
                .is_some_and(|layers| layers.transition_opacity)
        {
            let layer = self.push_layer(
                node,
                crate::scene::LayerKind::Opacity {
                    alpha: alpha.unwrap_or(255),
                    x: abs.0,
                    y: abs.1,
                    w,
                    h,
                },
            );
            self.retained_layers.opacity.insert(node, layer);
            if let Some(links) = self.motion_layers.get_mut(&node) {
                links.opacity = Some(layer);
            }
            pushed += 1;
        }
        if let Some(mode) = blend {
            self.push_layer(
                node,
                crate::scene::LayerKind::Blend {
                    x: abs.0,
                    y: abs.1,
                    w,
                    h,
                    mode,
                },
            );
            pushed += 1;
        }
        if let Some(sigma) = backdrop {
            let layer = self.push_layer(
                node,
                crate::scene::LayerKind::BackdropBlur {
                    x: abs.0,
                    y: abs.1,
                    w,
                    h,
                    sigma,
                },
            );
            if let Some(links) = self.motion_layers.get_mut(&node) {
                links.backdrop = Some(layer);
            }
            pushed += 1;
        }
        if let Some(matrix) = color_filter {
            self.push_layer(
                node,
                crate::scene::LayerKind::ColorFilter {
                    matrix: Arc::clone(matrix),
                },
            );
            pushed += 1;
        }
        if let Some(sigma) = blur {
            let layer = self.push_layer(
                node,
                crate::scene::LayerKind::Blur {
                    x: abs.0,
                    y: abs.1,
                    w,
                    h,
                    sigma,
                },
            );
            if let Some(links) = self.motion_layers.get_mut(&node) {
                links.blur = Some(layer);
            }
            pushed += 1;
        }
        pushed
    }

    fn push_content_layers(
        &mut self,
        node: NodeId,
        effect: PaintKind,
        bounds: [f32; 4],
        radius: f32,
        corners: Option<CornerShape>,
        scroll: (f32, f32),
    ) -> usize {
        let [x, y, w, h] = bounds;
        let mut pushed = 0;
        if effect.scroll {
            self.push_layer(
                node,
                crate::scene::LayerKind::Clip {
                    x,
                    y,
                    w,
                    h,
                    corner_radius: 0.0,
                },
            );
            let layer = self.push_layer(
                node,
                crate::scene::LayerKind::Offset {
                    x: -scroll.0,
                    y: -scroll.1,
                },
            );
            if let Some(links) = self.motion_layers.get_mut(&node) {
                links.scroll = Some(layer);
            }
            pushed += 2;
        } else if effect.clip {
            let layer = self.push_layer(node, clip_kind(bounds, radius, corners));
            self.retained_layers.clips.insert(node, (layer, bounds));
            pushed += 1;
        }
        pushed
    }
}
