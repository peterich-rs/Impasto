//! CPU jobs shared by picture compilation and glyph rasterization.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use engine::scene::{CanvasOp, Command, Picture, PictureId};
use rayon::ThreadPool;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};

use crate::Status;
use crate::compiler::{self, PreparedPicture};
use crate::textures::Load;

const PICTURE_THRESHOLD: usize = 4;
const PATH_THRESHOLD: usize = 64;

/// A process-wide pool bounds contention when several windows prepare scenes.
pub(crate) fn pool() -> Option<&'static ThreadPool> {
    static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism().ok()?.get().min(6);
        if threads < 2 {
            return None;
        }
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("impasto-prepare-{index}"))
            .build()
            .map_err(|error| {
                tracing::warn!(target: "impasto::compile", %error, "picture worker pool unavailable");
            })
            .ok()
    })
    .as_ref()
}

pub(crate) fn path_count(ops: &[CanvasOp]) -> usize {
    ops.iter()
        .filter(|op| {
            matches!(
                op,
                CanvasOp::Fill { .. } | CanvasOp::Vector { .. } | CanvasOp::Clip { .. }
            )
        })
        .count()
}

/// Compiles CPU-only pictures in parallel. Image resolution and all buffer
/// creation stay on the raster queue in the original picture order.
pub(crate) fn pictures<'a>(
    pending: impl Iterator<Item = (&'a Picture, f32)>,
    dimensions: impl Fn(&str) -> Option<(f32, f32)>,
    analytic_curves: bool,
) -> (
    Option<&'static ThreadPool>,
    HashMap<PictureId, Result<PreparedPicture, Status>>,
) {
    let mut jobs = Vec::new();
    let mut paths = 0;
    for (picture, scale) in pending {
        jobs.push((picture, scale));
        if jobs.len() < PICTURE_THRESHOLD && paths < PATH_THRESHOLD {
            paths += picture
                .commands()
                .iter()
                .filter_map(|command| match command {
                    Command::Canvas { list, .. } => Some(path_count(list.ops())),
                    _ => None,
                })
                .sum::<usize>();
        }
    }
    let rebuilt = jobs.len();
    if rebuilt < PICTURE_THRESHOLD && paths < PATH_THRESHOLD {
        return (None, HashMap::new());
    }
    let Some(pool) = pool() else {
        return (None, HashMap::new());
    };
    let span = tracing::debug_span!(
        target: "impasto::compile",
        "parallel_prepare",
        pictures_rebuilt = rebuilt,
        workers = pool.current_num_threads(),
        tess_us = tracing::field::Empty,
        glyph_pre_us = 0u64
    );
    let _entered = span.enter();
    let started = Instant::now();
    let eligible = jobs
        .into_iter()
        .filter_map(|(picture, scale)| {
            let sources: Option<Vec<_>> = picture
                .images()
                .iter()
                .map(|(source, _)| dimensions(source).map(|size| (Arc::clone(source), size)))
                .collect();
            Some((picture, scale, sources?))
        })
        .collect::<Vec<_>>();
    let prepared = pool.install(|| {
        eligible
            .into_par_iter()
            .map(|(picture, scale, sources)| {
                let prepared =
                    compiler::prepare(picture, scale, Some(pool), analytic_curves, &mut |source| {
                        sources.iter().find(|(known, _)| known == source).map_or(
                            Load::Failed,
                            |(_, (width, height))| Load::Ready {
                                width: *width,
                                height: *height,
                            },
                        )
                    });
                (picture.id, prepared)
            })
            .collect()
    });
    span.record(
        "tess_us",
        u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
    );
    (Some(pool), prepared)
}
