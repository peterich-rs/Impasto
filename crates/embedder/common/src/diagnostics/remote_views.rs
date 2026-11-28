use engine::scene::{LayerKind, LayerTree};
use engine::tree::{FrameRepaintBatch, NodeRepaintStats, PickedViewEvent};
use serde_json::{Value, json};

pub(crate) fn leaderboard(entries: &[NodeRepaintStats]) -> Value {
    Value::Array(entries.iter().map(|entry| json!({
        "node_raw": entry.node.to_raw().to_string(), "generation": entry.node.to_raw() >> 32,
        "total_repaints": entry.total_repaints, "total_layouts": entry.total_layouts,
        "fast_patch_count": entry.path_counts[0], "picture_patch_count": entry.path_counts[1],
        "full_rebuild_count": entry.path_counts[2], "canvas_redraw_count": entry.path_counts[3],
        "last_frame": entry.last_frame, "last_path": entry.last_path.map(|path| path.name()),
        "last_reason": entry.last_reason.bits(), "recent_mask": format!("{:016x}", entry.recent_mask),
        "recent_repaints": entry.recent_mask.count_ones(),
    })).collect())
}

pub(super) fn repaints(batch: &FrameRepaintBatch) -> Value {
    let entries: Vec<_> = batch.entries.iter().map(|entry| json!({
        "node_raw": entry.node.to_raw().to_string(), "kind": entry.kind,
        "rect": [entry.rect.x, entry.rect.y, entry.rect.w, entry.rect.h],
        "path": entry.path.name(), "reason": entry.reason.bits(),
        "caused_layout": entry.reason.contains(engine::tree::RepaintReason::LAYOUT_GEOMETRY),
        "picture_id": entry.picture_id.map(|id| id.0.to_string()), "command_count": entry.command_count,
        "source": entry.source.as_ref().map(|(component, path, line, column)| json!({
            "component": component, "path": path, "line": line, "column": column
        })),
    })).collect();
    json!({
        "frame_number": batch.frame_number,
        "total_repainted_views": batch.total_repainted_views, "overflow_count": batch.overflow_count,
        "fast_patch_views": batch.path_counts[0], "picture_patch_views": batch.path_counts[1],
        "full_rebuild_views": batch.path_counts[2], "canvas_redraw_views": batch.path_counts[3],
        "entries": entries,
    })
}

pub(super) fn picked(event: &PickedViewEvent) -> Value {
    json!({
        "node_id": event.chain.last().map(|node| node.to_raw().to_string()),
        "chain": event.chain.iter().map(|node| node.to_raw().to_string()).collect::<Vec<_>>(),
        "x": event.x, "y": event.y, "final": event.final_pick,
    })
}

pub(super) fn layer_tree(id: u64, tree: &LayerTree) -> Value {
    const LIMIT: usize = 20_000;
    let layers: Vec<_> = tree
        .layers
        .iter()
        .take(LIMIT)
        .enumerate()
        .map(|(id, layer)| {
            let (kind, picture) = match &layer.kind {
                LayerKind::Paint(_) => ("paint", None),
                LayerKind::Fill { .. } => ("fill", None),
                LayerKind::Offset { .. } => ("offset", None),
                LayerKind::Transform { .. } => ("transform", None),
                LayerKind::Clip { .. } | LayerKind::CornerClip { .. } => ("clip", None),
                LayerKind::Opacity { .. } => ("opacity", None),
                LayerKind::Blur { .. } => ("blur", None),
                LayerKind::ColorFilter { .. } => ("color_filter", None),
                LayerKind::BackdropBlur { .. } | LayerKind::BackdropMaterial { .. } => {
                    ("backdrop", None)
                }
                LayerKind::Blend { .. } => ("blend", None),
                LayerKind::Picture { picture } => ("picture", Some(picture.0.to_string())),
                LayerKind::PlatformView { .. } => ("platform_view", None),
            };
            json!({
                "id": id, "kind": kind, "picture": picture,
                "parent": layer.parent.map(|id| id.0),
                "first_child": layer.first_child.map(|id| id.0), "next": layer.next.map(|id| id.0),
            })
        })
        .collect();
    let pictures: Vec<_> = tree.pictures.iter().take(LIMIT).map(|picture| json!({
        "id": picture.id.0.to_string(), "generation": picture.generation,
        "origin": picture.origin, "commands": picture.commands().len(), "images": picture.images().len(),
    })).collect();
    json!({
        "frame_id": id, "epoch": tree.epoch, "root": tree.root.0, "layers": layers, "pictures": pictures,
        "truncated": tree.layers.len() > LIMIT || tree.pictures.len() > LIMIT,
    })
}
