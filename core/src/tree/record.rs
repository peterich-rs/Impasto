//! Record a [`DrawList`] from the laid-out tree.

use std::sync::Arc;

use protocol::{Command, DrawList};

use super::node::Kind;
use super::{NodeId, Tree};
use crate::Error;

/// Axis-aligned clip in paint space. `None` means a transform ancestor made culling unsafe.
#[derive(Clone, Copy)]
struct PaintClip {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Tree {
    pub fn record(&self) -> Result<DrawList, Error> {
        let mut commands = Vec::new();
        self.record_into(&mut commands)?;
        Ok(DrawList { commands })
    }

    /// Clears `commands` and refills them. The caller can keep the buffer's capacity.
    pub fn record_into(&self, commands: &mut Vec<Command>) -> Result<(), Error> {
        commands.clear();
        let _ = self.record_node(self.root.ok_or(Error::NoRoot)?, commands, (0.0, 0.0), None);
        Ok(())
    }

    /// Nodes `record` walks, including rows whose clipped children are skipped.
    pub fn record_visit_count(&self) -> Result<usize, Error> {
        let mut commands = Vec::new();
        Ok(self.record_node(
            self.root.ok_or(Error::NoRoot)?,
            &mut commands,
            (0.0, 0.0),
            None,
        ))
    }

    /// Independent scale for a transform node. Non-finite values or values whose
    /// absolute value is below `1e-4` are rejected and leave the node unchanged.
    pub fn set_scale_xy(&mut self, id: NodeId, scale_x: f32, scale_y: f32) -> Result<(), Error> {
        if !scale_x.is_finite()
            || !scale_y.is_finite()
            || scale_x.abs() < 1.0e-4
            || scale_y.abs() < 1.0e-4
        {
            return Err(Error::BadNumber);
        }
        let node = self.node_mut(id)?;
        if node.kind != Kind::Transform {
            return Err(Error::WrongKind);
        }
        node.scale = scale_x;
        node.scale_y = scale_y;
        self.mark_paint();
        Ok(())
    }

    /// Center stroke on this node's fill rectangle. Non-positive width clears it.
    pub fn set_stroke(&mut self, id: NodeId, width: f32, color: [u8; 4]) -> Result<(), Error> {
        let node = self.node_mut(id)?;
        node.stroke_width = width;
        node.stroke_color = color;
        self.mark_paint();
        Ok(())
    }

    /// Unit-square gradient. Fewer than two legal stops records as a solid fill.
    pub fn set_gradient(
        &mut self,
        id: NodeId,
        gradient: Option<protocol::Gradient>,
    ) -> Result<(), Error> {
        self.node_mut(id)?.gradient = gradient;
        self.mark_paint();
        Ok(())
    }

    /// Material shadow. Non-positive elevation records no shadow.
    pub fn set_shadow(
        &mut self,
        id: NodeId,
        shadow: Option<protocol::Shadow>,
    ) -> Result<(), Error> {
        self.node_mut(id)?.shadow = shadow;
        self.mark_paint();
        Ok(())
    }

    /// Foreground blur in scene points. Non-positive or non-finite sigma records nothing.
    pub fn set_blur(&mut self, id: NodeId, sigma: f32) -> Result<(), Error> {
        self.node_mut(id)?.blur = sigma;
        self.mark_paint();
        Ok(())
    }

    /// Backdrop blur in scene points. Non-positive or non-finite sigma records nothing.
    pub fn set_backdrop_blur(&mut self, id: NodeId, sigma: f32) -> Result<(), Error> {
        self.node_mut(id)?.backdrop = sigma;
        self.mark_paint();
        Ok(())
    }

    /// Layer blend. [`protocol::BlendMode::SrcOver`] records no push and no pop.
    pub fn set_blend(&mut self, id: NodeId, mode: protocol::BlendMode) -> Result<(), Error> {
        self.node_mut(id)?.blend = mode;
        self.mark_paint();
        Ok(())
    }

    pub(super) fn record_node(
        &self,
        id: NodeId,
        commands: &mut Vec<Command>,
        offset: (f32, f32),
        clip: Option<PaintClip>,
    ) -> usize {
        let Some(node) = self.get(id) else {
            return 0;
        };
        let x = node.x - offset.0;
        let y = node.y - offset.1;
        if node.kind != Kind::Transform && outside(clip, x, y, node.w, node.h) {
            return 1;
        }
        let (fill, stroke_width, stroke_color, gradient, shadow) = rect_paint(node);
        if node.w > 0.0
            && node.h > 0.0
            && (fill.is_some() || stroke_width > 0.0 || gradient.is_some() || shadow.is_some())
        {
            let mut color = fill.unwrap_or([0, 0, 0, 0]);
            let gradient = match gradient.as_ref().map(prepared_gradient) {
                Some(PreparedGradient::Paint(gradient)) => Some(gradient),
                Some(PreparedGradient::Solid(stop)) => {
                    color = stop;
                    None
                }
                None | Some(PreparedGradient::Drop) => None,
            };
            commands.push(Command::Rect {
                x,
                y,
                w: node.w,
                h: node.h,
                corner_radius: node.corner_radius,
                color,
                stroke_width,
                stroke_color,
                gradient,
                shadow,
            });
        }
        if let Some(text) = &node.text
            && !text.utf8.is_empty()
        {
            let clip_overflow = text.style.overflow == protocol::TextOverflow::Clip
                && !text.style.soft_wrap
                && node.w > 0.0
                && node.h > 0.0;
            if clip_overflow {
                commands.push(Command::PushClip {
                    x,
                    y,
                    w: node.w,
                    h: node.h,
                    corner_radius: 0.0,
                });
            }
            let spans = text.spans.clone();
            let links = text.links.clone();
            let utf8 = if spans.is_empty() {
                Arc::from(text.utf8.as_str())
            } else {
                let joined: String = spans.iter().map(|span| span.utf8.as_str()).collect();
                debug_assert_eq!(joined, text.utf8);
                Arc::from(joined)
            };
            commands.push(Command::Text {
                x,
                y,
                max_width: text.max_width.get(),
                font_size: text.font_size * self.text_scale,
                color: text.color,
                utf8,
                style: text.style.clone(),
                spans,
                links,
            });
            if clip_overflow {
                commands.push(Command::Pop);
            }
        }
        if let Some(image) = &node.image {
            commands.push(Command::Image {
                x,
                y,
                w: node.w,
                h: node.h,
                source: Arc::from(image.source.as_str()),
                mode: image.mode,
                required: image.required,
            });
        }
        let kind = node.kind;
        let (w, h) = (node.w, node.h);
        let corner_radius = node.corner_radius;
        let alpha = node.alpha;
        let scroll = node.scroll;
        let transform = (
            node.translate_x,
            node.translate_y,
            node.rotate_rad,
            node.scale,
            node.scale_y,
        );
        let blur = positive_sigma(node.blur);
        let backdrop = positive_sigma(node.backdrop);
        let blend = (node.blend != protocol::BlendMode::SrcOver).then_some(node.blend);
        let children = node.children.clone();
        let child_offset = if kind == Kind::Scroll {
            (offset.0 + scroll.0, offset.1 + scroll.1)
        } else {
            offset
        };
        let child_clip = match kind {
            Kind::Transform => None,
            Kind::Scroll | Kind::Clip => intersect(clip, x, y, w, h),
            _ => clip,
        };
        match kind {
            Kind::Clip => commands.push(Command::PushClip {
                x,
                y,
                w,
                h,
                corner_radius,
            }),
            Kind::Opacity => commands.push(Command::PushOpacity {
                x,
                y,
                w,
                h,
                alpha: alpha.unwrap_or(255),
            }),
            Kind::Scroll => commands.push(Command::PushClip {
                x,
                y,
                w,
                h,
                corner_radius: 0.0,
            }),
            Kind::Transform => commands.push(Command::PushTransform {
                translate_x: transform.0,
                translate_y: transform.1,
                rotate_rad: transform.2,
                scale_x: transform.3,
                scale_y: transform.4,
            }),
            _ => {}
        }
        if let Some(mode) = blend {
            commands.push(Command::PushBlend { x, y, w, h, mode });
        }
        if let Some(sigma) = blur {
            commands.push(Command::PushBlur { x, y, w, h, sigma });
        }
        if let Some(sigma) = backdrop {
            commands.push(Command::PushBackdropBlur { x, y, w, h, sigma });
        }
        let mut visits = 1;
        for child in children {
            visits += self.record_node(child, commands, child_offset, child_clip);
        }
        if backdrop.is_some() {
            commands.push(Command::Pop);
        }
        if blur.is_some() {
            commands.push(Command::Pop);
        }
        if blend.is_some() {
            commands.push(Command::Pop);
        }
        if matches!(
            kind,
            Kind::Clip | Kind::Opacity | Kind::Scroll | Kind::Transform
        ) {
            commands.push(Command::Pop);
        }
        visits
    }
}

fn positive_sigma(sigma: f32) -> Option<f32> {
    if sigma.is_finite() && sigma > 0.0 {
        Some(sigma)
    } else {
        None
    }
}

fn rect_paint(
    node: &super::node::Node,
) -> (
    Option<[u8; 4]>,
    f32,
    [u8; 4],
    Option<protocol::Gradient>,
    Option<protocol::Shadow>,
) {
    let stroke_width = if node.stroke_width.is_finite() && node.stroke_width > 0.0 {
        node.stroke_width
    } else {
        0.0
    };
    let shadow = node
        .shadow
        .filter(|shadow| shadow.elevation.is_finite() && shadow.elevation > 0.0);
    (
        node.fill,
        stroke_width,
        node.stroke_color,
        node.gradient.clone(),
        shadow,
    )
}

enum PreparedGradient {
    Paint(protocol::Gradient),
    Solid([u8; 4]),
    Drop,
}

fn prepared_gradient(gradient: &protocol::Gradient) -> PreparedGradient {
    let (stops, linear_ends, radial) = match gradient {
        protocol::Gradient::Linear { start, end, stops } => (stops, Some((*start, *end)), None),
        protocol::Gradient::Radial {
            center,
            radius,
            stops,
        } => (stops, None, Some((*center, *radius))),
    };
    if stops.len() < 2
        || stops
            .iter()
            .any(|stop| !stop.offset.is_finite() || !(0.0..=1.0).contains(&stop.offset))
    {
        return PreparedGradient::Drop;
    }
    let mut stops = stops.clone();
    stops.sort_by(|left, right| {
        left.offset
            .partial_cmp(&right.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some((start, end)) = linear_ends {
        if !start
            .iter()
            .chain(end.iter())
            .all(|value| value.is_finite())
        {
            return PreparedGradient::Drop;
        }
        if start == end {
            return PreparedGradient::Solid(
                stops.last().map(|stop| stop.color).unwrap_or([0, 0, 0, 0]),
            );
        }
        return PreparedGradient::Paint(protocol::Gradient::Linear { start, end, stops });
    }
    let Some((center, radius)) = radial else {
        return PreparedGradient::Drop;
    };
    if !center[0].is_finite() || !center[1].is_finite() || !radius.is_finite() || radius <= 0.0 {
        return PreparedGradient::Drop;
    }
    PreparedGradient::Paint(protocol::Gradient::Radial {
        center,
        radius,
        stops,
    })
}

fn outside(clip: Option<PaintClip>, x: f32, y: f32, w: f32, h: f32) -> bool {
    let Some(clip) = clip else {
        return false;
    };
    x + w <= clip.x || y + h <= clip.y || x >= clip.x + clip.w || y >= clip.y + clip.h
}

fn intersect(clip: Option<PaintClip>, x: f32, y: f32, w: f32, h: f32) -> Option<PaintClip> {
    let Some(clip) = clip else {
        return Some(PaintClip { x, y, w, h });
    };
    let left = clip.x.max(x);
    let top = clip.y.max(y);
    let right = (clip.x + clip.w).min(x + w);
    let bottom = (clip.y + clip.h).min(y + h);
    if right <= left || bottom <= top {
        None
    } else {
        Some(PaintClip {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        })
    }
}

#[cfg(test)]
mod record_bench {
    use super::*;
    use crate::MonoShaper;
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    const WARMUP: usize = 10;
    const SAMPLES: usize = 40;
    const P95_INDEX: usize = 37;

    fn build_rows(n: usize, scrolled: bool) -> Result<(Tree, Option<NodeId>, NodeId), Error> {
        let mut tree = Tree::new(MonoShaper);
        let scroll = if scrolled { Some(tree.scroll()?) } else { None };
        let column = tree.column(0.0)?;
        for index in 0..n {
            let row = tree.pointer_region()?;
            let pad = tree.padding(4.0)?;
            let deco = tree.decorated([20, 20, 20, 255], 0.0)?;
            let text = tree.text(format!("Row {index}"), 16.0, [240, 240, 240, 255])?;
            tree.add_child(deco, text)?;
            tree.add_child(pad, deco)?;
            tree.add_child(row, pad)?;
            tree.add_child(column, row)?;
        }
        if let Some(scroll) = scroll {
            tree.add_child(scroll, column)?;
        }
        Ok((tree, scroll, column))
    }

    fn p95(samples: &mut [Duration]) -> Duration {
        samples.sort();
        samples[P95_INDEX]
    }

    fn sample_pass(tree: &mut Tree, previous: &DrawList) -> (Duration, Duration, Duration) {
        let started = Instant::now();
        tree.layout().unwrap();
        let layout = started.elapsed();
        let started = Instant::now();
        let list = tree.record().unwrap();
        let record = started.elapsed();
        let started = Instant::now();
        let _ = black_box(previous == &list);
        let compare = started.elapsed();
        black_box(list);
        (layout, record, compare)
    }

    fn run_samples(n: usize, viewport_h: f32) -> Result<(Duration, Duration, Duration), Error> {
        let (mut tree, _, _) = build_rows(n, false)?;
        tree.set_viewport(0.0, 0.0, 360.0, viewport_h)?;
        tree.layout()?;
        let mut previous = tree.record()?;
        let mut layouts = Vec::with_capacity(SAMPLES);
        let mut records = Vec::with_capacity(SAMPLES);
        let mut compares = Vec::with_capacity(SAMPLES);
        for index in 0..WARMUP + SAMPLES {
            let (layout, record, compare) = sample_pass(&mut tree, &previous);
            previous = tree.record()?;
            if index >= WARMUP {
                layouts.push(layout);
                records.push(record);
                compares.push(compare);
            }
        }
        Ok((p95(&mut layouts), p95(&mut records), p95(&mut compares)))
    }

    #[test]
    #[ignore]
    fn record_bench() {
        for n in [100usize, 1000, 5000] {
            let height = 1_000_000.0;
            let (layout, record, compare) = run_samples(n, height).expect("bench tree");
            println!(
                "visible n={n} layout_p95={:.3}ms record_p95={:.3}ms compare_p95={:.3}ms",
                layout.as_secs_f64() * 1000.0,
                record.as_secs_f64() * 1000.0,
                compare.as_secs_f64() * 1000.0
            );
            if n == 5000 && cfg!(all(target_os = "macos", target_arch = "aarch64")) {
                assert!(
                    record.as_secs_f64() * 1000.0 <= 2.0,
                    "record p95 {}ms",
                    record.as_secs_f64() * 1000.0
                );
                assert!(
                    compare.as_secs_f64() * 1000.0 <= 1.0,
                    "compare p95 {}ms",
                    compare.as_secs_f64() * 1000.0
                );
            }
        }

        let (mut tree, scroll, column) = build_rows(5000, true).expect("scroll tree");
        let scroll = scroll.expect("scroll");
        tree.set_viewport(0.0, 0.0, 360.0, 640.0).expect("viewport");
        tree.layout().expect("layout");
        let content = tree.rect(column).expect("content");
        tree.set_scroll_offset(scroll, 0.0, (content.h * 0.5 - 320.0).max(0.0))
            .expect("offset");
        let started = Instant::now();
        let visits = tree.record_visit_count().expect("visits");
        let record = started.elapsed();
        println!(
            "scroll 5000 visits={visits} record={:.3}ms",
            record.as_secs_f64() * 1000.0
        );
    }
}
