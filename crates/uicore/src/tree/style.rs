//! Per-node style sources. One flush writes Taffy at most once per node.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::Error;
use crate::style::{
    Interaction, Key, LineHeightValue, Patch, Property, Style, StyleError, StyleOrigin,
    StyleSourceId, StyleSurface, WinningDeclaration, author_key_allowed, flatten, merge_layers,
};

use super::Tree;
use super::node::{Kind, NodeFlags, NodeId, PaintProps, Role, TransformGeom};

#[derive(Clone)]
pub(super) struct NodeStyle {
    pub(super) generation: u32,
    pub(super) preset: taffy::Style,
    component: Style,
    class_text: String,
    class_list: String,
    inline: Style,
    imperative: Style,
    animation: Style,
    extra: Vec<ExtraSource>,
    /// Winning values after inheritance. Children read this, not the raw layers.
    computed: crate::style::Resolved,
    computed_ready: bool,
    /// Text color before any style write. Reset restores it.
    initial: Presentation,
    initial_clip: bool,
    /// Layout longhands last submitted. Paint-only flushes leave Taffy alone.
    /// Class variants and tokens this node must see again when the environment changes.
    deps: crate::style::DepMask,
    /// Explicit surface. Stack and scroll kinds still win unless this is list content.
    surface: StyleSurface,
}

#[derive(Clone)]
struct ExtraSource {
    source: StyleSourceId,
    class_text: String,
    origin: StyleOrigin,
    order: u32,
    style: Style,
}

#[derive(Clone)]
struct TextPresentation {
    size: f32,
    color: [u8; 4],
    style: Arc<protocol::TextStyle>,
}

#[derive(Clone)]
struct Presentation {
    paint: PaintProps,
    transform: Option<TransformGeom>,
    text: Option<TextPresentation>,
    image: Option<protocol::ContentMode>,
}

struct Prepared {
    resolved: crate::style::Resolved,
    layout: taffy::Style,
    presentation: Presentation,
    deps: crate::style::DepMask,
}

impl Presentation {
    fn capture(node: &super::node::Node) -> Self {
        Self {
            paint: node.paint.as_deref().cloned().unwrap_or_default(),
            transform: node.transform.as_deref().copied(),
            text: node.text().map(|text| TextPresentation {
                size: text.font_size,
                color: text.color,
                style: Arc::clone(&text.style),
            }),
            image: node.image().map(|image| image.mode),
        }
    }

    fn transform_mut(&mut self) -> &mut TransformGeom {
        self.transform.get_or_insert_with(|| TransformGeom {
            pivot: Some((0.0, 0.0, 0.5, 0.5)),
            ..TransformGeom::identity()
        })
    }

    fn apply(&mut self, property: &Property, env: &crate::style::Environment) -> Result<(), Error> {
        use Property as P;
        match property {
            P::BackgroundColor(color) => self.paint.fill = Some(env.resolve_color(color)?),
            P::BorderColor(color) => self.paint.stroke_color = env.resolve_color(color)?,
            P::BorderWidth(length) => self.paint.stroke_width = env.resolve_points(length)?,
            P::OutlineColor(color) => self.paint.outline_color = env.resolve_color(color)?,
            P::OutlineWidth(length) => self.paint.outline_width = env.resolve_points(length)?,
            P::BorderRadius(length) => self.paint.corner_radius = env.resolve_points(length)?,
            P::Opacity(value) => self.paint.alpha = (*value < 1.0).then(|| opacity_byte(*value)),
            P::Blur(value) => self.paint.blur = *value,
            P::Backdrop(value) => self.paint.backdrop = *value,
            P::Blend(value) => self.paint.blend = *value,
            P::Gradient(value) => self.paint.gradient = Some(value.clone()),
            P::Shadow(value) => self.paint.shadow = Some(*value),
            P::BoxShadow(value) => self.paint.box_shadows = Arc::from(value.clone()),
            P::TranslateX(length) => {
                self.transform_mut().translate_x = env.resolve_points(length)?
            }
            P::TranslateY(length) => {
                self.transform_mut().translate_y = env.resolve_points(length)?
            }
            P::Rotate(value) => self.transform_mut().rotate_rad = *value,
            P::ScaleX(value) => self.transform_mut().scale_x = *value,
            P::ScaleY(value) => self.transform_mut().scale_y = *value,
            P::TransformOriginX(length) | P::TransformOriginY(length) => {
                let (points, ratio) = origin_parts(length, env)?;
                let geom = self.transform_mut();
                let (ox, oy, rx, ry) = geom.pivot.unwrap_or((0.0, 0.0, 0.5, 0.5));
                geom.pivot = Some(if matches!(property, P::TransformOriginX(_)) {
                    (points, oy, ratio, ry)
                } else {
                    (ox, points, rx, ratio)
                });
            }
            // Resolve inherited values even on containers: invalid tokens must
            // fail before the source is accepted, not later on a text child.
            P::Color(color) => {
                let color = env.resolve_color(color)?;
                if let Some(text) = &mut self.text {
                    text.color = color;
                }
            }
            P::FontSize(length) => {
                let size = env.resolve_font_size(length)?;
                if !size.is_finite() || size < 0.0 {
                    return Err(Error::BadNumber);
                }
                if let Some(text) = &mut self.text {
                    text.size = size;
                }
            }
            P::FontFamily(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).family = Some(value.clone());
                }
            }
            P::FontWeight(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).weight = *value;
                }
            }
            P::LineHeight(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).height = match value {
                        LineHeightValue::Normal => 0.0,
                        LineHeightValue::Factor(value) => *value,
                        LineHeightValue::Points(value) => {
                            if text.size > 0.0 {
                                value / text.size
                            } else {
                                0.0
                            }
                        }
                    };
                }
            }
            P::TextAlign(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).align = *value;
                }
            }
            P::WritingDirectionRtl(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).direction_rtl = *value;
                }
            }
            P::MaxLines(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).max_lines = Some(*value);
                }
            }
            P::Ellipsis(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).ellipsis =
                        (!value.is_empty()).then(|| value.clone());
                }
            }
            P::SoftWrap(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).soft_wrap = *value;
                }
            }
            P::TextOverflow(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).overflow = *value;
                }
            }
            P::Underline(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).underline = *value;
                }
            }
            P::LineThrough(value) => {
                if let Some(text) = &mut self.text {
                    Arc::make_mut(&mut text.style).line_through = *value;
                }
            }
            P::ObjectFit(value) => {
                if let Some(mode) = &mut self.image {
                    *mode = *value;
                }
            }
            // Layout and box flags are committed by the other two projections.
            _ => {}
        }
        Ok(())
    }
}

fn origin_mut(sources: &mut NodeStyle, origin: StyleOrigin) -> Result<&mut Style, Error> {
    match origin {
        StyleOrigin::Component => Ok(&mut sources.component),
        StyleOrigin::Inline => Ok(&mut sources.inline),
        StyleOrigin::Imperative => Ok(&mut sources.imperative),
        StyleOrigin::Animation => Ok(&mut sources.animation),
        StyleOrigin::Classes => Err(Error::BadNumber),
    }
}

/// Declarations collected across mutations and committed as one source write.
impl Tree {
    /// Replace the imperative source. `style` is validated before the node changes.
    pub fn set_style(&mut self, id: NodeId, style: Style) -> Result<(), Error> {
        self.replace_origin(id, StyleOrigin::Imperative, style)
    }

    /// Replace the inline source. Component defaults and classes stay underneath.
    pub fn set_inline_style(&mut self, id: NodeId, style: Style) -> Result<(), Error> {
        self.replace_origin(id, StyleOrigin::Inline, style)
    }

    /// Replace the component-default source. Author class and inline win.
    pub fn set_component_style(&mut self, id: NodeId, style: Style) -> Result<(), Error> {
        self.replace_origin(id, StyleOrigin::Component, style)
    }

    /// Replace the animation source. Completion copies the end value elsewhere.
    pub fn set_animation_style(&mut self, id: NodeId, style: Style) -> Result<(), Error> {
        self.replace_origin(id, StyleOrigin::Animation, style)
    }

    /// An extra source at `origin`. Cleanup removes only this handle.
    pub fn create_style_source(
        &mut self,
        id: NodeId,
        origin: StyleOrigin,
    ) -> Result<StyleSourceId, Error> {
        self.ensure_style(id)?;
        let slot = self
            .next_style_source
            .checked_add(1)
            .ok_or(Error::BadNumber)?;
        self.next_style_source = slot;
        let source = StyleSourceId {
            slot,
            generation: id.generation,
        };
        self.style_slot_mut(id)?.extra.push(ExtraSource {
            source,
            class_text: String::new(),
            origin,
            order: slot,
            style: Style::new(),
        });
        self.style_source_owners.insert(source, id);
        Ok(source)
    }

    /// Replace one owned source. Stale handles cannot address another node.
    pub fn replace_style_source(
        &mut self,
        source: StyleSourceId,
        style: Style,
    ) -> Result<(), Error> {
        let id = self.source_owner(source).ok_or(Error::UnknownNode)?;
        self.update_sources(id, |next| {
            let extra = next
                .extra
                .iter_mut()
                .find(|extra| extra.source == source)
                .ok_or(Error::UnknownNode)?;
            extra.style = style;
            extra.class_text.clear();
            Ok(())
        })
    }

    pub fn replace_style_source_class(
        &mut self,
        source: StyleSourceId,
        text: &str,
    ) -> Result<(), Error> {
        let id = self.source_owner(source).ok_or(Error::UnknownNode)?;
        self.update_sources(id, |next| {
            let extra = next
                .extra
                .iter_mut()
                .find(|extra| extra.source == source)
                .ok_or(Error::UnknownNode)?;
            if extra.origin != StyleOrigin::Classes {
                return Err(Error::BadNumber);
            }
            extra.class_text = text.to_owned();
            extra.style = Style::new();
            Ok(())
        })
    }

    /// Remove only the source named by this handle.
    pub fn remove_style_source(&mut self, source: StyleSourceId) -> Result<(), Error> {
        let Some(id) = self.source_owner(source) else {
            return Ok(());
        };
        self.update_sources(id, |next| {
            next.extra.retain(|extra| extra.source != source);
            Ok(())
        })?;
        self.style_source_owners.remove(&source);
        Ok(())
    }

    fn replace_origin(
        &mut self,
        id: NodeId,
        origin: StyleOrigin,
        style: Style,
    ) -> Result<(), Error> {
        self.commit_style_snapshot(id, origin, style, true)
    }

    /// Validate one complete source write. A patch retains only current longhands.
    pub fn commit_style_snapshot(
        &mut self,
        id: NodeId,
        origin: StyleOrigin,
        style: Style,
        replace: bool,
    ) -> Result<(), Error> {
        self.update_sources(id, |next| {
            let source = origin_mut(next, origin)?;
            if replace {
                *source = style;
            } else {
                let values = flatten(&style);
                let keys = values.keys().copied().collect::<Vec<_>>();
                clear_keys(source, &keys);
                source.push_fragment(values.into_values().collect())?;
            }
            Ok(())
        })
    }

    /// Resolve and validate the complete candidate before accepting its source.
    fn update_sources(
        &mut self,
        id: NodeId,
        write: impl FnOnce(&mut NodeStyle) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.ensure_style(id)?;
        let mut next = self.style_slot_mut(id)?.clone();
        write(&mut next)?;
        let env = self.style_env.clone();
        let _ = self.prepare_style(id, &next, &env)?;
        self.style_nodes.insert(id.index, next);
        self.enqueue_style(id);
        self.flush_styles()
    }

    /// Merge `patch` into the imperative source. A reset blocks lower sources.
    pub fn patch_style(&mut self, id: NodeId, patch: Patch) -> Result<(), Error> {
        self.commit_style_snapshot(
            id,
            StyleOrigin::Imperative,
            Style::fragment(patch.declarations().to_vec())?,
            false,
        )
    }

    /// Remove imperative keys; lower sources become visible again.
    pub fn clear_style(&mut self, id: NodeId, keys: Option<&[Key]>) -> Result<(), Error> {
        self.update_sources(id, |next| {
            match keys {
                None => next.imperative = Style::new(),
                Some(keys) => clear_keys(&mut next.imperative, keys),
            }
            Ok(())
        })
    }

    pub fn set_class(&mut self, id: NodeId, text: &str) -> Result<(), Error> {
        self.update_sources(id, |next| {
            next.class_text = text.to_owned();
            Ok(())
        })
    }

    /// Register a style surface. Existing author declarations are checked first.
    ///
    /// A rejected registration leaves the previous surface in place and does not
    /// write Taffy.
    pub fn set_style_surface(&mut self, id: NodeId, surface: StyleSurface) -> Result<(), Error> {
        self.ensure_style(id)?;
        let mut next = self
            .style_nodes
            .get(&id.index)
            .filter(|slot| slot.generation == id.generation)
            .cloned()
            .ok_or(Error::UnknownNode)?;
        next.surface = surface;
        let env = self.style_env.clone();
        let _ = self.prepare_style(id, &next, &env)?;
        if let Some(slot) = self.style_nodes.get_mut(&id.index) {
            slot.surface = surface;
        }
        Ok(())
    }

    pub fn set_class_list(&mut self, id: NodeId, text: &str) -> Result<(), Error> {
        self.update_sources(id, |next| {
            next.class_list = text.to_owned();
            Ok(())
        })
    }

    pub(super) fn invalidate_style_deps(&mut self, changed: crate::style::DepMask) {
        if changed.is_empty() {
            return;
        }
        let pending: Vec<NodeId> = self
            .style_nodes
            .iter()
            .filter(|(_, slot)| slot.deps.intersects(changed))
            .map(|(index, slot)| NodeId {
                index: *index,
                generation: slot.generation,
            })
            .collect();
        for id in pending {
            self.enqueue_style(id);
        }
    }

    /// A finished opacity, translation, rotation, or scale stays on the imperative source.
    pub(super) fn settle_style_motion(&mut self, id: NodeId, prop: crate::AnimProp, value: f32) {
        let Some(properties) = motion_properties(prop, value) else {
            return;
        };
        let mut patch = Patch::new();
        for property in properties {
            if patch.set(property).is_err() {
                return;
            }
        }
        let _ = self.patch_style(id, patch);
    }

    /// The session environment, including current window dimensions.
    pub fn style_environment(&self) -> &crate::style::Environment {
        &self.style_env
    }

    /// Validate every affected candidate before replacing the shared environment.
    pub fn set_style_environment(&mut self, env: crate::style::Environment) -> Result<(), Error> {
        env.validate()?;
        let changed = environment_delta(&self.style_env, &env);
        let nodes: Vec<NodeId> = self
            .style_nodes
            .iter()
            .filter(|(_, slot)| slot.deps.intersects(changed))
            .map(|(index, slot)| NodeId {
                index: *index,
                generation: slot.generation,
            })
            .collect();
        for id in &nodes {
            let source = self
                .style_nodes
                .get(&id.index)
                .ok_or(Error::UnknownNode)?
                .clone();
            let _ = self.prepare_style(*id, &source, &env)?;
        }
        self.style_env = env;
        for id in nodes {
            self.enqueue_style(id);
        }
        self.flush_styles()
    }

    /// Gestures produced while hiding a subtree. The session dispatches them
    /// after it releases the tree borrow.
    pub fn take_style_gestures(&mut self) -> Vec<super::Gesture> {
        std::mem::take(&mut self.style_gestures)
    }

    /// Resolve dirty nodes and submit Taffy when a layout value changed.
    pub fn flush_styles(&mut self) -> Result<(), Error> {
        if self.style_queue.is_empty() {
            return Ok(());
        }
        let mut pending = std::mem::take(&mut self.style_queue);
        pending.sort_by_key(|id| self.depth_of(*id));
        pending.dedup();
        self.style_applying = true;
        let result = self.flush_ids(&pending);
        self.style_applying = false;
        if result.is_ok() && !self.style_queue.is_empty() {
            return self.flush_styles();
        }
        result
    }

    pub(super) fn drop_style(&mut self, index: u32) {
        self.style_nodes.remove(&index);
        self.style_source_owners
            .retain(|_, owner| owner.index != index);
        self.style_queue.retain(|id| id.index != index);
    }

    fn flush_ids(&mut self, ids: &[NodeId]) -> Result<(), Error> {
        for id in ids {
            if self.node(*id).is_err() {
                self.style_nodes.remove(&id.index);
                continue;
            }
            self.flush_one(*id)?;
        }
        Ok(())
    }

    fn prepare_style(
        &mut self,
        id: NodeId,
        sources: &NodeStyle,
        env: &crate::style::Environment,
    ) -> Result<Prepared, Error> {
        let class_text = combined_class(&sources.class_text, &sources.class_list);
        let (class_style, mut deps) = if class_text.is_empty() {
            (Style::new(), crate::style::DepMask::empty())
        } else {
            let parsed = self.class_cache.get(&class_text)?;
            (parsed.select(env, self.interaction(id))?, parsed.deps)
        };
        let mut sources = sources.clone();
        for extra in &mut sources.extra {
            if !extra.class_text.is_empty() {
                let parsed = self.class_cache.get(&extra.class_text)?;
                deps |= parsed.deps;
                extra.style = parsed.select(env, self.interaction(id))?;
            }
        }
        reject_surface(id, self.node(id)?.kind, &sources, &class_style)?;
        let (ancestor, environment_root) = self.ancestor_computed(id, env);
        if environment_root {
            deps |= crate::style::DepMask::THEME;
        }
        let resolved = merge_owned(&sources, &class_style).with_inheritance(Some(&ancestor));
        let mut layout = sources.preset.clone();
        let rtl = matches!(
            resolved.property(Key::LayoutDirection),
            Some(Property::LayoutDirection(crate::style::DirectionMode::Rtl))
        );
        for property in layout_map(&resolved).values() {
            crate::style::taffy::apply_layout(&mut layout, property, env, rtl)?;
        }
        let mut presentation = sources.initial.clone();
        for winning in resolved.values.values() {
            if let WinningDeclaration::Value(property) = winning {
                property.validate()?;
                deps |= crate::style::dependencies(property);
                presentation.apply(property, env)?;
            }
        }
        Ok(Prepared {
            resolved,
            layout,
            presentation,
            deps,
        })
    }

    fn flush_one(&mut self, id: NodeId) -> Result<(), Error> {
        let Some(sources) = self
            .style_nodes
            .get(&id.index)
            .filter(|slot| slot.generation == id.generation)
            .cloned()
        else {
            return Ok(());
        };
        let env = self.style_env.clone();
        let prepared = self.prepare_style(id, &sources, &env)?;
        if prepared.layout != self.node(id)?.style {
            self.write_style(id, prepared.layout)?;
        }
        self.apply_presentation(id, prepared.presentation)?;
        self.apply_box_flags(id, &prepared.resolved)?;
        let inherited = inherited_changed(&sources.computed, &prepared.resolved);
        if let Some(slot) = self.style_nodes.get_mut(&id.index) {
            slot.computed = prepared.resolved;
            slot.computed_ready = true;
            slot.deps = prepared.deps;
        }
        if inherited {
            self.enqueue_children(id);
        }
        Ok(())
    }

    fn apply_presentation(&mut self, id: NodeId, next: Presentation) -> Result<(), Error> {
        let node = self.node(id)?;
        let old_paint = node.paint.as_deref().cloned().unwrap_or_default();
        let paint_changed = old_paint != next.paint;
        let mut picture_paint = old_paint.clone();
        picture_paint.alpha = next.paint.alpha;
        picture_paint.blur = next.paint.blur;
        picture_paint.backdrop = next.paint.backdrop;
        picture_paint.blend = next.paint.blend;
        let picture_changed = picture_paint != next.paint;
        let topology_changed = alpha_bucket(old_paint.alpha.unwrap_or(255))
            != alpha_bucket(next.paint.alpha.unwrap_or(255))
            || (old_paint.blur > 0.0) != (next.paint.blur > 0.0)
            || (old_paint.backdrop > 0.0) != (next.paint.backdrop > 0.0)
            || (old_paint.blend != protocol::BlendMode::SrcOver)
                != (next.paint.blend != protocol::BlendMode::SrcOver)
            || node.transform().is_identity()
                != next
                    .transform
                    .unwrap_or_else(TransformGeom::identity)
                    .is_identity();
        let transform_changed = node.transform.as_deref().copied() != next.transform;
        let mut measure_changed = false;
        let mut text_changed = false;
        if let (Some(text), Some(initial)) = (node.text(), next.text.as_ref()) {
            let mut metrics_style = (*text.style).clone();
            metrics_style.underline = initial.style.underline;
            metrics_style.line_through = initial.style.line_through;
            measure_changed = text.font_size != initial.size || metrics_style != *initial.style;
            text_changed =
                measure_changed || text.color != initial.color || text.style != initial.style;
        }
        let image_changed = node
            .image()
            .is_some_and(|image| Some(image.mode) != next.image);
        if !(paint_changed || transform_changed || text_changed || image_changed) {
            return Ok(());
        }
        let node = self.node_mut(id)?;
        if paint_changed {
            node.paint = Some(Box::new(next.paint));
        }
        if transform_changed {
            node.transform = next.transform.map(Box::new);
            node.role.set(Role::TRANSFORM, node.transform.is_some());
        }
        if let (Some(text), Some(initial)) = (node.text_mut(), next.text) {
            text.font_size = initial.size;
            text.color = initial.color;
            text.style = initial.style;
        }
        if let (Some(image), Some(mode)) = (node.image_mut(), next.image) {
            image.mode = mode;
        }
        if measure_changed {
            self.touch_measure(id)?;
        }
        if picture_changed || text_changed || image_changed {
            self.mark_repaint(id);
        }
        if topology_changed {
            self.repaint_ancestors(id);
        } else if transform_changed || paint_changed {
            self.paint_layer_value();
        }
        Ok(())
    }

    fn ancestor_computed(
        &self,
        id: NodeId,
        env: &crate::style::Environment,
    ) -> (crate::style::Resolved, bool) {
        let mut current = self.node(id).ok().and_then(|node| node.parent);
        while let Some(parent) = current {
            if let Some(slot) = self.style_nodes.get(&parent.index)
                && slot.generation == parent.generation
                && slot.computed_ready
            {
                return (slot.computed.clone(), false);
            }
            current = self.node(parent).ok().and_then(|node| node.parent);
        }
        let mut resolved = crate::style::Resolved::default();
        for property in [
            Property::LayoutDirection(env.direction),
            Property::WritingDirectionRtl(env.direction == crate::style::DirectionMode::Rtl),
        ] {
            resolved
                .values
                .insert(property.key(), WinningDeclaration::Value(property));
        }
        (resolved, true)
    }

    fn apply_box_flags(
        &mut self,
        id: NodeId,
        resolved: &crate::style::Resolved,
    ) -> Result<(), Error> {
        let hidden = matches!(
            resolved.property(Key::Display),
            Some(Property::Display(crate::style::DisplayMode::None))
        );
        let was_hidden = self.node(id)?.flags.contains(NodeFlags::HIDDEN);
        if hidden != was_hidden {
            self.node_mut(id)?.flags.set(NodeFlags::HIDDEN, hidden);
            self.mark_repaint(id);
            if hidden {
                let notices = self.conceal(id);
                self.style_gestures.extend(notices);
            }
        }
        let z = match resolved.property(Key::ZIndex) {
            Some(Property::ZIndex(z)) => *z,
            _ => 0,
        };
        if self.node(id)?.z_index != z {
            self.node_mut(id)?.z_index = z;
            if let Some(parent) = self.node(id)?.parent {
                self.mark_repaint(parent);
            } else {
                self.mark_repaint(id);
            }
        }
        let initial_clip = self
            .style_nodes
            .get(&id.index)
            .is_some_and(|slot| slot.initial_clip);
        let mut clip = initial_clip;
        for key in [Key::Overflow, Key::OverflowX, Key::OverflowY] {
            if let Some(property) = resolved.property(key) {
                let mode = match property {
                    Property::Overflow(mode)
                    | Property::OverflowX(mode)
                    | Property::OverflowY(mode) => mode,
                    _ => continue,
                };
                clip = *mode != crate::style::OverflowMode::Visible;
                if clip {
                    break;
                }
            }
        }
        if !self.node(id)?.is_scroll() && self.node(id)?.role.contains(Role::CLIP) != clip {
            self.node_mut(id)?.role.set(Role::CLIP, clip);
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    fn source_owner(&self, source: StyleSourceId) -> Option<NodeId> {
        self.style_source_owners
            .get(&source)
            .copied()
            .filter(|id| self.node(*id).is_ok())
    }

    fn interaction(&self, id: NodeId) -> Interaction {
        Interaction {
            hover: self
                .arena
                .hovered
                .is_some_and(|node| self.is_under(node, id).unwrap_or(false)),
            active: self.arena.routes.values().any(|route| {
                route
                    .pressed
                    .is_some_and(|node| self.is_under(node, id).unwrap_or(false))
            }),
            focus: self.focused == Some(id),
            disabled: self
                .node(id)
                .ok()
                .and_then(|node| node.semantics.as_ref())
                .and_then(|hint| hint.disabled)
                .unwrap_or(false),
        }
    }

    fn ensure_style(&mut self, id: NodeId) -> Result<(), Error> {
        let generation = id.generation;
        let _ = self.node(id)?;
        let known = self
            .style_nodes
            .get(&id.index)
            .is_some_and(|slot| slot.generation == generation);
        if known {
            return Ok(());
        }
        let preset = self.node(id)?.style.clone();
        let initial = Presentation::capture(self.node(id)?);
        self.style_nodes.insert(
            id.index,
            NodeStyle {
                generation,
                preset,
                component: Style::new(),
                class_text: String::new(),
                class_list: String::new(),
                inline: Style::new(),
                imperative: Style::new(),
                animation: Style::new(),
                extra: Vec::new(),
                computed: crate::style::Resolved::default(),
                computed_ready: false,
                initial,
                initial_clip: self.node(id)?.role.contains(Role::CLIP),
                deps: crate::style::DepMask::empty(),
                surface: StyleSurface::General,
            },
        );
        Ok(())
    }

    fn style_slot_mut(&mut self, id: NodeId) -> Result<&mut NodeStyle, Error> {
        self.style_nodes
            .get_mut(&id.index)
            .filter(|slot| slot.generation == id.generation)
            .ok_or(Error::UnknownNode)
    }

    fn enqueue_style(&mut self, id: NodeId) {
        if !self.style_queue.contains(&id) {
            self.style_queue.push(id);
        }
    }

    fn enqueue_children(&mut self, id: NodeId) {
        let mut child = self.get(id).and_then(|node| node.first_child);
        while let Some(current) = child {
            let next = self.get(current).and_then(|node| node.next);
            if self.ensure_style(current).is_ok() {
                self.enqueue_style(current);
            }
            child = next;
        }
    }

    fn depth_of(&self, id: NodeId) -> u32 {
        let mut depth = 0u32;
        let mut current = Some(id);
        while let Some(node_id) = current {
            depth = depth.saturating_add(1);
            current = self.node(node_id).ok().and_then(|node| node.parent);
        }
        depth
    }
}

fn merge_owned(sources: &NodeStyle, class_style: &Style) -> crate::style::Resolved {
    let mut owned = Vec::new();
    for origin in [
        StyleOrigin::Component,
        StyleOrigin::Classes,
        StyleOrigin::Inline,
        StyleOrigin::Imperative,
        StyleOrigin::Animation,
    ] {
        owned.push(match origin {
            StyleOrigin::Component => sources.component.clone(),
            StyleOrigin::Classes => class_style.clone(),
            StyleOrigin::Inline => sources.inline.clone(),
            StyleOrigin::Imperative => sources.imperative.clone(),
            StyleOrigin::Animation => sources.animation.clone(),
        });
        let mut extras: Vec<&ExtraSource> = sources
            .extra
            .iter()
            .filter(|extra| extra.origin == origin)
            .collect();
        extras.sort_by_key(|extra| extra.order);
        for extra in extras {
            owned.push(extra.style.clone());
        }
    }
    let refs: Vec<&Style> = owned.iter().collect();
    merge_layers(&refs)
}

fn layout_map(resolved: &crate::style::Resolved) -> BTreeMap<Key, Property> {
    let mut map = BTreeMap::new();
    for (key, winning) in &resolved.values {
        if !key.impact().contains(crate::style::Impact::LAYOUT) {
            continue;
        }
        let WinningDeclaration::Value(property) = winning else {
            continue;
        };
        if skips_logical(*key, resolved) {
            continue;
        }
        map.insert(*key, property.clone());
    }
    map
}

fn motion_properties(prop: crate::AnimProp, value: f32) -> Option<Vec<Property>> {
    if !value.is_finite() {
        return None;
    }
    match prop {
        crate::AnimProp::Opacity => Some(vec![Property::Opacity((value / 255.0).clamp(0.0, 1.0))]),
        crate::AnimProp::TranslateX => Some(vec![Property::TranslateX(
            crate::style::Length::points(value),
        )]),
        crate::AnimProp::TranslateY => Some(vec![Property::TranslateY(
            crate::style::Length::points(value),
        )]),
        crate::AnimProp::Rotate => Some(vec![Property::Rotate(value)]),
        crate::AnimProp::Scale => Some(vec![Property::ScaleX(value), Property::ScaleY(value)]),
        _ => None,
    }
}

fn environment_delta(
    previous: &crate::style::Environment,
    next: &crate::style::Environment,
) -> crate::style::DepMask {
    use crate::style::DepMask;
    let mut changed = DepMask::empty();
    if previous.window_width != next.window_width
        || previous.window_height != next.window_height
        || previous.breakpoints != next.breakpoints
        || previous.spacing != next.spacing
        || previous.root_rem != next.root_rem
    {
        changed |= DepMask::WINDOW;
    }
    if previous.dark != next.dark {
        changed |= DepMask::DARK;
    }
    if !previous.same_theme(next) {
        changed |= DepMask::THEME;
    }
    changed
}

fn inherited_changed(before: &crate::style::Resolved, after: &crate::style::Resolved) -> bool {
    const KEYS: [Key; 8] = [
        Key::Color,
        Key::FontFamily,
        Key::FontSize,
        Key::FontWeight,
        Key::LineHeight,
        Key::TextAlign,
        Key::WritingDirection,
        Key::LayoutDirection,
    ];
    KEYS.iter()
        .any(|key| before.property(*key) != after.property(*key))
}

fn effective_surface(kind: Kind, stored: StyleSurface) -> StyleSurface {
    if stored == StyleSurface::ListContent {
        return StyleSurface::ListContent;
    }
    match kind {
        Kind::Stack => StyleSurface::Stack,
        Kind::Scroll => StyleSurface::ScrollViewport,
        Kind::View | Kind::Text | Kind::Image => StyleSurface::General,
    }
}

fn reject_style(
    style: &Style,
    origin: StyleOrigin,
    surface: StyleSurface,
    raw: u64,
) -> Result<(), Error> {
    if surface == StyleSurface::General || origin == StyleOrigin::Component {
        return Ok(());
    }
    for declaration in flatten(style).into_values() {
        if author_key_allowed(surface, &declaration) {
            continue;
        }
        return Err(StyleError::Forbidden {
            node: Some(raw),
            origin: origin.name(),
            key: declaration.key().name(),
        }
        .into());
    }
    Ok(())
}

fn reject_surface(
    id: NodeId,
    kind: Kind,
    sources: &NodeStyle,
    class_style: &Style,
) -> Result<(), Error> {
    let surface = effective_surface(kind, sources.surface);
    let raw = id.to_raw();
    reject_style(class_style, StyleOrigin::Classes, surface, raw)?;
    reject_style(&sources.inline, StyleOrigin::Inline, surface, raw)?;
    reject_style(&sources.imperative, StyleOrigin::Imperative, surface, raw)?;
    reject_style(&sources.animation, StyleOrigin::Animation, surface, raw)?;
    for extra in &sources.extra {
        reject_style(&extra.style, extra.origin, surface, raw)?;
    }
    Ok(())
}

fn combined_class(class_text: &str, class_list: &str) -> String {
    if class_list.is_empty() {
        return class_text.to_owned();
    }
    if class_text.is_empty() {
        return class_list.to_owned();
    }
    let mut combined = String::with_capacity(class_text.len() + class_list.len() + 1);
    combined.push_str(class_text);
    combined.push(' ');
    combined.push_str(class_list);
    combined
}

fn origin_parts(
    length: &crate::style::Length,
    env: &crate::style::Environment,
) -> Result<(f32, f32), Error> {
    if let crate::style::Length::Fraction(value) = length {
        return Ok((0.0, *value));
    }
    Ok((env.resolve_points(length)?, 0.0))
}

fn clear_keys(style: &mut Style, keys: &[Key]) {
    let kept = style
        .fragments()
        .iter()
        .map(|fragment| {
            fragment
                .iter()
                .filter(|declaration| !keys.contains(&declaration.key()))
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|fragment| !fragment.is_empty())
        .collect::<Vec<_>>();
    *style = Style::new();
    for fragment in kept {
        let _ = style.push_fragment(fragment);
    }
}

fn skips_logical(key: Key, resolved: &crate::style::Resolved) -> bool {
    let set = |key: Key| resolved.property(key).is_some();
    match key {
        Key::PaddingInlineStart | Key::PaddingInlineEnd => {
            set(Key::PaddingLeft) || set(Key::PaddingRight)
        }
        Key::MarginInlineStart | Key::MarginInlineEnd => {
            set(Key::MarginLeft) || set(Key::MarginRight)
        }
        Key::InsetInlineStart | Key::InsetInlineEnd => set(Key::Left) || set(Key::Right),
        _ => false,
    }
}

fn opacity_byte(value: f32) -> u8 {
    let scaled = (value.clamp(0.0, 1.0) * 255.0).round();
    if scaled <= 0.0 {
        0
    } else if scaled >= 255.0 {
        255
    } else {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "scaled is inside 1..=254 after the range checks"
        )]
        {
            scaled as u8
        }
    }
}

fn alpha_bucket(alpha: u8) -> u8 {
    if alpha == 0 {
        0
    } else if alpha == 255 {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::super::Tree;
    use crate::style::{Declaration, DisplayMode, Key, PaintColor, Property, Style};
    use crate::{Error, MonoShaper};

    fn color_style(color: [u8; 4]) -> Style {
        Style::fragment(vec![Declaration::set(Property::Color(PaintColor::rgba(
            color,
        )))])
        .expect("color")
    }

    #[test]
    fn repeated_patches_do_not_retain_obsolete_declarations() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view()?;
        for width in 0_u16..1000 {
            let mut patch = crate::style::Patch::new();
            patch.set(Property::Width(crate::style::SizeValue::Length(
                crate::style::Length::points(f32::from(width)),
            )))?;
            tree.patch_style(view, patch)?;
        }
        let source = &tree.style_nodes[&view.index].imperative;
        assert_eq!(source.fragments().iter().map(Vec::len).sum::<usize>(), 1);
        tree.layout()?;
        assert_eq!(tree.rect(view)?.w, 999.0);
        Ok(())
    }

    #[test]
    fn a_new_view_shrinks_nothing_and_has_no_parent_max() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view()?;
        let style = tree.node(view)?.style.clone();
        assert_eq!(style.flex_shrink, 0.0);
        assert_eq!(style.flex_grow, 0.0);
        assert_eq!(style.min_size.width, taffy::prelude::length(0.0));
        assert_ne!(style.max_size.width, taffy::prelude::percent(1.0));
        Ok(())
    }

    #[test]
    fn background_color_does_not_ask_for_layout() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view()?;
        tree.layout()?;
        tree.set_style(
            view,
            Style::fragment(vec![Declaration::set(Property::BackgroundColor(
                PaintColor::rgba([1, 2, 3, 255]),
            ))])?,
        )?;
        assert!(!tree.needs_layout());
        assert_eq!(tree.node(view)?.fill(), Some([1, 2, 3, 255]));
        Ok(())
    }

    #[test]
    fn text_inherits_through_a_parent_that_only_forwards_color() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.view()?;
        let middle = tree.view()?;
        let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        tree.add_child(root, middle)?;
        tree.add_child(middle, text)?;
        tree.set_style(root, color_style([9, 8, 7, 255]))?;
        tree.layout()?;
        let painted = tree.record()?;
        let found = painted.commands.iter().find_map(|command| match command {
            protocol::Command::Text { color, .. } => Some(*color),
            _ => None,
        });
        assert_eq!(found, Some([9, 8, 7, 255]));
        Ok(())
    }

    #[test]
    fn reset_blocks_inherited_color() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.view()?;
        let text = tree.text("Hi", 16.0, [4, 5, 6, 255])?;
        tree.add_child(root, text)?;
        tree.set_style(root, color_style([9, 8, 7, 255]))?;
        tree.set_inline_style(text, Style::fragment(vec![Declaration::reset(Key::Color)])?)?;
        tree.layout()?;
        let painted = tree.record()?;
        let found = painted.commands.iter().find_map(|command| match command {
            protocol::Command::Text { color, .. } => Some(*color),
            _ => None,
        });
        assert_eq!(found, Some([4, 5, 6, 255]));
        Ok(())
    }

    #[test]
    fn dark_variant_restyles_only_after_the_environment_changes() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view()?;
        tree.set_class(view, "dark:hidden")?;
        tree.layout()?;
        assert!(
            !tree
                .node(view)?
                .flags
                .contains(super::super::node::NodeFlags::HIDDEN)
        );
        tree.set_style_environment(crate::style::Environment::dark())?;
        tree.flush_styles()?;
        tree.layout()?;
        assert!(
            tree.node(view)?
                .flags
                .contains(super::super::node::NodeFlags::HIDDEN)
        );
        Ok(())
    }

    #[test]
    fn display_none_is_not_hit_and_not_recorded() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let view = tree.view()?;
        tree.layout()?;
        tree.set_size(view, 40.0, 40.0)?;
        tree.layout()?;
        tree.set_style(
            view,
            Style::fragment(vec![Declaration::set(Property::Display(DisplayMode::None))])?,
        )?;
        tree.layout()?;
        assert!(tree.hit_root(10.0, 10.0).is_none());
        let painted = tree.record()?;
        assert!(painted.commands.is_empty());
        Ok(())
    }
}
