use std::ffi::c_void;
use std::ptr::with_exposed_provenance_mut;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use dispatch2::DispatchQueue;
use embedder::Command;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_quartz_core::CAMetalDrawable;
use raster::Status;

use super::{Event, Events, ImpastoIOSHost, input, with_host};
use crate::surface::LayerLease;

const MAX_OVERLAYS: usize = 12;

type Drawable = ProtocolObject<dyn CAMetalDrawable>;

#[derive(Default)]
pub(crate) struct Compositor {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    surface: u64,
    configured: bool,
    requested: Option<usize>,
    layers: Vec<Arc<LayerLease>>,
    frame: Option<Frame>,
    serial: u64,
    submitted: bool,
}

struct Frame {
    serial: u64,
    resets: bool,
    drawables: Vec<usize>,
    published_at: Instant,
}

impl Drop for Frame {
    fn drop(&mut self) {
        let drawables = std::mem::take(&mut self.drawables);
        DispatchQueue::main().exec_async(move || {
            for address in drawables {
                // SAFETY: Frame owns one retain of each drawable. UIKit borrows them until ack.
                drop(unsafe {
                    Retained::<Drawable>::from_raw(with_exposed_provenance_mut(address))
                });
            }
        });
    }
}

pub(crate) struct Targets {
    pub(crate) layers: Vec<Arc<LayerLease>>,
    pub(crate) resets: bool,
}

impl Compositor {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn reset(&self, surface: u64) {
        let mut state = self.lock();
        *state = State {
            surface,
            serial: state.serial,
            ..State::default()
        };
    }

    pub(crate) fn refresh_layers(&self, surface: u64) {
        let mut state = self.lock();
        if state.surface == surface {
            state.layers.clear();
            state.frame = None;
            state.requested = None;
        }
    }

    pub(crate) fn configured(&self, surface: u64) -> bool {
        let state = self.lock();
        state.surface == surface && state.configured
    }

    pub(crate) fn targets(
        &self,
        surface: u64,
        count: usize,
        events: &Events,
    ) -> Result<Option<Targets>, Status> {
        let mut state = self.lock();
        if state.surface != surface || state.frame.is_some() {
            return Err(Status::AcquireMiss);
        }
        if count == 0 {
            return Ok(state.configured.then(|| Targets {
                layers: Vec::new(),
                resets: true,
            }));
        }
        if count > MAX_OVERLAYS {
            return Err(Status::BadScene);
        }
        if state.layers.len() >= count {
            return Ok(Some(Targets {
                layers: state.layers[..count].to_vec(),
                resets: false,
            }));
        }
        if state.requested != Some(count) {
            state.requested = Some(count);
            drop(state);
            events.send(
                Event::CompositorLayers,
                surface,
                Some(&serde_json::json!({"count": count}).to_string()),
            );
        }
        Err(Status::AcquireMiss)
    }

    pub(crate) fn publish(
        &self,
        surface: u64,
        drawables: Vec<Retained<Drawable>>,
        resets: bool,
        views: &[engine::tree::PlatformViewPlacement],
        events: &Events,
    ) -> Result<(), Status> {
        let mut state = self.lock();
        if state.surface != surface || state.frame.is_some() {
            return Err(Status::AcquireMiss);
        }
        state.serial = state.serial.wrapping_add(1);
        let frame = Frame {
            serial: state.serial,
            resets,
            published_at: Instant::now(),
            drawables: drawables
                .into_iter()
                .map(|drawable| Retained::into_raw(drawable).expose_provenance())
                .collect(),
        };
        let message = serde_json::json!({
            "serial": frame.serial, "resets": resets, "drawables": frame.drawables,
            "views": input::platform_views_value(views),
        })
        .to_string();
        state.frame = Some(frame);
        drop(state);
        events.send(Event::ComposedFrame, surface, Some(&message));
        Ok(())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_compositor_layers(
    host: *mut ImpastoIOSHost,
    surface: u64,
    layers: *const *const c_void,
    count: usize,
) -> bool {
    with_host(host, |host| {
        if !host.current_surface(surface) || count == 0 || count > MAX_OVERLAYS || layers.is_null()
        {
            return false;
        }
        let mut state = host.compositor.lock();
        if state.surface != surface || state.requested != Some(count) {
            return false;
        }
        // SAFETY: the platform owns the array and its CAMetalLayers during this call.
        let layers = unsafe { std::slice::from_raw_parts(layers, count) };
        let retained = layers
            .iter()
            .map(|layer| {
                // SAFETY: the platform passes live layer references.
                unsafe { LayerLease::retain(layer.cast_mut()) }.map(Arc::new)
            })
            .collect::<Option<Vec<_>>>();
        let Some(retained) = retained else {
            return false;
        };
        state.layers = retained;
        state.configured = true;
        state.requested = None;
        drop(state);
        host.link.post(Command::Invalidate);
        true
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_compositor_finish(
    host: *mut ImpastoIOSHost,
    surface: u64,
    serial: u64,
    submitted: bool,
) {
    with_host(host, |host| {
        let mut state = host.compositor.lock();
        if state.surface != surface
            || state
                .frame
                .as_ref()
                .is_none_or(|frame| frame.serial != serial)
        {
            return;
        }
        let Some(frame) = state.frame.take() else {
            return;
        };
        if frame.resets && submitted {
            state.configured = false;
            state.layers.clear();
        }
        let first = submitted && !state.submitted;
        state.submitted |= submitted;
        drop(state);
        let ack_us = frame.published_at.elapsed().as_micros();
        tracing::debug!(target: "impasto::surface", serial, submitted, ack_us, "UIKit compositor ack");
        engine::session::trace::Section::event(|| format!("UIKit ack: {ack_us} us"));
        drop(frame);
        if first {
            host.clock.events.send(Event::FrameSubmitted, surface, None);
        }
        if !submitted {
            host.link.post(Command::Invalidate);
        }
    });
}
