//! Working target selection, color conversion, and late surface acquisition.

use engine::scene::LayerId;

use super::layers::{opaque_tail, scene_reads_root};
use super::{
    Compositor, Coverage, DeferredRootPass, Frame, FullPass, Image, PassInputs, Scene,
    frame_encoder,
};
use crate::Status;
use crate::compiler::{Clip, IDENTITY};
use crate::gpu::Gpu;
use crate::profiler::Capture;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputFormat {
    pub format: wgpu::TextureFormat,
    space: wgpu::SurfaceColorSpace,
    straight_alpha: bool,
}

impl OutputFormat {
    fn working(self) -> wgpu::TextureFormat {
        match self.space {
            wgpu::SurfaceColorSpace::DisplayP3 | wgpu::SurfaceColorSpace::ExtendedSrgbLinear => {
                wgpu::TextureFormat::Rgba16Float
            }
            _ => wgpu::TextureFormat::Rgba8UnormSrgb,
        }
    }

    pub fn surface(config: &wgpu::SurfaceConfiguration) -> Self {
        Self {
            format: config.format,
            space: config.color_space,
            straight_alpha: config.alpha_mode == wgpu::CompositeAlphaMode::PostMultiplied,
        }
    }
}

impl From<wgpu::TextureFormat> for OutputFormat {
    fn from(format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            straight_alpha: false,
            space: if format == wgpu::TextureFormat::Rgba16Float {
                wgpu::SurfaceColorSpace::ExtendedSrgbLinear
            } else {
                wgpu::SurfaceColorSpace::Srgb
            },
        }
    }
}

/// Encoded offscreen work with ordered points that still need the drawable.
pub(crate) struct PreparedOutput {
    root: Option<DeferredRootPass>,
    tail: wgpu::CommandEncoder,
}

impl PreparedOutput {
    pub fn finish(
        mut self,
        output: &wgpu::TextureView,
        compositor: &mut Compositor,
    ) -> wgpu::CommandEncoder {
        if let Some(mut root) = self.root.take() {
            root.encode(&mut self.tail, output, &compositor.pipelines);
            root.list.clear();
            compositor.draw_lists.push(root.list);
        }
        self.tail
    }
}

struct OutputPlan {
    conversion: Option<FullPass>,
    main_format: wgpu::TextureFormat,
}

impl OutputPlan {
    fn select(
        scene: &Scene<'_>,
        output: OutputFormat,
        visible_root: LayerId,
        working: wgpu::TextureFormat,
    ) -> Self {
        let OutputFormat {
            format,
            space,
            straight_alpha,
            ..
        } = output;
        let p3 = space == wgpu::SurfaceColorSpace::DisplayP3;
        let linear = space == wgpu::SurfaceColorSpace::ExtendedSrgbLinear;
        let direct_candidate =
            !straight_alpha && (linear && format == working || !p3 && format.is_srgb());
        let reads_root = direct_candidate && scene_reads_root(scene, Some(visible_root));
        let direct = direct_candidate && !reads_root;
        let conversion = (!direct).then_some(if p3 {
            FullPass::DisplayP3
        } else if format.is_srgb() || linear {
            FullPass::Copy
        } else {
            FullPass::EncodeSrgb
        });
        Self {
            conversion,
            main_format: working,
        }
    }
}

impl Compositor {
    pub fn encode(
        &mut self,
        gpu: &Gpu,
        scene: Scene<'_>,
        output: &wgpu::TextureView,
        output_format: impl Into<OutputFormat>,
        size: [u32; 2],
        capture: Option<(&mut Capture, Option<usize>)>,
    ) -> Result<wgpu::CommandEncoder, Status> {
        let prepared = self.prepare_output(
            gpu,
            scene,
            Some(output),
            output_format.into(),
            size,
            capture,
        )?;
        Ok(prepared.tail)
    }

    pub fn prepare_surface(
        &mut self,
        gpu: &Gpu,
        scene: Scene<'_>,
        output_format: OutputFormat,
        size: [u32; 2],
        capture: Option<&mut Capture>,
    ) -> Result<PreparedOutput, Status> {
        self.prepare_output(
            gpu,
            scene,
            None,
            output_format,
            size,
            capture.map(|capture| (capture, None)),
        )
    }

    fn prepare_output(
        &mut self,
        gpu: &Gpu,
        scene: Scene<'_>,
        output: Option<&wgpu::TextureView>,
        output_format: OutputFormat,
        size: [u32; 2],
        capture: Option<(&mut Capture, Option<usize>)>,
    ) -> Result<PreparedOutput, Status> {
        self.last_viewport = Some(size);
        self.pipelines.install_warmed();
        let working = output_format.working();
        let command = frame_encoder(gpu);
        let (visible_root, occluded) = opaque_tail(&scene, size);
        self.render_stats.occluded_siblings += occluded;
        let plan = OutputPlan::select(&scene, output_format, visible_root, working);
        let direct = plan.conversion.is_none();
        let first = self.render_stats.passes == 0;
        self.render_stats.direct_present = direct && (first || self.render_stats.direct_present);
        let late_acquire = output.is_none();
        let mut output = Image::output(output.unwrap_or(&self.white), output_format.format, size);
        if late_acquire {
            // The white view is a unique target identity while the window view
            // is unavailable. Deferred passes never attach or read that view.
            output.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        let mut frame = Frame {
            gpu,
            scene,
            compositor: self,
            command,
            pending: None,
            main_pass: None,
            main_inputs: PassInputs::default(),
            late_acquire,
            deferred_root: None,
            clears: HashMap::new(),
            root: output.view.clone(),
            main: output.view.clone(),
            capture,
            size,
            working,
            straight_alpha: output_format.straight_alpha,
        };
        let main = frame.main_target(&output, &plan);
        frame.main = main.view.clone();
        frame.replay_root(&main, visible_root)?;
        frame.compositor.finish_glyphs(frame.scene.fonts)?;
        frame.finish_root(main, &output, plan)?;
        Ok(PreparedOutput {
            root: frame.deferred_root,
            tail: frame.command,
        })
    }
}

impl Frame<'_> {
    fn main_target(&mut self, output: &Image, plan: &OutputPlan) -> Image {
        let size = self.size;
        let format = plan.main_format;
        if plan.conversion.is_none() {
            output.clone()
        } else {
            self.take(size, format)
        }
    }

    fn finish_root(&mut self, main: Image, output: &Image, plan: OutputPlan) -> Result<(), Status> {
        self.finish(&main.view);
        let Some(conversion) = plan.conversion else {
            return Ok(());
        };
        self.clear(&output.view);
        self.full(
            &main,
            None,
            &Coverage::default(),
            output,
            output.area(),
            conversion,
        )?;
        self.release(main);
        Ok(())
    }

    fn replay_root(&mut self, main: &Image, visible_root: LayerId) -> Result<(), Status> {
        self.clear(&main.view);
        if !self.scene.tree.layers.is_empty() {
            self.layers(Some(visible_root), main, IDENTITY, &Clip::default())?;
        }
        Ok(())
    }
}
