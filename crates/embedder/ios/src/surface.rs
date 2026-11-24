use std::ffi::c_void;
use std::ptr::{NonNull, with_exposed_provenance_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use embedder::{NativePtr, Presented, Surface, SurfaceId, SurfaceTarget};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{msg_send, sel};
use objc2_metal::{MTLDrawable, MTLTexture};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use raster::{PixelSize, Rasterizer, RawSurfaceHandle, Status, WgpuSurface};

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
    surface: WgpuSurface,
    gate: Arc<SurfaceGate>,
    events: Arc<Events>,
    visible_frame: Arc<AtomicBool>,
    submitted: bool,
    compositor: Arc<Compositor>,
}

impl MetalSurface {
    pub(crate) fn open(
        raster: &mut Rasterizer,
        target: SurfaceTarget,
        gate: Arc<SurfaceGate>,
        events: Arc<Events>,
        compositor: Arc<Compositor>,
    ) -> Result<Self, Status> {
        let layer = NonNull::new(target.layer.0).ok_or(Status::SurfaceLost)?;
        // SAFETY: the registry retains this CAMetalLayer until raster retirement is acknowledged.
        let surface = unsafe {
            raster.create_surface(RawSurfaceHandle::CoreAnimationLayer(layer), target.pixels)
        }?;
        let result = Self {
            target,
            surface,
            gate,
            events,
            visible_frame: Arc::new(AtomicBool::new(false)),
            submitted: false,
            compositor,
        };
        result.install_observer();
        Ok(result)
    }

    fn draw(&mut self, engine: &mut Rasterizer, pixels: PixelSize) -> Result<Presented, Status> {
        let id = self.target.id;
        if !self.gate.allows(id) {
            return Err(Status::AcquireMiss);
        }

        engine.prepare_metal_surface(&mut self.surface, pixels)?;
        // SAFETY: this surface's registry lease is active.
        let layer = unsafe { &*self.target.layer.0.cast::<CAMetalLayer>() };
        layer.setAllowsNextDrawableTimeout(true);
        engine.present(&mut self.surface, pixels)?;
        if !self.submitted && self.gate.allows(id) {
            self.submitted = true;
            self.events.send(Event::FrameSubmitted, id.0, None);
        }
        Ok(Presented::Continue)
    }

    fn install_observer(&self) {
        // SAFETY: this surface's registry lease is active.
        let layer = unsafe { &*self.target.layer.0.cast::<CAMetalLayer>() };
        if !layer.respondsToSelector(sel!(impastoSetDrawableObserver:)) {
            return;
        }
        let id = self.target.id;
        let visible = Arc::clone(&self.visible_frame);
        let events = Arc::downgrade(&self.events);
        let gate = Arc::clone(&self.gate);
        let observer = RcBlock::new(
            move |drawable: NonNull<ProtocolObject<dyn CAMetalDrawable>>| {
                if visible.load(Ordering::Acquire) || !gate.allows(id) {
                    return;
                }
                // SAFETY: the layer passes a retained drawable to this block.
                let drawable = unsafe { drawable.as_ref() };
                if !drawable.respondsToSelector(sel!(addPresentedHandler:))
                    || !drawable.respondsToSelector(sel!(presentedTime))
                {
                    return;
                }
                let visible = Arc::clone(&visible);
                let events = events.clone();
                let gate = Arc::clone(&gate);
                let handler =
                    RcBlock::new(move |drawable: NonNull<ProtocolObject<dyn MTLDrawable>>| {
                        // SAFETY: Metal retains the drawable for the duration of the callback.
                        if unsafe { drawable.as_ref() }.presentedTime() > 0.0
                            && gate.allows(id)
                            && !visible.swap(true, Ordering::AcqRel)
                            && let Some(events) = events.upgrade()
                        {
                            events.send(Event::FramePresented, id.0, None);
                        }
                    });
                // SAFETY: Metal copies the valid block before returning.
                unsafe { drawable.addPresentedHandler(RcBlock::as_ptr(&handler)) };
            },
        );
        // SAFETY: the Swift CAMetalLayer subclass implements this selector
        // with a drawable block parameter and retains a copy of the block.
        let _: () = unsafe { msg_send![layer, impastoSetDrawableObserver: &*observer] };
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
        let ordered = engine.platform_views();
        let interleaved = ordered.len() == views.len()
            && ordered
                .iter()
                .zip(views)
                .all(|(expected, view)| *expected == view.node.to_raw());
        engine.prepare_metal_surface(&mut self.surface, pixels)?;
        let count = if interleaved { views.len() } else { 0 };
        let Some(targets) = self.compositor.targets(id.0, count, &self.events)? else {
            return self.draw(engine, pixels);
        };
        let acquire_started = std::time::Instant::now();
        let drawables = std::iter::once(self.target.layer)
            .chain(targets.layers.iter().map(|layer| layer.ptr()))
            .map(|layer| acquire(layer, pixels))
            .collect::<Result<Vec<_>, _>>();
        engine.note_metal_acquire(acquire_started.elapsed());
        let drawables = drawables?;
        if !self.gate.allows(id) {
            return Err(Status::AcquireMiss);
        }
        let mut batch = engine.begin_metal_slices(&self.surface)?;
        for (index, drawable) in drawables.iter().enumerate() {
            let slice = (!targets.resets).then_some(index);
            // SAFETY: the drawable belongs to the same Metal device and remains
            // retained through all encoding and the UIKit transaction.
            unsafe {
                engine.encode_metal_slice(
                    &mut batch,
                    slice,
                    Retained::as_ptr(drawable).cast_mut().cast(),
                    pixels,
                )
            }?;
        }
        engine.wait_for_metal_schedule(batch)?;
        if !self.gate.allows(id) {
            return Err(Status::AcquireMiss);
        }
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
    layer.setAllowsNextDrawableTimeout(true);
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
        objc2::rc::autoreleasepool(|_| self.compose(engine, pixels, views))
    }

    fn recover_device(&mut self, engine: &mut Rasterizer, pixels: PixelSize) -> Result<(), Status> {
        engine.recover_surface(&mut self.surface, pixels)?;
        self.compositor.refresh_layers(self.target.id.0);
        Ok(())
    }
}
