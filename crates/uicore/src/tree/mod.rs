//! Render tree. Layout goes through Taffy. Positions, hits, and gestures stay on this type.

mod debug;
mod geometry;
mod gesture;
mod hit;
mod layout;
mod node;
mod record;
mod scroll;
mod semantics;
mod store;
mod style;
mod text;

pub use debug::{DebugAxis, LayoutIssue};
#[cfg(test)]
pub(crate) use hit::Affine;
pub use layout::{LayoutCause, LayoutTrace};
pub use node::{
    Alignment, AxisSize, BoxEdge, Discarded, DragClaim, Edges, FocusDirection, Gesture, GridLine,
    Inset, Justify, NodeId, PanelAlign, Rect, RowCrossAlign, ScrollEdge, SubtreeSpace, TrackSize,
};
pub use scroll::ScrollSample;

use crate::{AnimProp, Simulation};

struct ActiveMotion {
    started_ms: f64,
    node: NodeId,
    prop: AnimProp,
    simulation: Box<dyn Simulation>,
    /// This spring is the snap that follows coast. Its end must not snap again.
    snap: bool,
}

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use node::Slot;
use taffy::TaffyTree;

#[cfg(test)]
use node::MAX_GENERATION;

use crate::Shaper;

/// Render tree. `layout` writes positions. `pointer` updates click and drag.
///
/// The shaper is erased here so one tree type serves the test shaper and the
/// host's Impeller shaper. The trait is not `Send`: an Impeller typography
/// context stays on the UI thread, and so does the tree.
pub struct Tree {
    slots: Vec<Slot>,
    free: Vec<u32>,
    taffy: TaffyTree<usize>,
    shaper: Box<dyn Shaper>,
    /// [`Shaper::begin_frame`] stays open until [`Tree::record_into`] closes it.
    pub(super) measure_open: bool,
    root: Option<NodeId>,
    /// Subtrees [`Tree::remove`] detached. [`Tree::discard_detached`] retires
    /// the ones that are still parentless.
    pending_discard: Vec<NodeId>,
    /// Press, drag, and hover routes. Structure edits do not clear this.
    arena: gesture::PointerTable,
    focused: Option<NodeId>,
    /// Innermost focus scope. Empty means the whole tree.
    scopes: Vec<NodeId>,
    /// Scroll friction and property springs, sampled by [`Tree::tick_motions`].
    motions: Vec<ActiveMotion>,
    /// Motions that finished since the last [`Tree::take_motion_ends`].
    motion_ends: Vec<(NodeId, AnimProp)>,
    /// Last timestamp passed to [`Tree::tick_motions`]. New motions start here.
    clock_ms: f64,
    /// Set while a sample writes a property, so that write does not rebase itself.
    applying_motion: bool,
    /// Relayout boundaries whose children must be read back from Taffy.
    dirty_boundaries: Vec<NodeId>,
    /// Child picture of a scroll. Keyed by the scroll node.
    pictures: HashMap<NodeId, Arc<protocol::Picture>>,
    /// Runs keyed by the owner that closed them, in paint order.
    runs: HashMap<NodeId, Vec<Arc<protocol::Picture>>>,
    /// Capture in progress. Empty outside `record_into`.
    run: record::RunState,
    /// Owners whose runs were sealed by the current `record_into`.
    finished_owners: Vec<NodeId>,
    /// Strict descendants that open layers. Recomputed at the start of each record.
    subtree_layers: HashMap<NodeId, bool>,
    /// Strict descendants that own a picture, from a list row or a canvas.
    subtree_owners: HashMap<NodeId, bool>,
    /// Parent layer while `record` is linking the layer tree.
    layer_stack: Vec<protocol::LayerId>,
    /// Last sibling linked under each layer, indexed by [`protocol::LayerId`].
    layer_tail: Vec<Option<protocol::LayerId>>,
    /// Last layer linked with no parent. Its `next` is the following top-level layer.
    layer_top: Option<protocol::LayerId>,
    /// Commands written into pictures by the latest `record`.
    picture_writes: usize,
    /// Bumps when a picture is rebuilt or a layer property changes.
    layer_epoch: u64,
    layers: crate::layer::LayerTree,
    /// Last semantics nodes. A clean node reuses its strings.
    semantics_cache: HashMap<crate::SemanticsId, crate::SemanticsNode>,
    /// Image sources still on the tree. Maintained on insert and removal.
    image_counts: HashMap<Arc<str>, u32>,
    image_generation: u64,
    /// Offset notices produced by [`Tree::tick_motions`], drained by the session.
    scroll_notices: Vec<Gesture>,
    /// Style or structure must run Taffy again.
    needs_layout: bool,
    /// Paint changed. Layout boxes stay valid.
    needs_paint: bool,
    /// How many times `layout` entered Taffy. Paint-only updates leave this unchanged.
    layout_passes: u64,
    /// Text nodes whose paint key must be retained before the next record.
    pending_text: Vec<NodeId>,
    sync_visited: u32,
    sync_synced: u32,
    sync_ms: f64,
    taffy_ms: f64,
    /// Sum of `layout` bodies since the last [`Tree::take_layout_trace`].
    layout_work_ms: f64,
    /// `layout` calls that entered Taffy since the last trace.
    layout_entries: u32,
    /// First reason `needs_layout` became true since the last trace.
    layout_cause: LayoutCause,
    text_reused: Cell<u32>,
    text_shaped: Cell<u32>,
    /// `layout_passes` already published by [`Tree::semantics_patch`].
    semantics_layout_seen: u64,
    /// Scroll nodes whose offset changed since the last full semantics walk.
    scroll_semantics: Vec<NodeId>,
    /// Hint or structure changed. The next patch walks the whole tree.
    semantics_full: bool,
    /// Reused by [`Tree::sweep_subtree_layers`].
    sweep_stack: Vec<record::SweepFrame>,
    sweep_children: Vec<NodeId>,
    /// Multiplier applied when measuring and recording text. `1.0` leaves sizes alone.
    text_scale: f32,
    pixel_scale: f32,
    laid_out: bool,
    /// Content origin in scene points. Insets shift the root; `(0, 0)` is the surface origin.
    origin: (f32, f32),
    viewport: Option<(f32, f32)>,
    /// Full window size in scene points. Zero until the session applies metrics.
    window: (f32, f32),
    /// Always-on safe area. Keyboard insets stay out of this rect.
    safe: (f32, f32, f32, f32),
    /// Scroll nodes whose pull threshold committed since the last take.
    refresh_pending: Vec<NodeId>,
    /// Developer diagnostics. Off by default; see `tree/debug.rs`.
    debug: debug::DebugState,
    style_nodes: HashMap<u32, style::NodeStyle>,
    style_queue: Vec<NodeId>,
    style_source_owners: HashMap<crate::style::StyleSourceId, NodeId>,
    next_style_source: u32,
    style_applying: bool,
    style_env: crate::style::Environment,
    class_cache: crate::style::ClassCache,
    style_gestures: Vec<Gesture>,
}

#[cfg(test)]
mod slot_tests {
    use super::{MAX_GENERATION, Tree};
    use crate::{Error, MonoShaper};

    #[test]
    fn recycled_slot_rejects_the_previous_id() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        let stale = text;
        tree.retire(text.index);
        let reused = tree.text("Yo", 16.0, [0, 0, 0, 255])?;
        assert_eq!(reused.index, stale.index);
        assert!(matches!(
            tree.set_text(stale, "no"),
            Err(Error::UnknownNode)
        ));
        tree.set_text(reused, "Ok")?;
        Ok(())
    }

    #[test]
    fn removed_child_can_be_inserted_until_discard_detached() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.column(0.0)?;
        let child = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        tree.add_child(parent, child)?;
        tree.remove(parent, child)?;
        tree.set_text(child, "still")?;
        tree.add_child(parent, child)?;
        assert_eq!(tree.parent(child)?, Some(parent));
        assert!(tree.discard_detached().ids.is_empty());
        let anchor = tree.text("A", 16.0, [0, 0, 0, 255])?;
        let extra = tree.text("B", 16.0, [0, 0, 0, 255])?;
        tree.add_child(parent, anchor)?;
        tree.remove(parent, anchor)?;
        tree.insert(parent, extra, Some(anchor))?;
        assert_eq!(tree.parent(extra)?, Some(parent));
        tree.remove(parent, child)?;
        tree.remove(parent, child)?;
        let retired = tree.discard_detached();
        assert!(retired.ids.contains(&child));
        assert!(matches!(tree.parent(child), Err(Error::UnknownNode)));
        Ok(())
    }

    #[test]
    fn remove_keeps_taffy_and_the_metrics() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.column(0.0)?;
        let child = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        tree.add_child(parent, child)?;
        tree.layout()?;
        tree.remove(parent, child)?;
        let node = tree.node(child)?;
        assert!(node.taffy.is_some());
        assert!(node.text().and_then(|text| text.metrics.as_ref()).is_some());
        assert!(tree.detach(parent, child)?.is_empty());
        assert!(tree.pending_discard.is_empty());
        assert!(tree.node(child)?.taffy.is_some());
        let retired = tree.discard(child)?;
        assert!(retired.ids.contains(&child));
        assert!(matches!(tree.node(child), Err(Error::UnknownNode)));
        Ok(())
    }

    #[test]
    fn generation_cap_retires_the_slot() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        let index = text.index;
        tree.slots[index as usize].generation = MAX_GENERATION;
        tree.retire(index);
        assert!(tree.free.is_empty());
        let next = tree.text("Yo", 16.0, [0, 0, 0, 255])?;
        assert_ne!(next.index, index);
        Ok(())
    }

    #[test]
    fn revive_rebuilds_taffy_after_release() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.column(0.0)?;
        let child = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        tree.add_child(parent, child)?;
        tree.layout()?;
        tree.detach(parent, child)?;
        tree.release_taffy(child)?;
        assert!(tree.node(child)?.taffy.is_none());
        tree.add_child(parent, child)?;
        assert!(tree.node(child)?.taffy.is_some());
        tree.layout()?;
        assert!(tree.rect(child)?.w > 0.0);
        Ok(())
    }
}

#[cfg(test)]
mod canvas_tests {
    use std::sync::Arc;

    use super::node::NodeFlags;
    use super::{NodeId, Tree};
    use crate::{Error, MonoShaper};

    fn near(actual: Option<(f32, f32)>, expected: (f32, f32)) -> bool {
        actual.is_some_and(|(x, y)| {
            (x - expected.0).abs() < 1.0e-3 && (y - expected.1).abs() < 1.0e-3
        })
    }

    fn canvas_with(sources: &[&str]) -> protocol::CanvasList {
        let images = sources
            .iter()
            .map(|source| protocol::CanvasImageRef {
                source: Arc::<str>::from(*source),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        protocol::CanvasList {
            body: Arc::new(protocol::CanvasBody {
                ops: Box::new([]),
                images,
                isolation: protocol::CanvasIsolation::Shared,
                bytes: 0,
            }),
        }
    }

    #[test]
    fn canvas_picture_bit_stays_independent_of_list_rows() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.column(0.0)?;
        let row = tree.sized(40.0, 24.0)?;
        tree.add_child(parent, row)?;
        tree.layout()?;
        let passes = tree.layout_passes();
        tree.node_mut(parent)?
            .flags
            .remove(NodeFlags::REPAINT_DIRTY);
        tree.node_mut(row)?.flags.remove(NodeFlags::REPAINT_DIRTY);

        tree.set_canvas(row, Some(protocol::CanvasList::empty()))?;
        assert!(
            tree.node(row)?
                .flags
                .contains(NodeFlags::PICTURE_FROM_CANVAS)
        );
        assert!(!tree.node(row)?.flags.contains(NodeFlags::PICTURE_OWNER));
        assert!(tree.is_picture_owner(row));
        assert!(tree.node(row)?.flags.contains(NodeFlags::REPAINT_DIRTY));
        assert!(tree.node(parent)?.flags.contains(NodeFlags::REPAINT_DIRTY));
        assert!(!tree.node(row)?.flags.contains(NodeFlags::LAYOUT_DIRTY));
        assert!(!tree.needs_layout());
        assert_eq!(tree.layout_passes(), passes);
        tree.record()?;
        assert!(tree.pictures.contains_key(&row));
        assert_eq!(tree.subtree_owners.get(&parent).copied(), Some(true));

        // Replacing the list does not clear the list-row bit or dirty the parent
        // again: this node was already a picture owner.
        tree.set_picture_owner(row, true)?;
        tree.node_mut(parent)?
            .flags
            .remove(NodeFlags::REPAINT_DIRTY);
        tree.set_canvas(row, Some(protocol::CanvasList::empty()))?;
        assert!(tree.node(row)?.flags.contains(NodeFlags::PICTURE_OWNER));
        assert!(
            tree.node(row)?
                .flags
                .contains(NodeFlags::PICTURE_FROM_CANVAS)
        );
        assert!(!tree.node(parent)?.flags.contains(NodeFlags::REPAINT_DIRTY));
        assert!(!tree.needs_layout());

        tree.set_picture_owner(row, false)?;
        assert!(!tree.node(row)?.flags.contains(NodeFlags::PICTURE_OWNER));
        assert!(
            tree.node(row)?
                .flags
                .contains(NodeFlags::PICTURE_FROM_CANVAS)
        );
        assert!(tree.node(row)?.canvas.is_some());
        assert!(tree.is_picture_owner(row));

        tree.set_picture_owner(row, true)?;
        tree.set_canvas(row, None)?;
        assert!(tree.node(row)?.flags.contains(NodeFlags::PICTURE_OWNER));
        assert!(
            !tree
                .node(row)?
                .flags
                .contains(NodeFlags::PICTURE_FROM_CANVAS)
        );
        assert!(tree.node(row)?.canvas.is_none());
        assert!(tree.is_picture_owner(row));
        tree.record()?;
        assert!(tree.pictures.contains_key(&row));

        tree.set_picture_owner(row, false)?;
        assert!(!tree.is_picture_owner(row));
        tree.set_canvas(row, Some(protocol::CanvasList::empty()))?;
        tree.set_picture_owner(row, false)?;
        assert!(tree.is_picture_owner(row));
        assert!(tree.node(row)?.canvas.is_some());
        Ok(())
    }

    #[test]
    fn canvas_owner_paints_after_its_parent_background() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.sized(100.0, 80.0)?;
        tree.node_mut(parent)?.write_paint(|paint| {
            paint.fill = Some([10, 20, 30, 255]);
        });
        let canvas = tree.sized(80.0, 40.0)?;
        tree.add_child(parent, canvas)?;
        tree.layout()?;
        tree.set_canvas(canvas, Some(protocol::CanvasList::empty()))?;
        tree.record()?;

        let layers = &tree.layers;
        let root = &layers.layers[layers.root.0 as usize];
        let crate::layer::LayerKind::Picture { picture } = root.kind else {
            panic!("parent background should be the first picture");
        };
        let background = layers.picture(picture).expect("background picture");
        assert!(matches!(background.commands.first(), Some(protocol::Command::Rect {
            color: [10, 20, 30, 255], ..
        })));
        let owner = &layers.layers[root.next.expect("canvas offset").0 as usize];
        let child = &layers.layers[owner.first_child.expect("canvas picture").0 as usize];
        let crate::layer::LayerKind::Picture { picture } = child.kind else {
            panic!("canvas owner should contain its picture");
        };
        let canvas = layers.picture(picture).expect("canvas picture");
        assert!(matches!(canvas.commands.first(), Some(protocol::Command::Canvas { .. })));
        Ok(())
    }

    #[test]
    fn canvas_command_sits_in_the_content_box() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let host = tree.sized(100.0, 80.0)?;
        tree.set_padding(host, 10.0)?;
        tree.node_mut(host)?.write_paint(|paint| {
            paint.fill = Some([10, 20, 30, 255]);
            paint.outline_width = 2.0;
            paint.outline_color = [1, 2, 3, 255];
        });
        let child = tree.sized(8.0, 8.0)?;
        tree.node_mut(child)?
            .write_paint(|paint| paint.fill = Some([9, 9, 9, 255]));
        tree.add_child(host, child)?;
        tree.layout()?;
        tree.set_canvas(host, Some(protocol::CanvasList::empty()))?;
        assert!(!tree.needs_layout());

        let stored_x = tree.node(host)?.x + tree.node(host)?.content_x;
        let stored_y = tree.node(host)?.y + tree.node(host)?.content_y;
        let content_w = tree.node(host)?.content_w;
        let content_h = tree.node(host)?.content_h;
        assert!(tree.node(host)?.content_x > 0.0 && tree.node(host)?.content_y > 0.0);
        assert!(content_w > 0.0 && content_h > 0.0);

        let list = tree.record()?;
        let canvas_at = list.commands.iter().position(|command| {
            matches!(
                command,
                protocol::Command::Canvas { x, y, width, height, .. }
                    if *x == stored_x && *y == stored_y && *width == content_w && *height == content_h
            )
        });
        let Some(canvas_at) = canvas_at else {
            panic!("missing canvas at the content box");
        };
        let fill_at = list.commands.iter().position(|command| {
            matches!(
                command,
                protocol::Command::Rect {
                    color: [10, 20, 30, 255],
                    ..
                }
            )
        });
        let outline_at = list.commands.iter().position(|command| {
            matches!(command, protocol::Command::Rect { stroke_width, .. } if *stroke_width > 0.0)
        });
        let child_at = list.commands.iter().position(|command| {
            matches!(
                command,
                protocol::Command::Rect {
                    color: [9, 9, 9, 255],
                    ..
                }
            )
        });
        assert!(fill_at.is_some_and(|index| index < canvas_at));
        assert!(outline_at.is_some_and(|index| index < canvas_at));
        assert!(child_at.is_some_and(|index| index > canvas_at));

        let mut empty = Tree::new(MonoShaper::default());
        let blank = empty.column(0.0)?;
        empty.layout()?;
        assert_eq!(empty.node(blank)?.content_w, 0.0);
        assert_eq!(empty.node(blank)?.content_h, 0.0);
        empty.set_canvas(blank, Some(protocol::CanvasList::empty()))?;
        let hidden = empty.record()?;
        assert!(
            hidden
                .commands
                .iter()
                .all(|command| !matches!(command, protocol::Command::Canvas { .. }))
        );
        Ok(())
    }

    #[test]
    fn canvas_sources_join_the_census_until_cleared_or_retired() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let host = tree.sized(32.0, 32.0)?;
        let photo = tree.image("asset:photo", 8.0, 8.0)?;
        tree.add_child(host, photo)?;
        tree.set_canvas(host, Some(canvas_with(&["asset:photo", "asset:mark"])))?;
        let counts = tree.image_census();
        assert_eq!(counts.counts.get("asset:photo").copied(), Some(2));
        assert_eq!(counts.counts.get("asset:mark").copied(), Some(1));

        tree.set_canvas(host, Some(canvas_with(&["asset:mark"])))?;
        let counts = tree.image_census();
        assert_eq!(counts.counts.get("asset:photo").copied(), Some(1));
        assert_eq!(counts.counts.get("asset:mark").copied(), Some(1));

        tree.set_canvas(host, None)?;
        let counts = tree.image_census();
        assert_eq!(counts.counts.get("asset:photo").copied(), Some(1));
        assert!(counts.counts.get("asset:mark").is_none());

        tree.set_canvas(host, Some(canvas_with(&["asset:gone"])))?;
        assert_eq!(
            tree.image_census().counts.get("asset:gone").copied(),
            Some(1)
        );
        tree.discard(host)?;
        assert!(tree.image_census().counts.is_empty());
        Ok(())
    }

    #[test]
    fn canvas_local_undoes_scroll_and_the_node_transform() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        tree.set_viewport(0.0, 0.0, 100.0, 80.0)?;
        let scroll = tree.scroll()?;
        tree.set_max_size(scroll, 80.0, 60.0)?;
        let outer = tree.transform()?;
        tree.set_transform(outer, 4.0, 5.0, 0.0, 1.0)?;
        let node = tree.transform()?;
        tree.set_size(node, 200.0, 240.0)?;
        tree.set_padding(node, 6.0)?;
        tree.add_child(outer, node)?;
        tree.add_child(scroll, outer)?;
        tree.layout()?;
        tree.set_transform(node, 12.0, 18.0, 0.0, 1.0)?;
        tree.set_scroll_offset(scroll, 30.0, 40.0)?;

        let (bx, by) = tree.layout_point(node)?;
        let content_x = tree.node(node)?.content_x;
        let content_y = tree.node(node)?.content_y;
        let (scroll_x, scroll_y) = tree.node(scroll)?.scroll_paint();
        assert!(content_x > 0.0 && content_y > 0.0);
        assert_ne!((scroll_x, scroll_y), (0.0, 0.0));
        let cx = 8.0;
        let cy = 5.0;
        let px = bx + content_x + cx;
        let py = by + content_y + cy;
        let scene_x = px - scroll_x + 12.0 + 4.0;
        let scene_y = py - scroll_y + 18.0 + 5.0;
        assert!(near(tree.canvas_local(node, scene_x, scene_y), (cx, cy)));
        assert_ne!(tree.scene_to_layout(node, scene_x, scene_y), Some((px, py)));

        tree.set_transform(node, 12.0, 18.0, std::f32::consts::FRAC_PI_4, 1.5)?;
        let scene = tree.layout_to_scene(node, px, py);
        assert!(scene.is_some());
        if let Some((sx, sy)) = scene {
            assert!(near(tree.canvas_local(node, sx, sy), (cx, cy)));
        }

        tree.set_transform(node, 1.0, 1.0, 0.0, 0.0)?;
        assert!(tree.canvas_local(node, scene_x, scene_y).is_none());

        tree.set_transform(node, 12.0, 18.0, 0.0, 1.0)?;
        tree.node_mut(node)?.flags.insert(NodeFlags::HIDDEN);
        assert!(tree.canvas_local(node, scene_x, scene_y).is_none());
        assert!(
            tree.canvas_local(
                NodeId {
                    index: 9,
                    generation: 3
                },
                0.0,
                0.0
            )
            .is_none()
        );
        Ok(())
    }

    #[test]
    fn canvas_surface_uses_the_content_box_and_inherited_direction() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.sized(100.0, 80.0)?;
        tree.node_mut(parent)?.style.direction = taffy::style::Direction::Rtl;
        let child = tree.sized(40.0, 30.0)?;
        tree.set_padding(child, 4.0)?;
        tree.add_child(parent, child)?;
        tree.layout()?;
        let Some(surface) = tree.canvas_surface(child) else {
            panic!("missing surface");
        };
        assert!(!surface.hidden);
        assert!(surface.rtl);
        assert_eq!(surface.width, tree.node(child)?.content_w);
        assert_eq!(surface.height, tree.node(child)?.content_h);
        assert!(surface.width < 40.0 && surface.height < 30.0);

        tree.node_mut(parent)?.style.direction = taffy::style::Direction::Ltr;
        let Some(ltr) = tree.canvas_surface(child) else {
            panic!("missing ltr surface");
        };
        assert!(!ltr.rtl);
        tree.node_mut(parent)?.style.direction = taffy::style::Direction::Rtl;

        tree.node_mut(parent)?.flags.insert(NodeFlags::HIDDEN);
        let Some(hidden) = tree.canvas_surface(child) else {
            panic!("missing hidden surface");
        };
        assert!(hidden.hidden);
        assert!(
            tree.canvas_surface(NodeId {
                index: 9,
                generation: 3
            })
            .is_none()
        );
        Ok(())
    }

    #[test]
    fn canvas_text_metrics_ignore_text_scale() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        tree.set_text_scale(2.0)?;
        let metrics =
            tree.canvas_text_metrics("ab", &crate::canvas::CanvasFont::default(), false)?;
        assert!((metrics.width - 10.0).abs() < 0.01);
        assert!((metrics.font_bounding_box_ascent - 8.0).abs() < 0.01);
        assert!((metrics.font_bounding_box_descent - 2.0).abs() < 0.01);
        let empty = tree.canvas_text_metrics("", &crate::canvas::CanvasFont::default(), false)?;
        assert_eq!(empty.width, 0.0);
        assert!((empty.font_bounding_box_ascent - 8.0).abs() < 0.01);
        Ok(())
    }
}
