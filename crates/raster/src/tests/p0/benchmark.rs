use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{ImagePixels, LayerTree, PixelSize, output};
use crate::{FontLibrary, RasterBackend, RasterOptions, Rasterizer, ValidationMode};

pub(super) struct Headless {
    raster: Rasterizer,
    init_us: f64,
}

pub(super) fn headless(fonts: FontLibrary) -> Headless {
    let backend = match std::env::var("IMPASTO_BENCH_BACKEND").as_deref() {
        Ok("metal") => Some(RasterBackend::Metal),
        Ok("vulkan") => Some(RasterBackend::Vulkan),
        Ok("gles") => Some(RasterBackend::Gles),
        Ok("dx12") => Some(RasterBackend::Dx12),
        Ok("auto") | Err(std::env::VarError::NotPresent) => None,
        other => panic!("invalid benchmark backend: {other:?}"),
    };
    let started = Instant::now();
    let raster = Rasterizer::headless(
        RasterOptions {
            enable_gpu_timing: std::env::var("IMPASTO_BENCH_GPU").as_deref() != Ok("0"),
            validation: ValidationMode::AsyncOnly,
            backend,
            ..RasterOptions::default()
        },
        fonts,
    )
    .unwrap();
    Headless {
        raster,
        init_us: started.elapsed().as_secs_f64() * 1_000_000.0,
    }
}

pub(super) fn run(
    host: &mut Headless,
    name: &str,
    frames: u32,
    size: PixelSize,
    scene: impl FnMut(u32) -> LayerTree,
) {
    run_with_images(
        host,
        name,
        frames,
        size,
        scene,
        ImagePixels {
            width: 2,
            height: 2,
            rgba: &[
                230, 110, 50, 255, 90, 120, 180, 255, 88, 90, 124, 255, 70, 60, 200, 255,
            ],
        },
    );
}

pub(super) fn run_with_images(
    host: &mut Headless,
    name: &str,
    frames: u32,
    size: PixelSize,
    scene: impl FnMut(u32) -> LayerTree,
    image: ImagePixels<'_>,
) {
    run_in_format(
        host,
        name,
        frames,
        size,
        scene,
        image,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );
}

pub(super) fn run_in_format(
    host: &mut Headless,
    name: &str,
    frames: u32,
    size: PixelSize,
    mut scene: impl FnMut(u32) -> LayerTree,
    image: ImagePixels<'_>,
    format: wgpu::TextureFormat,
) {
    let raster = &mut host.raster;
    let gpu = raster.device.as_ref().unwrap().gpu.clone();
    let target = output(&gpu, format, size);
    let view = target.create_view(&Default::default());
    let mut samples = Vec::new();
    let mut metrics: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for frame in 0..frames {
        let frame_id = u64::from(frame) + 1;
        let frame_scene = scene(frame);
        raster.begin_gpu_frame(raster.options.enable_gpu_timing.then_some(frame_id));
        let prepare = Instant::now();
        raster
            .submit_layers(&frame_scene, 1.0, |_| {
                Some(ImagePixels {
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba,
                })
            })
            .unwrap();
        let scene_install = prepare.elapsed();
        raster.encode_and_submit(&view, format, size).unwrap();
        let stats = raster.submit_stats();
        // Use a steady single in-flight submission. The wait and readback are
        // outside all CPU metrics, and each sample has the same queue depth.
        wait_for_gpu(&gpu);
        let output_ready = prepare.elapsed();
        let mut readback = raster.drain_gpu_timings().pop();
        if raster.gpu_timings_pending() {
            // Metal queues the resolve after the rendered frame completes.
            wait_for_gpu(&gpu);
            readback = raster.drain_gpu_timings().pop();
        }
        assert!(!raster.gpu_timings_pending());
        let status = readback
            .as_ref()
            .map_or(raster.gpu_sample().status, |u| u.sample.status);
        let timing = readback.and_then(|u| {
            assert_eq!(u.sample.frame_id, frame_id);
            u.timing
        });
        let times = [
            ("picture_prepare_us", stats.picture_prepare),
            ("compose_us", stats.compose),
            ("encode_us", stats.encode),
            ("flush_us", stats.flush),
            ("queue_submit_us", stats.queue_submit),
            ("scene_install_us", scene_install),
            ("output_ready_us", output_ready),
        ]
        .map(|(name, time)| (name, time.as_secs_f64() * 1_000_000.0));
        let mut by_kind = BTreeMap::new();
        let gpu_passes =
            timing.as_ref().map(|gpu| {
                gpu.passes.iter().map(|pass| {
                let kind = format!("{:?}", pass.kind);
                *by_kind.entry(kind.clone()).or_insert(0.0) += pass.duration_us;
                json!({ "kind": kind, "draws": pass.draws, "duration_us": pass.duration_us })
            }).collect::<Vec<_>>()
            });
        // The first frame is the cold sample. Report its counters and timing
        // but exclude it from the steady-state distribution.
        if frame > 0 {
            for (name, value) in times {
                metrics.entry(name.into()).or_default().push(value);
            }
            if let Some(gpu) = &timing {
                for (name, value) in [
                    ("gpu_frame_span_us", gpu.frame_span_us),
                    ("gpu_passes_sum_us", gpu.passes_sum_us),
                ] {
                    metrics.entry(name.into()).or_default().push(value);
                }
                for (kind, value) in by_kind {
                    metrics
                        .entry(format!("gpu_{kind}_sum_us"))
                        .or_default()
                        .push(value);
                }
            }
        }
        let r = stats.render;
        let counters = json!({
                "picture_workers": stats.picture_workers,
                "prerasterized_glyphs": stats.atlas.prerasterized,
                "atlas_misses": stats.atlas.misses,
                "atlas_upload_calls": stats.atlas.upload_calls,
                "texture_entries": stats.textures,
                "atlas_region_reclaims": stats.atlas.region_reclaims,
                "backdrop_cache_hits": r.backdrop_cache_hits,
                "transition_snapshot_hits": r.transition_snapshot_hits,
                "transition_snapshot_bakes": r.transition_snapshot_bakes,
                "occluded_siblings": r.occluded_siblings,
                "subtree_cache_hits": r.subtree_cache.hits,
                "subtree_cache_bytes": r.subtree_cache.bytes,
                "damage_pct": stats.damage.fraction(size) * 100.0,
                "partial_redraw": r.partial_redraw,
                "draws": r.draws, "passes": r.passes, "offscreen_passes": r.offscreen_passes,
                "compute_passes": r.compute_passes, "direct_present": r.direct_present,
                "buffers_created": r.buffers_created, "bind_groups_created": r.bind_groups_created,
                "vertex_bytes": r.vertex_bytes, "uniform_bytes": r.uniform_bytes,
                "stop_bytes": r.stop_bytes, "extra_bytes": r.extra_bytes, "mesh_upload_bytes": r.mesh_upload_bytes,
                "atlas_upload_bytes": r.atlas_upload_bytes, "gradient_lut_bytes": r.gradient_lut_bytes,
                "gradient_lut_hits": r.gradient_lut_hits, "copy_pixels": r.copy_pixels,
                "blur_pixels": r.blur_pixels, "offscreen_alloc_bytes": r.offscreen_alloc_bytes,
                "offscreen_peak_bytes": r.offscreen_peak_bytes, "pooled_texture_bytes": r.pooled_texture_bytes,
                "stencil_draws": r.stencil_draws, "early_z_instances": r.early_z_instances,
                "transient_bytes": r.transient_bytes,
        });
        samples.push(json!({
            "frame": frame_id,
            "cpu_us": times.into_iter().collect::<BTreeMap<_, _>>(),
            "gpu_status": status.name(),
            "gpu_passes": gpu_passes,
            "gpu_frame_span_us": timing.as_ref().map(|gpu| gpu.frame_span_us),
            "gpu_passes_sum_us": timing.as_ref().map(|gpu| gpu.passes_sum_us),
            "image_upload": {
                "source_bytes": stats.image_upload.source_bytes, "temporary_bytes": stats.image_upload.temporary_bytes,
                "resident_bytes": stats.image_upload.resident_bytes, "render_passes": stats.image_upload.render_passes,
                "compute_passes": stats.image_upload.compute_passes,
            },
            "counters": counters,
        }));
    }
    let percentiles = metrics
        .into_iter()
        .map(|(name, mut samples)| {
            samples.sort_by(f64::total_cmp);
            let percentile = |p: usize| samples[(samples.len() * p).div_ceil(100) - 1];
            (
                name,
                json!({ "samples": samples.len(), "p50": percentile(50), "p95": percentile(95) }),
            )
        })
        .collect::<BTreeMap<String, Value>>();
    let info = gpu.adapter.get_info();
    let report = json!({
        "schema": 1,
        "scene": name,
        "viewport": [size.width, size.height],
        "output_format": format!("{format:?}"),
        "dpr": 1,
        "environment": {
            "git_sha": std::env::var("IMPASTO_BENCH_GIT_SHA").unwrap_or_else(|_| {
                let output = std::process::Command::new("git").args(["rev-parse", "HEAD"]).output();
                output.ok().filter(|out| out.status.success()).map_or_else(
                    || "unknown".into(),
                    |out| String::from_utf8_lossy(&out.stdout).trim().to_owned(),
                )
            }),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "adapter": info.name,
            "driver": info.driver,
            "driver_info": info.driver_info,
            "backend": format!("{:?}", info.backend),
            "gpu_tier": format!("{:?}", gpu.caps.tier),
            "adapter_timestamp_query": gpu.caps.adapter_features.contains(wgpu::Features::TIMESTAMP_QUERY),
            "device_timestamp_query": gpu.caps.has_timestamp_query,
            "gpu_timing_requested": raster.options.enable_gpu_timing,
            "validation_mode": "AsyncOnly",
            "debug_assertions": cfg!(debug_assertions),
            "in_flight": 1,
        },
        "steady_state_us": percentiles,
        "raster_init_us": host.init_us,
        "samples": samples,
    });
    println!("IMPASTO_BENCH_JSON {report}");
}

pub(super) fn wait_for_gpu(gpu: &crate::Gpu) {
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
}
