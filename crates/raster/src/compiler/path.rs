//! Triangulates Canvas fill geometry during picture compilation.

use engine::scene::CanvasPath;

use super::Tessellation;
use crate::batch::{lyon_fill_rule, tessellation_path};
use crate::status::Status;

pub(super) struct PreparedPath {
    pub(super) buffers: Tessellation,
    pub(super) fringe: Option<Box<[[f32; 2]]>>,
}

pub(super) fn prepare_path(
    path: &CanvasPath,
    tolerance: f32,
    direct: bool,
) -> Result<PreparedPath, Status> {
    Ok(PreparedPath {
        buffers: tessellate(path, tolerance)?,
        fringe: direct
            .then(|| crate::fringe::convex(path, tolerance))
            .flatten(),
    })
}

/// Tessellates a path in canvas points into position+coverage vertices.
pub fn tessellate(
    canvas_path: &CanvasPath,
    tolerance: f32,
) -> Result<lyon_tessellation::VertexBuffers<[f32; 3], u32>, Status> {
    let mut tessellator = lyon_tessellation::FillTessellator::new();
    let mut buffers: lyon_tessellation::VertexBuffers<[f32; 3], u32> =
        lyon_tessellation::VertexBuffers::new();
    let path = tessellation_path(&canvas_path.verbs);
    let result = tessellator.tessellate_path(
        &path,
        &lyon_tessellation::FillOptions::tolerance(tolerance)
            .with_fill_rule(lyon_fill_rule(canvas_path.fill)),
        &mut lyon_tessellation::BuffersBuilder::new(
            &mut buffers,
            |vertex: lyon_tessellation::FillVertex| {
                let position = vertex.position();
                [position.x, position.y, 1.0]
            },
        ),
    );
    result.map_err(|error| {
        tracing::warn!(target: "impasto::canvas", %error, "path tessellation failed");
        Status::BadScene
    })?;
    Ok(buffers)
}

pub fn mesh_bounds(vertices: &[[f32; 3]]) -> [f32; 4] {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for vertex in vertices {
        min[0] = min[0].min(vertex[0]);
        min[1] = min[1].min(vertex[1]);
        max[0] = max[0].max(vertex[0]);
        max[1] = max[1].max(vertex[1]);
    }
    if min[0].is_infinite() {
        return [0.0; 4];
    }
    [min[0], min[1], max[0] - min[0], max[1] - min[1]]
}
