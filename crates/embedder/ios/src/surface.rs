use std::ffi::c_void;
use std::ptr::{NonNull, with_exposed_provenance_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use embedder::{NativePtr, Presented, Surface, SurfaceId, SurfaceTarget};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::sel;
use objc2_metal::{MTLDrawable, MTLTexture};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use raster::{PixelSize, Rasterizer, Status};

use crate::bridge::compositor::Compositor;
use crate::bridge::{Event, Events};

/// A retain acquired on the UIKit thread. Only the render thread dereferences
/// the address, and the final release is dispatched back to UIKit.
pub(crate) struct LayerLease(usize);

impl LayerLease {
    /// The caller provides a live CAMetalLayer on its UIKit thread.
    pub(crate) unsafe fn retain(layer: *mut c_void) -> Option<Self> {
        // SAFETY: the C caller supplies a live CAMetalLayer.
        unsafe { Retained::retain(layer.cast::<CAMetalLayer>()) }
            .map(|layer| Self(Retained::into_raw(layer).expose_provenance()))
    }

    pub(crate) fn ptr(&self) -> NativePtr {
        NativePtr(with_exposed_provenance_mut(self.0))
    }
}

impl Drop for LayerLease {
    fn drop(&mut self) {
        let address = self.0;
        DispatchQueue::main().exec_async(move || {
            // SAFETY: address stores exactly one retain from into_raw.
            drop(unsafe {
                Retained::<CAMetalLayer>::from_raw(with_exposed_provenance_mut(address))
            });
        });
    }
}

#[derive(Default)]
pub(crate) struct SurfaceGate {
    pub(crate) active: AtomicU64,
    pub(crate) visible: AtomicBool,
}

impl SurfaceGate {
    fn allows(&self, id: SurfaceId) -> bool {
        self.active.load(Ordering::Acquire) == id.0 && self.visible.load(Ordering::Acquire)
    }
}

pub(crate) struct MetalSurface {
    target: SurfaceTarget,
    gate: Arc<SurfaceGate>,
    events: Arc<Events>,
    visible_frame: Arc<AtomicBool>,
    submitted: bool,
    compositor: Arc<Compositor>,
    encode_failures: u8,
}

impl MetalSurface {
    pub(crate) fn new(
        target: SurfaceTarget,
        gate: Arc<SurfaceGate>,
        events: Arc<Events>,
        compositor: Arc<Compositor>,
    ) -> Self {
        Self {
            target,
            gate,
            events,
            visible_frame: Arc::new(AtomicBool::new(false)),
            submitted: false,
            compositor,
            encode_failures: 0,
        }
    }

    fn draw(&mut self, engine: &mut Rasterizer, pixels: PixelSize) -> Result<Presented, Status> {
        let id = self.target.id;
        if !self.gate.allows(id) {
            return Err(Status::AcquireMiss);
        }

        // SAFETY: the surface registry retains the layer through the render
        // thread's acknowledgement of retirement. Only drawable acquisition is
        // performed here. UIKit configures the view and layer properties.
        let drawable = acquire(self.target.layer, pixels)?;
        if !self.gate.allows(id) {
            return Err(Status::AcquireMiss);
        }
        self.observe(id, &drawable);
        engine.draw_metal(Retained::as_ptr(&drawable).cast_mut().cast())?;
        if !self.submitted && self.gate.allows(id) {
            self.submitted = true;
            self.events.send(Event::FrameSubmitted, id.0, None);
        }
        Ok(Presented::Continue)
    }

    fn observe(&self, id: SurfaceId, drawable: &ProtocolObject<dyn CAMetalDrawable>) {
        // Some simulator drawables lack presentation reporting entirely.
        if !self.visible_frame.load(Ordering::Acquire)
            && drawable.respondsToSelector(sel!(addPresentedHandler:))
            && drawable.respondsToSelector(sel!(presentedTime))
        {
            let visible = Arc::clone(&self.visible_frame);
            let events = Arc::downgrade(&self.events);
            let gate = Arc::clone(&self.gate);
            let handler =
                RcBlock::new(move |drawable: NonNull<ProtocolObject<dyn MTLDrawable>>| {
                    // SAFETY: Metal supplies the retained drawable for the duration of this block.
                    let time = unsafe { drawable.as_ref() }.presentedTime();
                    if time > 0.0
                        && gate.allows(id)
                        && !visible.swap(true, Ordering::AcqRel)
                        && let Some(events) = events.upgrade()
                    {
                        events.send(Event::FramePresented, id.0, None);
                    }
                });
            // SAFETY: Metal copies the valid block and calls it with the presented drawable.
            unsafe { drawable.addPresentedHandler(RcBlock::as_ptr(&handler)) };
        }
    }

    fn compose(
        &mut self,
        engine: &mut Rasterizer,
        pixels: PixelSize,
        views: &[engine::tree::PlatformViewPlacement],
    ) -> Result<Presented, Status> {
        let id = self.target.id;
        if !self.gate.allows(id) {
            return Err(Status::AcquireMiss);
        }
        let ordered = engine.metal_platform_views();
        let interleaved = ordered.len() == views.len()
            && ordered
                .iter()
                .zip(views)
                .all(|(expected, view)| *expected == view.node.to_raw());
        let count = if interleaved { views.len() } else { 0 };
        let Some(targets) = self.compositor.targets(id.0, count, &self.events)? else {
            return self.draw(engine, pixels);
        };
        let mut drawables = Vec::with_capacity(count + 1);
        drawables.push(acquire(self.target.layer, pixels)?);
        for layer in &targets.layers {
            drawables.push(acquire(layer.ptr(), pixels)?);
        }
        if !self.gate.allows(id) {
            return Err(Status::AcquireMiss);
        }
        self.observe(id, &drawables[0]);
        for (index, drawable) in drawables.iter().enumerate() {
            let slice = (!targets.resets).then_some(index);
            // SAFETY: the drawable belongs to the same Metal device and remains
            // retained through all encoding and the UIKit transaction.
            unsafe { engine.encode_metal(slice, Retained::as_ptr(drawable).cast_mut().cast()) }?;
        }
        engine.wait_for_metal_schedule()?;
        let views = if targets.resets { &[] } else { views };
        self.compositor
            .publish(id.0, drawables, targets.resets, views, &self.events)?;
        Ok(Presented::Continue)
    }
}

fn acquire(
    layer: NativePtr,
    pixels: PixelSize,
) -> Result<Retained<ProtocolObject<dyn CAMetalDrawable>>, Status> {
    // SAFETY: the caller owns or borrows a layer lease until acquisition returns.
    let layer = unsafe { &*layer.0.cast::<CAMetalLayer>() };
    let drawable = layer.nextDrawable().ok_or(Status::AcquireMiss)?;
    let texture = drawable.texture();
    if texture.width() != pixels.width as usize || texture.height() != pixels.height as usize {
        return Err(Status::AcquireMiss);
    }
    Ok(drawable)
}

impl Surface for MetalSurface {
    fn present(&mut self, engine: &mut Rasterizer, pixels: PixelSize) -> Result<Presented, Status> {
        objc2::rc::autoreleasepool(|_| self.draw(engine, pixels))
    }

    fn present_with_views(
        &mut self,
        engine: &mut Rasterizer,
        pixels: PixelSize,
        views: &[engine::tree::PlatformViewPlacement],
    ) -> Result<Presented, Status> {
        let result = objc2::rc::autoreleasepool(|_| self.compose(engine, pixels, views));
        match result {
            Ok(_) => self.encode_failures = 0,
            Err(Status::PresentFailed) => {
                self.encode_failures = self.encode_failures.saturating_add(1);
                if self.encode_failures >= 3 {
                    return Err(Status::DeviceLost);
                }
            }
            Err(_) => {}
        }
        result
    }

    fn recover_device(&mut self, engine: &mut Rasterizer) -> Result<(), Status> {
        self.encode_failures = 0;
        engine.recreate_graphics(Rasterizer::create_metal)
    }
}
