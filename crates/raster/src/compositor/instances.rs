//! Records instance runs in painter order.

use super::{DrawPipeline, Frame, Image, PassBinding, single};
use crate::{
    Status,
    arena::Extra,
    pipelines::{BlendKey, PipelineKind, ViewUniform},
};

impl Frame<'_> {
    #[expect(
        clippy::too_many_arguments,
        reason = "records typed instances and their shared draw state"
    )]
    pub(super) fn record_instances<T: bytemuck::Pod>(
        &mut self,
        target: &Image,
        view: ViewUniform,
        scissor: Option<[u32; 4]>,
        kind: PipelineKind,
        blend: BlendKey,
        extra: Extra,
        material: impl Into<PassBinding>,
        mask: Option<PassBinding>,
        vertices: &[T],
        vertex_count: u32,
    ) -> Result<(), Status> {
        if vertices.is_empty() {
            return Ok(());
        }
        let view = self.compositor.arena.view(self.gpu, &view)?;
        let globals = self.globals(view, extra);
        let pipeline =
            self.compositor
                .pipelines
                .get(self.gpu, single(target.format), kind, blend)?;
        self.record(
            target,
            DrawPipeline::Standard(pipeline),
            globals,
            material,
            mask,
            vertices,
            scissor,
            vertex_count,
        )
    }
}
