//! Paints a layer tree. Text uses the font size and paragraph width stored on each command.

use std::cell::RefCell;
#[cfg(not(target_os = "android"))]
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;

use engine::scene::{ContentMode, Gradient, LayerId, LayerKind, LayerTree, PictureId};

use crate::{ImagePixels, Status};

pub(crate) struct HeldParagraph {
    pub(crate) paragraph: impeller::ImpellerParagraph,
    /// Paint-slot index that owns the cache reference.
    pub(crate) slot: u32,
}

pub(crate) struct RetainedPicture {
    pub(crate) generation: u64,
    pub(crate) list: impeller::ImpellerDisplayList,
    /// Extra retains for paragraphs this display list draws. Released only with
    /// the display list, after any root list that flattened it.
    pub(crate) held: Vec<HeldParagraph>,
    /// Canvas images that were not ready. Reuse fails once any of them is cached.
    pub(crate) has_external_texture: bool,
    pub(crate) waiting: Vec<std::sync::Arc<str>>,
}

pub(crate) enum Save {
    Clip,
    Transform,
    /// Paint read by `Restore`. Released after that restore.
    Layer(impeller::ImpellerPaint),
    /// SrcOver is not a layer. Pop balances the command without `Restore`.
    Skipped,
}

pub struct Rasterizer {
    context: impeller::ImpellerContext,
    list: impeller::ImpellerDisplayList,
    #[cfg(target_os = "ios")]
    slices: metal_compositor::Slices,
    /// Temporaries the installed root display list still references.
    held: Vec<HeldParagraph>,
    pictures: crate::pictures::PictureCache,
    textures: crate::textures::TextureStore,
    typography: crate::typography::TypographyHandle,
    last_submit: crate::SubmitStats,
    /// Proc table and loader shared with Vulkan resources retained outside the engine.
    #[cfg(target_os = "android")]
    vulkan_procs: Option<std::sync::Arc<vulkan::VulkanProcs>>,
    #[cfg(target_os = "android")]
    gles: Option<Rc<gles::GlesContext>>,
    /// Impeller objects are created and used on the render thread.
    render_thread: PhantomData<*mut ()>,
}

#[cfg(target_os = "android")]
#[path = "../vulkan.rs"]
mod vulkan;

#[cfg(target_os = "android")]
#[path = "../gles.rs"]
mod gles;

#[cfg(target_os = "android")]
pub use gles::GlesWindow;

#[cfg(target_os = "android")]
pub use vulkan::VulkanSwapchain;

/// The linked Impeller library must match the header this crate was built against.
fn impeller_version_matches() -> bool {
    // SAFETY: ImpellerGetVersion takes no pointers and does not retain state.
    let linked = unsafe { impeller::ImpellerGetVersion() };
    linked == impeller::IMPELLER_VERSION
}

mod canvas;
mod images;
#[cfg(target_os = "ios")]
mod metal_compositor;
mod ops;
mod paint;
mod upload;

#[cfg(test)]
pub(crate) use paint::paint_trace;
pub(crate) use paint::*;

pub use upload::{GpuImage, TextureFactory};

impl Rasterizer {
    #[cfg(not(target_os = "android"))]
    pub fn create_metal() -> Result<Self, Status> {
        if !impeller_version_matches() {
            return Err(Status::NotLinked);
        }
        // SAFETY: ImpellerContextCreateMetalNew and ImpellerTypographyContextNew are the
        // documented constructors. A null handle means this SDK cannot create Metal.
        // Release is a no-op on null, per impeller.h.
        let (context, typography) = unsafe {
            let context = impeller::ImpellerContextCreateMetalNew(impeller::IMPELLER_VERSION);
            let typography = impeller::ImpellerTypographyContextNew();
            if context.is_null() || typography.is_null() {
                impeller::ImpellerTypographyContextRelease(typography);
                impeller::ImpellerContextRelease(context);
                return Err(Status::NotLinked);
            }
            (context, typography)
        };
        Ok(Self::blank(context, typography))
    }

    /// Null Impeller handles. Hostshell tests use this when they do not draw.
    /// `ImpellerContextRelease` is a no-op on null.
    pub fn inert() -> Self {
        Self::blank(ptr::null_mut(), ptr::null_mut())
    }

    pub(crate) fn blank(
        context: impeller::ImpellerContext,
        typography: impeller::ImpellerTypographyContext,
    ) -> Self {
        Self {
            context,
            list: ptr::null_mut(),
            #[cfg(target_os = "ios")]
            slices: metal_compositor::Slices::default(),
            held: Vec::new(),
            pictures: crate::pictures::PictureCache::new(),
            textures: crate::textures::TextureStore::new(),
            typography: std::mem::ManuallyDrop::new(Rc::new(RefCell::new(
                crate::typography::Typography::new(typography),
            ))),
            last_submit: crate::SubmitStats::default(),
            #[cfg(target_os = "android")]
            vulkan_procs: None,
            #[cfg(target_os = "android")]
            gles: None,
            render_thread: PhantomData,
        }
    }

    /// Shared with the session shaper. Measurement and drawing use one context.
    pub fn typography(&self) -> Rc<RefCell<crate::typography::Typography>> {
        Rc::clone(&self.typography)
    }

    #[cfg(test)]
    pub(crate) fn typography_mut(
        &mut self,
    ) -> std::cell::RefMut<'_, crate::typography::Typography> {
        self.typography.borrow_mut()
    }

    pub fn take_shape_stats(&mut self) -> crate::ShapeStats {
        self.typography.borrow_mut().take_shape_stats()
    }

    pub fn register_font(&mut self, data: &[u8], family: &str) -> Result<(), Status> {
        self.register_shared_font(std::sync::Arc::from(data), family)
    }

    pub fn register_shared_font(
        &mut self,
        data: std::sync::Arc<[u8]>,
        family: &str,
    ) -> Result<(), Status> {
        let installed = !self.list.is_null();
        let mut typography = self.typography.borrow_mut();
        let changed = typography.register_shared_face(data, family)?;
        if !changed {
            return Ok(());
        }
        if installed {
            typography.mark_stale();
        } else {
            typography.discard_measure_cache();
        }
        Ok(())
    }

    /// Composite `tree`. Pictures are cached by [`PictureId`] and generation.
    ///
    /// `point_scale` is applied once on the root builder. A cache hit still scans
    /// commands to mark textures used. Child display lists stay until the picture
    /// leaves the tree, and only after the root list that flattened them is released.
    pub fn submit_layers<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        mut resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        if point_scale <= 0.0 {
            return Err(Status::BadScene);
        }
        let mut typography = self.typography.borrow_mut();
        typography.begin_submit();
        self.last_submit = crate::SubmitStats::default();
        let mut swap = crate::pictures::PictureSwap::default();
        let mut pending: Vec<HeldParagraph> = Vec::new();
        let started = std::time::Instant::now();
        if let Err(status) = self.pictures.prepare(
            tree,
            point_scale,
            &mut resolve,
            &mut swap,
            &mut typography,
            &mut self.textures,
            &mut pending,
            &mut self.last_submit,
            self.context,
        ) {
            self.last_submit = crate::SubmitStats::default();
            self.pictures.rollback(swap, &mut typography);
            crate::painter::release_pending(&mut pending, &mut typography);
            return Err(status);
        }
        self.last_submit.picture_prepare = started.elapsed();
        let started = std::time::Instant::now();
        let builder = unsafe { impeller::ImpellerDisplayListBuilderNew(ptr::null()) };
        let paint = unsafe { impeller::ImpellerPaintNew() };
        if builder.is_null() || paint.is_null() {
            unsafe {
                impeller::ImpellerDisplayListBuilderRelease(builder);
                impeller::ImpellerPaintRelease(paint);
            }
            self.pictures.rollback(swap, &mut typography);
            crate::painter::release_pending(&mut pending, &mut typography);
            return Err(Status::BadScene);
        }
        let display = {
            let mut painter = crate::painter::Painter {
                typography: &mut typography,
                textures: &mut self.textures,
                picture_cache: Some(&self.pictures),
                pending: &mut pending,
                saves: Vec::new(),
                waiting: Vec::new(),
                impeller: self.context,
            };
            if !tree.layers.is_empty() {
                painter.scale_once(builder, point_scale, false);
            }
            let drawn =
                tree.layers.is_empty() || painter.draw_layers(tree, builder, Some(tree.root));
            let failed = !drawn || !painter.saves.is_empty();
            let display = if failed {
                ptr::null_mut()
            } else {
                unsafe { impeller::ImpellerDisplayListBuilderCreateDisplayListNew(builder) }
            };
            painter.drain_saves(builder);
            display
        };
        self.last_submit.compose = started.elapsed();
        unsafe {
            impeller::ImpellerDisplayListBuilderRelease(builder);
            impeller::ImpellerPaintRelease(paint);
        }
        if display.is_null() {
            self.last_submit = crate::SubmitStats::default();
            crate::painter::release_pending(&mut pending, &mut typography);
            self.pictures.rollback(swap, &mut typography);
            return Err(Status::BadScene);
        }
        #[cfg(target_os = "ios")]
        let slices = {
            let mut painter = crate::painter::Painter {
                typography: &mut typography,
                textures: &mut self.textures,
                picture_cache: Some(&self.pictures),
                pending: &mut pending,
                saves: Vec::new(),
                waiting: Vec::new(),
                impeller: self.context,
            };
            match metal_compositor::Slices::build(tree, &mut painter, point_scale) {
                Ok(slices) => slices,
                Err(error) => {
                    // SAFETY: the new fallback display list has not been installed.
                    unsafe { impeller::ImpellerDisplayListRelease(display) };
                    self.pictures.rollback(swap, &mut typography);
                    crate::painter::release_pending(&mut pending, &mut typography);
                    return Err(error);
                }
            }
        };
        #[cfg(target_os = "ios")]
        {
            self.slices = slices;
        }
        unsafe {
            impeller::ImpellerDisplayListRelease(self.list);
            self.list = ptr::null_mut();
        }
        typography.set_list_installed(false);
        for (_, picture) in swap.replaced {
            release_retained(picture, &mut typography);
        }
        self.pictures.evict_except(tree, &mut typography);
        typography.reap_retired();
        let previous = std::mem::take(&mut self.held);
        for item in previous {
            typography.release_draw(item.slot);
            // SAFETY: the root display list that referenced this retain was just released.
            unsafe { impeller::ImpellerParagraphRelease(item.paragraph) };
        }
        self.held = std::mem::take(&mut pending);
        typography.clear_stale();
        typography.set_list_installed(true);
        self.list = display;
        typography.evict();
        typography.evict_over_cap();
        let generation = typography.generation();
        self.textures.evict(generation);
        self.textures.evict_over_cap(generation);
        self.last_submit.layer_count = u32::try_from(tree.layers.len()).unwrap_or(u32::MAX);
        typography.note_paragraph_limit();
        self.textures.note_limit();
        self.pictures.note_limit();
        self.last_submit.paragraphs = u32::try_from(typography.measured_len()).unwrap_or(u32::MAX);
        self.last_submit.pictures = u32::try_from(self.pictures.len()).unwrap_or(u32::MAX);
        self.last_submit.textures = u32::try_from(self.textures.len()).unwrap_or(u32::MAX);
        Ok(())
    }

    /// The installed display lists independently retain the underlying image.
    pub fn invalidate_pictures(&mut self) {
        self.typography.borrow_mut().mark_stale();
    }

    pub fn forget_texture(&mut self, source: &str) {
        self.textures.forget(source);
    }

    /// Pictures built since the engine was created. The cache key is picture id and generation.
    pub fn picture_builds(&self) -> u64 {
        self.pictures.builds()
    }

    /// Counts from the last successful [`Rasterizer::submit_layers`].
    pub fn submit_stats(&self) -> crate::SubmitStats {
        self.last_submit
    }

    #[cfg(not(target_os = "android"))]
    pub fn draw_metal(&mut self, metal_drawable: *mut c_void) -> Result<(), Status> {
        if self.context.is_null() || self.list.is_null() {
            return Err(Status::NotReady);
        }
        if metal_drawable.is_null() {
            return Err(Status::AcquireMiss);
        }
        // SAFETY: `metal_drawable` is a CAMetalDrawable from the system default device,
        // kept alive by the caller for this call. The context uses that same device.
        let surface = unsafe {
            impeller::ImpellerSurfaceCreateWrappedMetalDrawableNew(self.context, metal_drawable)
        };
        if surface.is_null() {
            return Err(Status::PresentFailed);
        }
        let presented = unsafe {
            let drew = impeller::ImpellerSurfaceDrawDisplayList(surface, self.list);
            let presented = drew && impeller::ImpellerSurfacePresent(surface);
            impeller::ImpellerSurfaceRelease(surface);
            presented
        };
        if presented {
            Ok(())
        } else {
            Err(Status::PresentFailed)
        }
    }

    pub(crate) fn release_display(&mut self) {
        #[cfg(target_os = "ios")]
        {
            self.slices = metal_compositor::Slices::default();
        }
        // SAFETY: `list` is null or the display list from the previous submit.
        unsafe {
            impeller::ImpellerDisplayListRelease(self.list);
            self.list = ptr::null_mut();
        }
    }

    pub(crate) fn release_held(&mut self) {
        let held = std::mem::take(&mut self.held);
        let mut typography = self.typography.borrow_mut();
        for item in held {
            typography.release_draw(item.slot);
            // SAFETY: the display list that referenced this retain was just released.
            unsafe { impeller::ImpellerParagraphRelease(item.paragraph) };
        }
    }

    pub(crate) fn release_frame(&mut self) {
        self.release_display();
        self.release_held();
        let mut typography = self.typography.borrow_mut();
        self.pictures.release_all(&mut typography);
        self.textures.release_all();
        typography.set_list_installed(false);
        typography.release_all_paragraphs();
    }

    /// Drops the installed display list, pictures, textures, and paragraphs.
    /// The context and registered faces stay, so the next [`Self::submit_layers`]
    /// can rebuild. Drawing returns [`Status::NotReady`] until that submit.
    pub fn trim(&mut self) {
        self.release_frame();
        self.last_submit = crate::SubmitStats::default();
    }

    /// Replaces GPU resources after the host has released its old surface.
    /// The shaper retains the typography context and registered fonts.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn recreate_graphics(
        &mut self,
        create: impl FnOnce() -> Result<Self, Status>,
    ) -> Result<(), Status> {
        {
            #[cfg(target_os = "android")]
            let procs = self.vulkan_procs.clone();
            #[cfg(target_os = "android")]
            let _guard = procs.as_deref().map(vulkan::VulkanProcs::current);
            self.trim();
        }
        // Drop EGL before binding a new context on this thread.
        let mut retired = Self::inert();
        std::mem::swap(&mut self.typography, &mut retired.typography);
        std::mem::swap(self, &mut retired);
        drop(retired);
        let mut replacement = create()?;
        std::mem::swap(&mut self.typography, &mut replacement.typography);
        std::mem::swap(self, &mut replacement);
        Ok(())
    }

    /// Test helper. Each call uses a new picture generation so the commands are rebuilt.
    #[cfg(test)]
    pub(crate) fn paint<'pixels>(
        &mut self,
        list: &engine::scene::DrawList,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        let id = PictureId(1);
        let picture = std::sync::Arc::new(engine::scene::Picture::new(
            id,
            generation,
            (0.0, 0.0),
            std::sync::Arc::from(list.commands.as_slice()),
        ));
        let tree = LayerTree {
            layers: vec![engine::scene::Layer {
                kind: LayerKind::Picture { picture: id },
                parent: None,
                first_child: None,
                next: None,
            }],
            pictures: vec![picture],
            root: LayerId(0),
            epoch: generation,
        };
        self.submit_layers(&tree, point_scale, resolve)
    }

    #[cfg(test)]
    pub(crate) fn cached_textures(&self) -> usize {
        self.textures.len()
    }

    #[cfg(test)]
    pub(crate) fn cached_paragraphs(&self) -> usize {
        self.typography.borrow().measured_len()
    }

    #[cfg(test)]
    pub(crate) fn retained_pictures(&self) -> usize {
        self.pictures.len()
    }
}

pub(crate) fn release_retained(
    picture: RetainedPicture,
    typography: &mut crate::typography::Typography,
) {
    // SAFETY: `list` is the display list this picture owns. Release is a no-op on null.
    unsafe { impeller::ImpellerDisplayListRelease(picture.list) };
    for item in picture.held {
        typography.release_draw(item.slot);
        // SAFETY: this is the picture's extra retain, released with its display list.
        unsafe { impeller::ImpellerParagraphRelease(item.paragraph) };
    }
}

pub(crate) fn single_picture_child(tree: &LayerTree, id: LayerId) -> Option<PictureId> {
    let layer = tree.layers.get(id.0 as usize)?;
    let child = tree.layers.get(layer.first_child?.0 as usize)?;
    if child.next.is_some() {
        return None;
    }
    match child.kind {
        LayerKind::Picture { picture } => Some(picture),
        _ => None,
    }
}

impl crate::Surface for Rasterizer {
    fn register_font(&mut self, data: &[u8], family: &str) -> Result<(), Status> {
        Rasterizer::register_font(self, data, family)
    }

    fn submit_layers<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<(), Status> {
        Rasterizer::submit_layers(self, tree, point_scale, resolve)
    }

    #[cfg(not(target_os = "android"))]
    fn draw_metal(&mut self, metal_drawable: *mut c_void) -> Result<(), Status> {
        Rasterizer::draw_metal(self, metal_drawable)
    }

    fn picture_builds(&self) -> u64 {
        Rasterizer::picture_builds(self)
    }

    fn submit_stats(&self) -> crate::SubmitStats {
        Rasterizer::submit_stats(self)
    }
}

impl Drop for Rasterizer {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        let procs = self.vulkan_procs.take();
        #[cfg(target_os = "android")]
        let _guard = procs.as_deref().map(vulkan::VulkanProcs::current);
        self.release_frame();
        let last = Rc::strong_count(&self.typography) == 1;
        // SAFETY: the wrapper stays valid. This drops our `Rc` before the graphics context.
        // Paragraph pointers are already null, so `Typography::drop` does not release them twice.
        unsafe { std::mem::ManuallyDrop::drop(&mut self.typography) };
        if last {
            // SAFETY: `context` came from the context constructor. Release is a no-op on null.
            unsafe { impeller::ImpellerContextRelease(self.context) };
            self.context = ptr::null_mut();
        } else {
            debug_assert!(false, "shaper dropped after Rasterizer");
        }
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::Rasterizer;
    use crate::{ImagePixels, Status};
    use engine::scene::DrawList;

    #[test]
    #[ignore]
    fn submit_bench() {
        let mut engine = match Rasterizer::create_metal() {
            Ok(engine) => engine,
            Err(Status::NotLinked) => return,
            Err(status) => panic!("{status}"),
        };
        const WARMUP: u64 = 10;
        const SAMPLES: u64 = 40;
        let commands = scrolled_commands();
        let mut samples = Vec::with_capacity(SAMPLES as usize);
        for index in 0..WARMUP + SAMPLES {
            let tree = picture_tree(0.0, index + 1, std::sync::Arc::clone(&commands));
            let started = std::time::Instant::now();
            engine.submit_layers(&tree, 2.0, |_| None).expect("submit");
            if index >= WARMUP {
                samples.push(started.elapsed());
            }
        }
        samples.sort();
        let p95 = samples[37];
        println!("submit_p95={:.3}ms", p95.as_secs_f64() * 1000.0);
    }

    #[test]
    fn rejected_font_bytes_return_bad_scene() {
        let mut engine = match Rasterizer::create_metal() {
            Ok(engine) => engine,
            Err(Status::NotLinked) => return,
            Err(status) => panic!("{status}"),
        };
        let secret = b"SECRET-FONT-BYTES-DO-NOT-LOG";
        assert_eq!(
            engine.register_font(secret, "Custom"),
            Err(Status::BadScene)
        );
    }

    #[test]
    fn non_positive_scale_is_rejected() {
        let mut engine = match Rasterizer::create_metal() {
            Ok(engine) => engine,
            Err(Status::NotLinked) => return,
            Err(status) => panic!("{status}"),
        };
        let list = DrawList {
            commands: Vec::new(),
        };
        assert_eq!(engine.paint(&list, 0.0, |_| None), Err(Status::BadScene));
        assert_eq!(
            engine.submit_layers(&engine::scene::LayerTree::empty(), 0.0, |_| None),
            Err(Status::BadScene)
        );
    }

    #[test]
    fn measured_paragraph_matches_a_second_query_and_submits() {
        let mut engine = match Rasterizer::create_metal() {
            Ok(engine) => engine,
            Err(Status::NotLinked) => return,
            Err(status) => panic!("{status}"),
        };
        let font = match std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf") {
            Ok(font) => font,
            Err(_) => return,
        };
        engine.register_font(&font, "Sans").expect("font registers");
        let first = engine
            .typography_mut()
            .measure_paragraph(
                "Hi",
                28.0,
                280.0,
                [255, 255, 255, 255],
                &engine::scene::TextStyle::default(),
            )
            .expect("measure");
        let second = engine
            .typography_mut()
            .measure_paragraph(
                "Hi",
                28.0,
                280.0,
                [255, 255, 255, 255],
                &engine::scene::TextStyle::default(),
            )
            .expect("cached measure");
        assert_eq!(first, second);
        assert!(first.0 > 0.0 && first.1 > 0.0);
        let style = engine::scene::TextStyle::default();
        let color = [255, 255, 255, 255];
        let lines = engine
            .typography_mut()
            .lines("Hi", 28.0, 280.0, color, &style)
            .expect("lines");
        let again = engine
            .typography_mut()
            .lines("Hi", 28.0, 280.0, color, &style)
            .expect("cached lines");
        assert_eq!(lines.len(), again.len());
        assert!(!lines.is_empty());
        let caret = engine
            .typography_mut()
            .caret_for(
                engine::text::ParagraphQuery {
                    utf8: "Hi",
                    font_size: 28.0,
                    max_width: 280.0,
                    color,
                    style: &style,
                },
                2,
                false,
            )
            .expect("caret at end");
        assert!(caret.x > 0.0);
        let list = DrawList {
            commands: vec![engine::scene::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                paragraph: engine::scene::ParagraphId::NONE,
                color: [255, 255, 255, 255],
                utf8: std::sync::Arc::from("Hi"),
                style: std::sync::Arc::new(engine::scene::TextStyle::default()),
                spans: std::sync::Arc::from([]),
            }],
        };
        engine
            .paint(&list, 1.0, |_| None)
            .expect("measured text submits");
    }

    #[test]
    fn second_submit_reuses_the_texture() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = logo_list();
        let rgba = [0u8; 4];
        let mut calls = 0;
        let submit = |engine: &mut Rasterizer, calls: &mut i32| {
            engine.paint(&list, 1.0, |_| {
                *calls += 1;
                Some(ImagePixels {
                    width: 1,
                    height: 1,
                    rgba: &rgba,
                })
            })
        };
        submit(&mut engine, &mut calls).expect("first image");
        submit(&mut engine, &mut calls).expect("second image");
        assert_eq!(calls, 1);
        assert_eq!(engine.cached_textures(), 1);
        let empty = DrawList {
            commands: Vec::new(),
        };
        engine.paint(&empty, 1.0, |_| None).expect("empty");
        assert_eq!(engine.cached_textures(), 0);
    }

    #[test]
    fn failed_submit_keeps_the_texture() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = logo_list();
        let rgba = [0u8; 4];
        let mut calls = 0;
        engine
            .paint(&list, 1.0, |_| {
                calls += 1;
                Some(ImagePixels {
                    width: 1,
                    height: 1,
                    rgba: &rgba,
                })
            })
            .expect("image");
        let broken = DrawList {
            commands: vec![engine::scene::Command::Pop],
        };
        assert_eq!(engine.paint(&broken, 1.0, |_| None), Err(Status::BadScene));
        assert_eq!(engine.cached_textures(), 1);
        engine
            .paint(&list, 1.0, |_| {
                calls += 1;
                Some(ImagePixels {
                    width: 1,
                    height: 1,
                    rgba: &rgba,
                })
            })
            .expect("reused image");
        assert_eq!(calls, 1);
    }

    #[test]
    fn measured_paragraph_survives_submit() {
        let Some(mut engine) = fonted() else {
            return;
        };
        let style = engine::scene::TextStyle::default();
        let first = engine
            .typography_mut()
            .measure_paragraph("Hi", 28.0, 280.0, [255, 255, 255, 255], &style)
            .expect("measure");
        let list = DrawList {
            commands: vec![engine::scene::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                paragraph: engine::scene::ParagraphId::NONE,
                color: [255, 255, 255, 255],
                utf8: std::sync::Arc::from("Hi"),
                style: std::sync::Arc::new(style.clone()),
                spans: std::sync::Arc::from([]),
            }],
        };
        engine.paint(&list, 1.0, |_| None).expect("submit");
        let second = engine
            .typography_mut()
            .measure_paragraph("Hi", 28.0, 280.0, [255, 255, 255, 255], &style)
            .expect("cached");
        assert_eq!(first, second);
        assert_eq!(engine.cached_paragraphs(), 1);
    }

    #[test]
    fn released_paragraph_id_does_not_paint_the_new_color() {
        let Some(mut engine) = fonted() else {
            return;
        };
        let style = engine::scene::TextStyle::default();
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let old = engine
            .typography_mut()
            .retain_paragraph("Hi", 28.0, 280.0, red, &style, &[])
            .expect("retain red");
        engine.typography_mut().release_paragraph(old);
        assert!(!engine.typography_mut().paragraph_live(old));
        let next = engine
            .typography_mut()
            .retain_paragraph("Hi", 28.0, 280.0, green, &style, &[])
            .expect("retain green");
        assert_eq!(old.index, next.index);
        assert_ne!(old.generation, next.generation);
        assert!(engine.typography_mut().paragraph_live(next));
        let list = DrawList {
            commands: vec![engine::scene::Command::Text {
                x: 0.0,
                y: 0.0,
                max_width: 280.0,
                font_size: 28.0,
                paragraph: old,
                color: red,
                utf8: std::sync::Arc::from("Hi"),
                style: std::sync::Arc::new(style),
                spans: std::sync::Arc::from([]),
            }],
        };
        engine
            .paint(&list, 1.0, |_| None)
            .expect("stale id rebuilds");
        assert_eq!(engine.typography_mut().text_paint_probe(), (true, red));
        assert!(engine.typography_mut().paragraph_live(next));
    }

    #[test]
    fn unused_paragraph_is_evicted() {
        let Some(mut engine) = fonted() else {
            return;
        };
        engine
            .typography_mut()
            .measure_paragraph(
                "A",
                28.0,
                280.0,
                [255, 255, 255, 255],
                &engine::scene::TextStyle::default(),
            )
            .expect("measure");
        let list = DrawList {
            commands: Vec::new(),
        };
        engine.paint(&list, 1.0, |_| None).expect("empty");
        assert_eq!(engine.cached_paragraphs(), 1);
        engine.typography_mut().begin_measure();
        engine.typography_mut().end_measure();
        assert_eq!(engine.cached_paragraphs(), 0);
    }

    fn fonted() -> Option<Rasterizer> {
        let mut engine = opened()?;
        let font = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").ok()?;
        engine.register_font(&font, "Sans").expect("font registers");
        Some(engine)
    }

    #[test]
    fn an_uploaded_texture_is_blitted_without_resolving_pixels() {
        let Some(mut engine) = opened() else {
            return;
        };
        let Some(factory) = engine.texture_factory() else {
            return;
        };
        let image = factory.upload_rgba(1, 1, &[0, 0, 0, 255]).expect("upload");
        engine.install_uploaded("asset:logo", &image);
        let mut calls = 0;
        engine
            .paint(&logo_list(), 1.0, |_| {
                calls += 1;
                None
            })
            .expect("blit installed texture");
        assert_eq!(calls, 0);
    }

    fn logo_list() -> DrawList {
        DrawList {
            commands: vec![engine::scene::Command::Image {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                source: std::sync::Arc::from("asset:logo"),
                mode: engine::scene::ContentMode::Fill,
                required: true,
            }],
        }
    }

    #[test]
    fn trim_drops_the_list_and_the_next_submit_rebuilds() {
        let Some(mut engine) = opened() else {
            return;
        };
        let tree = engine::scene::LayerTree::empty();
        engine
            .submit_layers(&tree, 1.0, |_| None)
            .expect("empty tree submits");
        engine.trim();
        assert_eq!(
            engine.draw_metal(std::ptr::null_mut()),
            Err(Status::NotReady)
        );
        engine
            .submit_layers(&tree, 1.0, |_| None)
            .expect("submit after trim");
        assert_eq!(
            engine.draw_metal(std::ptr::null_mut()),
            Err(Status::AcquireMiss)
        );
    }

    #[test]
    fn src_over_blend_does_not_reject_the_frame() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![
                engine::scene::Command::PushBlend {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 8.0,
                    mode: engine::scene::BlendMode::SrcOver,
                },
                solid_rect(0.0),
                engine::scene::Command::Pop,
            ],
        };
        engine
            .paint(&list, 1.0, |_| None)
            .expect("SrcOver is not a layer");
    }

    fn opened() -> Option<Rasterizer> {
        match Rasterizer::create_metal() {
            Ok(engine) => Some(engine),
            Err(Status::NotLinked) => None,
            Err(status) => panic!("{status}"),
        }
    }

    fn solid_rect(corner_radius: f32) -> engine::scene::Command {
        engine::scene::Command::Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 8.0,
            corner_radius,
            color: [9, 8, 7, 255],
            stroke_width: 0.0,
            stroke_color: [0, 0, 0, 0],
            gradient: None,
            shadow: None,
            box_shadows: std::sync::Arc::from([]),
        }
    }

    #[test]
    fn clip_and_opacity_groups_submit() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![
                engine::scene::Command::PushClip {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 8.0,
                    corner_radius: 2.0,
                },
                solid_rect(4.0),
                engine::scene::Command::Pop,
                engine::scene::Command::PushOpacity {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 8.0,
                    alpha: 128,
                },
                solid_rect(0.0),
                engine::scene::Command::Pop,
            ],
        };
        engine
            .paint(&list, 2.0, |_| None)
            .expect("clip and opacity submit");
    }

    #[test]
    fn nested_clip_and_transform_submit() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![
                engine::scene::Command::PushClip {
                    x: 0.0,
                    y: 0.0,
                    w: 20.0,
                    h: 20.0,
                    corner_radius: 0.0,
                },
                engine::scene::Command::PushTransform {
                    translate_x: 4.0,
                    translate_y: 2.0,
                    rotate_rad: 0.0,
                    scale_x: 1.5,
                    scale_y: 1.5,
                },
                solid_rect(0.0),
                engine::scene::Command::Pop,
                engine::scene::Command::Pop,
            ],
        };
        engine
            .paint(&list, 2.0, |_| None)
            .expect("transform submit");
    }

    #[test]
    fn extra_pop_is_rejected() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![engine::scene::Command::Pop],
        };
        assert_eq!(engine.paint(&list, 1.0, |_| None), Err(Status::BadScene));
    }

    #[test]
    fn unclosed_clip_is_rejected() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![engine::scene::Command::PushClip {
                x: 0.0,
                y: 0.0,
                w: 4.0,
                h: 4.0,
                corner_radius: 0.0,
            }],
        };
        assert_eq!(engine.paint(&list, 1.0, |_| None), Err(Status::BadScene));
    }

    #[test]
    fn fit_letterboxes_a_square_into_a_wide_box() {
        let (_src, dst) = super::content_rects(
            0.0,
            0.0,
            200.0,
            100.0,
            100.0,
            100.0,
            engine::scene::ContentMode::Fit,
        )
        .expect("fit");
        assert_eq!(
            (dst.x, dst.y, dst.width, dst.height),
            (50.0, 0.0, 100.0, 100.0)
        );
    }

    #[test]
    fn optional_missing_image_submits_and_required_does_not() {
        let Some(mut engine) = opened() else {
            return;
        };
        engine
            .paint(&image_command(false), 1.0, |_| None)
            .expect("optional image");
        assert_eq!(
            engine.paint(&image_command(true), 1.0, |_| None),
            Err(Status::BadScene)
        );
    }

    fn image_command(required: bool) -> DrawList {
        DrawList {
            commands: vec![engine::scene::Command::Image {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                source: std::sync::Arc::from("asset:missing"),
                mode: engine::scene::ContentMode::Fill,
                required,
            }],
        }
    }

    #[test]
    fn paint_units_stay_in_scene_points_at_scale_two() {
        let Some(mut engine) = opened() else {
            return;
        };
        super::paint_trace::clear();
        let list = DrawList {
            commands: vec![
                engine::scene::Command::PushBlur {
                    x: 0.0,
                    y: 0.0,
                    w: 20.0,
                    h: 20.0,
                    sigma: 4.0,
                },
                engine::scene::Command::Rect {
                    x: 1.0,
                    y: 2.0,
                    w: 10.0,
                    h: 8.0,
                    corner_radius: 0.0,
                    color: [1, 2, 3, 255],
                    stroke_width: 3.0,
                    stroke_color: [4, 5, 6, 255],
                    gradient: None,
                    shadow: Some(engine::scene::Shadow {
                        color: [0, 0, 0, 128],
                        elevation: 5.0,
                    }),
                    box_shadows: std::sync::Arc::from([]),
                },
                engine::scene::Command::Pop,
            ],
        };
        engine
            .paint(&list, 2.0, |_| None)
            .expect("scene-point paint");
        let trace = super::paint_trace::take();
        assert_eq!(trace.stroke_widths, vec![3.0]);
        assert_eq!(trace.blur_sigmas, vec![(4.0, 4.0)]);
        assert_eq!(trace.shadows, vec![(5.0, 2.0)]);
    }

    #[test]
    fn span_family_weight_and_line_through_split_the_paragraph_key() {
        let Some(mut engine) = fonted() else {
            return;
        };
        super::paint_trace::clear();
        let color = [0, 0, 0, 255];
        let base = engine::scene::TextStyle::default();
        let spans = |family: &str, weight: u16| {
            vec![
                engine::scene::TextSpan {
                    utf8: "ab".to_owned(),
                    color,
                    gradient: None,
                    weight: 400,
                    underline: false,
                    line_through: false,
                    family: Some("Sans".to_owned()),
                },
                engine::scene::TextSpan {
                    utf8: "cd".to_owned(),
                    color,
                    gradient: None,
                    weight,
                    underline: false,
                    line_through: false,
                    family: Some(family.to_owned()),
                },
            ]
        };
        let key = engine::text::paint_key(
            std::sync::Arc::from("abcd"),
            std::sync::Arc::new(base.clone()),
            &spans("Other", 700),
            16.0,
            200.0,
            color,
        );
        engine
            .typography_mut()
            .measure_key(&key.metrics, key.color, Some(&key.span_paints))
            .expect("first span key");
        engine
            .typography_mut()
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Other", 700))
            .expect("the scene reuses the layout span key");
        assert_eq!(engine.cached_paragraphs(), 1);
        engine
            .typography_mut()
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Sans", 700))
            .expect("family changes the key");
        assert_eq!(engine.cached_paragraphs(), 2);
        engine
            .typography_mut()
            .measure_with_spans("abcd", 16.0, 200.0, color, &base, &spans("Other", 400))
            .expect("weight changes the key");
        assert_eq!(engine.cached_paragraphs(), 3);
        let mut struck = base.clone();
        struck.line_through = true;
        engine
            .typography_mut()
            .measure_paragraph("Hi", 16.0, 200.0, color, &struck)
            .expect("line through");
        let line_through =
            impeller::ImpellerTextDecorationType::kImpellerTextDecorationTypeLineThrough as i32;
        let trace = super::paint_trace::take();
        assert!(
            trace
                .decorations
                .iter()
                .any(|types| types & line_through != 0),
            "{:?}",
            trace.decorations
        );
    }

    #[test]
    fn scroll_offset_keeps_the_picture_display_list() {
        let Some(mut engine) = opened() else {
            return;
        };
        let mut commands = Vec::with_capacity(5000);
        for row in 0..5000 {
            let mut command = solid_rect(0.0);
            if let engine::scene::Command::Rect { y, .. } = &mut command {
                *y = row as f32 * 8.0;
            }
            commands.push(command);
        }
        let commands = std::sync::Arc::<[engine::scene::Command]>::from(commands);
        let first = picture_tree(0.0, 1, std::sync::Arc::clone(&commands));
        engine
            .submit_layers(&first, 1.0, |_| None)
            .expect("first layer tree");
        assert_eq!(engine.picture_builds(), 1);
        let scrolled = picture_tree(40.0, 1, commands);
        engine
            .submit_layers(&scrolled, 2.0, |_| None)
            .expect("scrolled layer tree");
        assert_eq!(engine.picture_builds(), 1);
        assert_eq!(engine.retained_pictures(), 1);
        let rewritten = picture_tree(40.0, 2, scrolled_commands());
        engine
            .submit_layers(&rewritten, 2.0, |_| None)
            .expect("rewritten picture");
        assert_eq!(engine.picture_builds(), 2);
    }

    #[test]
    fn register_font_rebuilds_picture_display_lists() {
        let Some(mut engine) = fonted() else {
            return;
        };
        let style = std::sync::Arc::new(engine::scene::TextStyle::default());
        let command = engine::scene::Command::Text {
            x: 0.0,
            y: 0.0,
            max_width: 280.0,
            font_size: 28.0,
            paragraph: engine::scene::ParagraphId::NONE,
            color: [255, 255, 255, 255],
            utf8: std::sync::Arc::from("Hi"),
            style,
            spans: std::sync::Arc::from([]),
        };
        let tree = picture_tree(0.0, 1, std::sync::Arc::from([command]));
        engine
            .submit_layers(&tree, 1.0, |_| None)
            .expect("text picture");
        assert_eq!(engine.picture_builds(), 1);
        let font =
            std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("font file");
        engine
            .register_font(&font, "Sans")
            .expect("font registers again");
        engine
            .submit_layers(&tree, 1.0, |_| None)
            .expect("picture after the same font");
        assert_eq!(engine.picture_builds(), 1);
    }

    #[test]
    fn layer_image_stays_cached_when_the_picture_is_reused() {
        let Some(mut engine) = opened() else {
            return;
        };
        let command = engine::scene::Command::Image {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            source: std::sync::Arc::from("asset:logo"),
            mode: engine::scene::ContentMode::Fill,
            required: true,
        };
        let tree = picture_tree(0.0, 1, std::sync::Arc::from([command]));
        let rgba = [0u8; 4];
        let mut calls = 0;
        let submit = |engine: &mut Rasterizer, tree: &engine::scene::LayerTree, calls: &mut i32| {
            engine.submit_layers(tree, 1.0, |_| {
                *calls += 1;
                Some(ImagePixels {
                    width: 1,
                    height: 1,
                    rgba: &rgba,
                })
            })
        };
        submit(&mut engine, &tree, &mut calls).expect("first picture");
        submit(&mut engine, &tree, &mut calls).expect("cached picture");
        assert_eq!(calls, 1);
        assert_eq!(engine.cached_textures(), 1);
        let empty = engine::scene::LayerTree::empty();
        engine
            .submit_layers(&empty, 1.0, |_| None)
            .expect("empty tree");
        assert_eq!(engine.retained_pictures(), 0);
        assert_eq!(engine.cached_textures(), 0);
    }

    fn scrolled_commands() -> std::sync::Arc<[engine::scene::Command]> {
        std::sync::Arc::from([solid_rect(0.0)])
    }

    fn picture_tree(
        offset_y: f32,
        generation: u64,
        commands: std::sync::Arc<[engine::scene::Command]>,
    ) -> engine::scene::LayerTree {
        use engine::scene::{Layer, LayerId, LayerKind, Picture, PictureId};
        let id = PictureId(7);
        let picture = std::sync::Arc::new(Picture::new(id, generation, (3.0, 4.0), commands));
        engine::scene::LayerTree {
            layers: vec![
                Layer {
                    kind: LayerKind::Offset {
                        x: 0.0,
                        y: offset_y,
                    },
                    parent: None,
                    first_child: Some(LayerId(1)),
                    next: None,
                },
                Layer {
                    kind: LayerKind::Picture { picture: id },
                    parent: Some(LayerId(0)),
                    first_child: None,
                    next: None,
                },
            ],
            pictures: vec![picture],
            root: LayerId(0),
            epoch: generation,
        }
    }

    #[test]
    fn picture_reuse_across_a_mixed_edit_sequence() {
        let Some(mut engine) = opened() else {
            return;
        };
        let commands = scrolled_commands();
        let tree = many_pictures(5000, 1, 0.0, commands.clone());
        engine
            .submit_layers(&tree, 1.0, |_| None)
            .expect("first submit");
        let built = engine.picture_builds();
        let moved = many_pictures(5000, 1, 4.0, commands.clone());
        engine.submit_layers(&moved, 1.0, |_| None).expect("offset");
        assert_eq!(engine.picture_builds(), built);
        engine.submit_layers(&moved, 2.0, |_| None).expect("scale");
        assert_eq!(engine.picture_builds(), built);
        let mut bumped = many_pictures(5000, 1, 4.0, commands);
        bumped.pictures[0] = std::sync::Arc::new(engine::scene::Picture::new(
            bumped.pictures[0].id,
            2,
            bumped.pictures[0].origin,
            std::sync::Arc::clone(bumped.pictures[0].commands()),
        ));
        engine
            .submit_layers(&bumped, 2.0, |_| None)
            .expect("generation");
        assert_eq!(engine.picture_builds(), built + 1);
    }

    fn many_pictures(
        count: u32,
        generation: u64,
        offset_y: f32,
        commands: std::sync::Arc<[engine::scene::Command]>,
    ) -> engine::scene::LayerTree {
        use engine::scene::{Layer, LayerId, LayerKind, Picture, PictureId};
        let mut layers = Vec::with_capacity(count as usize + 1);
        let mut pictures = Vec::with_capacity(count as usize);
        layers.push(Layer {
            kind: LayerKind::Offset {
                x: 0.0,
                y: offset_y,
            },
            parent: None,
            first_child: (count > 0).then_some(LayerId(1)),
            next: None,
        });
        for index in 0..count {
            let id = PictureId(u64::from(index) + 1);
            pictures.push(std::sync::Arc::new(Picture::new(
                id,
                generation,
                (0.0, 0.0),
                std::sync::Arc::clone(&commands),
            )));
            let layer_index = index + 1;
            layers.push(Layer {
                kind: LayerKind::Picture { picture: id },
                parent: Some(LayerId(0)),
                first_child: None,
                next: (layer_index < count).then_some(LayerId(layer_index + 1)),
            });
        }
        engine::scene::LayerTree {
            layers,
            pictures,
            root: LayerId(0),
            epoch: generation,
        }
    }

    #[test]
    fn canvas_paths_submit_and_a_missing_image_does_not_fail() {
        let Some(mut engine) = opened() else {
            return;
        };
        let list = DrawList {
            commands: vec![
                canvas_command(0.0, 0.0, 0.0, 10.0, engine::scene::CanvasList::empty()),
                canvas_command(1.0, 2.0, 20.0, 16.0, shared_canvas()),
                canvas_command(0.0, 0.0, 12.0, 12.0, isolated_canvas()),
                canvas_command(0.0, 0.0, 8.0, 8.0, glyph_canvas()),
            ],
        };
        assert!(engine.paint(&list, 2.0, |_| None).is_ok());
        assert_eq!(engine.cached_textures(), 0);
        let Ok(font) = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf") else {
            return;
        };
        if engine.register_font(&font, "Sans").is_err() {
            return;
        }
        let text = DrawList {
            commands: vec![canvas_command(0.0, 0.0, 40.0, 24.0, text_canvas())],
        };
        assert!(engine.paint(&text, 1.0, |_| None).is_ok());
    }

    #[test]
    fn canvas_picture_rebuilds_when_a_waiting_image_is_cached() {
        let Some(mut engine) = opened() else {
            return;
        };
        let commands = std::sync::Arc::from([canvas_command(0.0, 0.0, 16.0, 16.0, glyph_canvas())]);
        let tree = picture_tree(0.0, 1, std::sync::Arc::clone(&commands));
        assert!(engine.submit_layers(&tree, 1.0, |_| None).is_ok());
        assert_eq!(engine.picture_builds(), 1);
        assert_eq!(engine.cached_textures(), 0);
        assert!(engine.submit_layers(&tree, 1.0, |_| None).is_ok());
        assert_eq!(engine.picture_builds(), 1);

        let rgba = [9u8, 8, 7, 255, 1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255];
        let image = std::sync::Arc::from([engine::scene::Command::Image {
            x: 0.0,
            y: 0.0,
            w: 2.0,
            h: 2.0,
            source: std::sync::Arc::from("asset:glyph"),
            mode: engine::scene::ContentMode::Fill,
            required: true,
        }]);
        let both = image_then_canvas(image, std::sync::Arc::clone(&commands));
        let mut calls = 0u32;
        assert!(
            engine
                .submit_layers(&both, 1.0, |_| {
                    calls += 1;
                    Some(ImagePixels {
                        width: 2,
                        height: 2,
                        rgba: &rgba,
                    })
                })
                .is_ok()
        );
        assert_eq!(calls, 1);
        assert_eq!(engine.picture_builds(), 3);
        assert_eq!(engine.cached_textures(), 1);

        assert!(
            engine
                .submit_layers(&tree, 1.0, |_| {
                    calls += 1;
                    None
                })
                .is_ok()
        );
        assert_eq!(engine.picture_builds(), 3);
        assert_eq!(calls, 1);
        assert_eq!(engine.cached_textures(), 1);
    }

    fn canvas_command(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        list: engine::scene::CanvasList,
    ) -> engine::scene::Command {
        engine::scene::Command::Canvas {
            x,
            y,
            width,
            height,
            list,
        }
    }

    fn canvas_list(
        ops: Vec<engine::scene::CanvasOp>,
        images: Vec<engine::scene::CanvasImageRef>,
        isolation: engine::scene::CanvasIsolation,
    ) -> engine::scene::CanvasList {
        engine::scene::CanvasList {
            body: std::sync::Arc::new(engine::scene::CanvasBody {
                ops: ops.into_boxed_slice(),
                images: images.into_boxed_slice(),
                isolation,
                bytes: 1,
            }),
        }
    }

    fn sample_path() -> engine::scene::CanvasPath {
        engine::scene::CanvasPath {
            verbs: std::sync::Arc::from([
                engine::scene::PathVerb::Move(1.0, 1.0),
                engine::scene::PathVerb::Line(8.0, 1.0),
                engine::scene::PathVerb::Quad(9.0, 2.0, 8.0, 6.0),
                engine::scene::PathVerb::Cubic(4.0, 8.0, 2.0, 8.0, 1.0, 6.0),
                engine::scene::PathVerb::Close,
            ]),
            fill: engine::scene::FillRule::EvenOdd,
        }
    }

    fn stops() -> std::sync::Arc<[engine::scene::ColorStop]> {
        std::sync::Arc::from([
            engine::scene::ColorStop::srgb(0.0, [255, 0, 0, 255]),
            engine::scene::ColorStop::srgb(1.0, [0, 0, 255, 128]),
        ])
    }

    fn shared_canvas() -> engine::scene::CanvasList {
        let path = sample_path();
        canvas_list(
            vec![
                engine::scene::CanvasOp::Save,
                engine::scene::CanvasOp::Clip { path: path.clone() },
                engine::scene::CanvasOp::Fill {
                    path,
                    paint: engine::scene::CanvasPaint {
                        brush: engine::scene::CanvasBrush::Color([9, 8, 7, 255]),
                        alpha: 200,
                        composite: engine::scene::CanvasComposite::SourceOver,
                    },
                },
                engine::scene::CanvasOp::Restore,
                engine::scene::CanvasOp::Save,
            ],
            Vec::new(),
            engine::scene::CanvasIsolation::Shared,
        )
    }

    fn isolated_canvas() -> engine::scene::CanvasList {
        let path = sample_path();
        let stops = stops();
        canvas_list(
            vec![
                engine::scene::CanvasOp::Clear {
                    x: 1.0,
                    y: 1.0,
                    w: 4.0,
                    h: -4.0,
                    ctm: [0.0, 1.0, -1.0, 0.0, 2.0, 3.0],
                },
                engine::scene::CanvasOp::Fill {
                    path: path.clone(),
                    paint: engine::scene::CanvasPaint {
                        brush: engine::scene::CanvasBrush::Linear {
                            start: [0.0, 0.0],
                            end: [10.0, 0.0],
                            stops: std::sync::Arc::clone(&stops),
                        },
                        alpha: 255,
                        composite: engine::scene::CanvasComposite::Multiply,
                    },
                },
                engine::scene::CanvasOp::Fill {
                    path: path.clone(),
                    paint: engine::scene::CanvasPaint {
                        brush: engine::scene::CanvasBrush::Radial {
                            start: [4.0, 4.0],
                            start_radius: 0.0,
                            end: [4.0, 4.0],
                            end_radius: 6.0,
                            stops: std::sync::Arc::clone(&stops),
                        },
                        alpha: 128,
                        composite: engine::scene::CanvasComposite::Screen,
                    },
                },
                engine::scene::CanvasOp::Fill {
                    path,
                    paint: engine::scene::CanvasPaint {
                        brush: engine::scene::CanvasBrush::Conic {
                            center: [4.0, 4.0],
                            angle: 0.5,
                            stops,
                        },
                        alpha: 255,
                        composite: engine::scene::CanvasComposite::Xor,
                    },
                },
            ],
            Vec::new(),
            engine::scene::CanvasIsolation::Isolated,
        )
    }

    fn glyph_canvas() -> engine::scene::CanvasList {
        let source: std::sync::Arc<str> = std::sync::Arc::from("asset:glyph");
        canvas_list(
            vec![engine::scene::CanvasOp::Image(
                engine::scene::CanvasImageOp {
                    source: std::sync::Arc::clone(&source),
                    src: Some([-2.0, 0.0, 6.0, 4.0]),
                    x: 1.0,
                    y: 2.0,
                    w: Some(8.0),
                    h: None,
                    smoothing: false,
                    alpha: 255,
                    composite: engine::scene::CanvasComposite::DestinationOut,
                    ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                },
            )],
            vec![engine::scene::CanvasImageRef { source }],
            engine::scene::CanvasIsolation::Isolated,
        )
    }

    fn text_canvas() -> engine::scene::CanvasList {
        canvas_list(
            vec![engine::scene::CanvasOp::Text(engine::scene::CanvasTextOp {
                x: 1.0,
                y: 2.0,
                anchor_x: 4.0,
                utf8: std::sync::Arc::from("Hi"),
                font_size: 16.0,
                max_width: 0.0,
                compress: 0.5,
                ctm: [1.0, 0.0, 0.0, 1.0, 1.0, 2.0],
                style: std::sync::Arc::new(engine::scene::TextStyle {
                    slant: engine::scene::FontSlant::Italic,
                    ..engine::scene::TextStyle::default()
                }),
                paint: engine::scene::CanvasPaint {
                    brush: engine::scene::CanvasBrush::Color([0, 0, 0, 220]),
                    alpha: 255,
                    composite: engine::scene::CanvasComposite::SourceOver,
                },
            })],
            Vec::new(),
            engine::scene::CanvasIsolation::Shared,
        )
    }

    fn image_then_canvas(
        image: std::sync::Arc<[engine::scene::Command]>,
        canvas: std::sync::Arc<[engine::scene::Command]>,
    ) -> engine::scene::LayerTree {
        use engine::scene::{Layer, LayerId, LayerKind, Picture, PictureId};
        let image_id = PictureId(8);
        let canvas_id = PictureId(7);
        engine::scene::LayerTree {
            layers: vec![
                Layer {
                    kind: LayerKind::Offset { x: 0.0, y: 0.0 },
                    parent: None,
                    first_child: Some(LayerId(1)),
                    next: None,
                },
                Layer {
                    kind: LayerKind::Picture { picture: image_id },
                    parent: Some(LayerId(0)),
                    first_child: None,
                    next: Some(LayerId(2)),
                },
                Layer {
                    kind: LayerKind::Picture { picture: canvas_id },
                    parent: Some(LayerId(0)),
                    first_child: None,
                    next: None,
                },
            ],
            pictures: vec![
                std::sync::Arc::new(Picture::new(image_id, 1, (0.0, 0.0), image)),
                std::sync::Arc::new(Picture::new(canvas_id, 1, (0.0, 0.0), canvas)),
            ],
            root: LayerId(0),
            epoch: 1,
        }
    }
}
