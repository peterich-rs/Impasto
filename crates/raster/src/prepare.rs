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
                let prepared = compiler::prepare(picture, scale, Some(pool), &mut |source| {
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

/// One pending batch per owner, admitted into the shared worker pool.
pub(crate) struct Background<T> {
    pending: Option<Pending<T>>,
}

struct Pending<T> {
    ready: std::sync::mpsc::Receiver<T>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl<T> Default for Background<T> {
    fn default() -> Self {
        Self { pending: None }
    }
}

impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

impl<T: Send + 'static> Background<T> {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.pending = None;
    }

    pub(crate) fn take_ready(&mut self) -> Option<T> {
        match self.pending.as_ref()?.ready.try_recv() {
            Ok(ready) => {
                self.pending = None;
                Some(ready)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                None
            }
        }
    }

    pub(crate) fn offer(
        &mut self,
        notify: Arc<dyn Fn() + Send + Sync>,
        work: impl FnOnce(&(dyn Fn() -> bool + Sync)) -> T + Send + 'static,
    ) {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        static ACTIVE: AtomicUsize = AtomicUsize::new(0);
        struct Permit;
        impl Drop for Permit {
            fn drop(&mut self) {
                ACTIVE.fetch_sub(1, Ordering::Release);
            }
        }

        let Some(pool) = pool().filter(|_| !self.busy()) else {
            return;
        };
        let threads = pool.current_num_threads();
        let capacity = 2 * threads;
        if ACTIVE
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |count| {
                (count < capacity).then_some(count + 1)
            })
            .is_err()
        {
            return;
        }
        let permit = Permit;
        let (send, ready) = std::sync::mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.pending = Some(Pending {
            ready,
            cancelled: Arc::clone(&cancelled),
        });
        pool.spawn_fifo(move || {
            let cancelled = || cancelled.load(Ordering::Acquire);
            if cancelled() {
                return;
            }
            let result = work(&cancelled);
            drop(permit);
            if !cancelled() && send.send(result).is_ok() && !cancelled() {
                notify();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{Background, pool};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    #[test]
    fn cancelled_work_cannot_publish_into_a_new_request() {
        if pool().is_none() {
            return;
        }
        let timeout = Duration::from_secs(5);
        let (start, started) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (check, checked) = mpsc::channel();
        let (notify, notifications) = mpsc::channel();
        let stale_wake = notify.clone();
        let mut background = Background::default();
        background.offer(
            Arc::new(move || stale_wake.send(1).expect("stale wake receiver")),
            move |cancelled| {
                start.send(()).expect("started receiver");
                released.recv_timeout(timeout).expect("release slow job");
                check.send(cancelled()).expect("checked receiver");
                1
            },
        );
        started.recv_timeout(timeout).expect("worker started");
        assert_eq!(background.take_ready(), None);
        background.cancel();
        background.offer(
            Arc::new(move || notify.send(2).expect("wake receiver")),
            |_| 2,
        );
        release.send(()).expect("release receiver");
        assert!(checked.recv_timeout(timeout).expect("worker checked epoch"));
        assert_eq!(notifications.recv_timeout(timeout).expect("ready wake"), 2);
        assert_eq!(background.take_ready(), Some(2));
        assert_eq!(background.take_ready(), None);
    }
}
