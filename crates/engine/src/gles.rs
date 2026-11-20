//! EGL context and native window surfaces. All context work stays on the render thread.

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::rc::Rc;
use std::sync::OnceLock;

use khronos_egl as egl;

use super::{Engine, Status, impeller_version_matches};
use crate::PixelSize;

type Egl = egl::DynamicInstance<egl::EGL1_4>;

fn egl_api() -> Result<&'static Egl, Status> {
    static EGL: OnceLock<Option<Egl>> = OnceLock::new();
    EGL.get_or_init(|| {
        // SAFETY: Android supplies libEGL.so with the Khronos EGL ABI.
        unsafe { Egl::load_required_from_filename("libEGL.so") }.ok()
    })
    .as_ref()
    .ok_or(Status::NotLinked)
}

unsafe extern "C" fn gl_proc(name: *const c_char, _data: *mut c_void) -> *mut c_void {
    // SAFETY: Impeller supplies the address of a NUL-terminated procedure name.
    let name = unsafe { CStr::from_ptr(name) };
    if let Ok(egl) = egl_api()
        && let Ok(name) = name.to_str()
        && let Some(function) = egl.get_proc_address(name)
    {
        return function as *mut c_void;
    }
    static GLES: OnceLock<Option<libloading::Library>> = OnceLock::new();
    // SAFETY: the Android NDK GLES loader comes from the platform. Impeller
    // calls the resulting address using the declared signature for that name.
    let library = GLES.get_or_init(|| unsafe { libloading::Library::new("libGLESv3.so") }.ok());
    library
        .as_ref()
        .and_then(|lib| {
            // SAFETY: this looks up a procedure by its C name. Rust never calls it.
            unsafe { lib.get::<unsafe extern "C" fn()>(name.to_bytes_with_nul()) }.ok()
        })
        .map_or(ptr::null_mut(), |symbol| *symbol as *mut c_void)
}

fn egl_error(operation: &'static str, error: egl::Error) -> Status {
    tracing::warn!(target: "impasto::render::gles", operation, %error, "EGL call failed");
    match error {
        egl::Error::ContextLost | egl::Error::BadContext => Status::DeviceLost,
        egl::Error::BadNativeWindow | egl::Error::BadCurrentSurface | egl::Error::BadSurface => {
            Status::SurfaceLost
        }
        _ => Status::PresentFailed,
    }
}

pub(super) struct GlesContext {
    api: &'static Egl,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    pbuffer: Option<egl::Surface>,
}

impl GlesContext {
    fn new() -> Result<Self, Status> {
        let api = egl_api()?;
        // SAFETY: the Android system display uses EGL_DEFAULT_DISPLAY.
        let display = unsafe { api.get_display(egl::DEFAULT_DISPLAY) }.ok_or(Status::NotLinked)?;
        // Android supplies one process-wide display, also used by the host UI.
        // Only release resources created by this context, leaving the display initialized.
        api.initialize(display)
            .map_err(|e| egl_error("initialize", e))?;
        api.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| egl_error("bind API", e))?;
        // Impeller wraps framebuffer 0 and uses the EGL surface's depth and
        // stencil attachments for clipping and path coverage.
        let config = api
            .choose_first_config(
                display,
                &[
                    egl::SURFACE_TYPE,
                    egl::WINDOW_BIT | egl::PBUFFER_BIT,
                    egl::RENDERABLE_TYPE,
                    egl::OPENGL_ES3_BIT,
                    egl::RED_SIZE,
                    8,
                    egl::GREEN_SIZE,
                    8,
                    egl::BLUE_SIZE,
                    8,
                    egl::ALPHA_SIZE,
                    8,
                    egl::DEPTH_SIZE,
                    24,
                    egl::STENCIL_SIZE,
                    8,
                    egl::NONE,
                ],
            )
            .map_err(|e| egl_error("choose config", e))?
            .ok_or(Status::NotLinked)?;
        let context = api
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE],
            )
            .map_err(|e| egl_error("create context", e))?;
        let mut state = Self {
            api,
            display,
            config,
            context,
            pbuffer: None,
        };
        let pbuffer = api
            .create_pbuffer_surface(display, config, &[egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE])
            .map_err(|e| egl_error("create pbuffer", e))?;
        state.pbuffer = Some(pbuffer);
        state.make_current(pbuffer)?;
        Ok(state)
    }

    fn make_current(&self, surface: egl::Surface) -> Result<(), Status> {
        self.api
            .make_current(
                self.display,
                Some(surface),
                Some(surface),
                Some(self.context),
            )
            .map_err(|e| egl_error("make current", e))
    }
}

impl Drop for GlesContext {
    fn drop(&mut self) {
        if self.api.get_current_context() == Some(self.context) {
            let _ = self.api.make_current(self.display, None, None, None);
        }
        if let Some(pbuffer) = self.pbuffer.take() {
            let _ = self.api.destroy_surface(self.display, pbuffer);
        }
        let _ = self.api.destroy_context(self.display, self.context);
    }
}

pub struct GlesWindow {
    owner: Rc<GlesContext>,
    surface: egl::Surface,
}

impl Drop for GlesWindow {
    fn drop(&mut self) {
        let owner = &self.owner;
        if owner.api.get_current_surface(egl::DRAW) == Some(self.surface)
            && let Some(pbuffer) = owner.pbuffer
            && owner.make_current(pbuffer).is_err()
        {
            let _ = owner.api.make_current(owner.display, None, None, None);
        }
        let _ = owner.api.destroy_surface(owner.display, self.surface);
    }
}

impl Engine {
    pub fn create_gles() -> Result<Self, Status> {
        if !impeller_version_matches() {
            return Err(Status::NotLinked);
        }
        let egl = Rc::new(GlesContext::new()?);
        // SAFETY: a compatible context is current on this thread. The resolver
        // uses process-lifetime libraries and does not consume the user pointer.
        let context = unsafe {
            impeller::ImpellerContextCreateOpenGLESNew(
                impeller::IMPELLER_VERSION,
                Some(gl_proc),
                ptr::null_mut(),
            )
        };
        if context.is_null() {
            return Err(Status::NotLinked);
        }
        // SAFETY: the constructors create handles owned by the engine.
        let typography = unsafe { impeller::ImpellerTypographyContextNew() };
        if typography.is_null() {
            // SAFETY: release on the owning EGL thread before dropping its context.
            unsafe { impeller::ImpellerContextRelease(context) };
            return Err(Status::NotLinked);
        }
        let mut engine = Self::blank(context, typography);
        engine.gles = Some(egl);
        Ok(engine)
    }

    pub fn is_gles(&self) -> bool {
        self.gles.is_some()
    }

    /// Switches a Vulkan engine before its first window opens. The session's
    /// shaper continues to use the same registered typography context.
    pub fn use_gles(&mut self) -> Result<(), Status> {
        if self.is_gles() || !self.list.is_null() {
            return Err(Status::NotReady);
        }
        self.recreate_graphics(Self::create_gles)
    }

    /// # Safety
    /// `window` must refer to an Android `ANativeWindow` kept alive until the
    /// resulting EGL surface has been dropped.
    pub unsafe fn create_gles_window(&self, window: *mut c_void) -> Result<GlesWindow, Status> {
        let owner = self.gles.as_ref().ok_or(Status::NotLinked)?;
        // SAFETY: the caller provides a native window for the Android display.
        let surface = unsafe {
            owner
                .api
                .create_window_surface(owner.display, owner.config, window, None)
        }
        .map_err(|e| egl_error("create window surface", e))?;
        let window = GlesWindow {
            owner: Rc::clone(owner),
            surface,
        };
        owner.make_current(surface)?;
        Ok(window)
    }

    pub fn draw_gles(&mut self, window: &GlesWindow, pixels: PixelSize) -> Result<(), Status> {
        let context = self.gles.as_ref().ok_or(Status::NotLinked)?;
        if !Rc::ptr_eq(context, &window.owner) || self.list.is_null() || !pixels.is_usable() {
            return Err(Status::NotReady);
        }
        context.make_current(window.surface)?;
        let size = impeller::ImpellerISize {
            width: pixels.width.into(),
            height: pixels.height.into(),
        };
        // SAFETY: the window owns the current EGL drawable so framebuffer 0
        // remains valid for the duration of this call.
        let surface = unsafe {
            impeller::ImpellerSurfaceCreateWrappedFBONew(
                self.context,
                0,
                impeller::ImpellerPixelFormat::kImpellerPixelFormatRGBA8888,
                &size,
            )
        };
        if surface.is_null() {
            return Err(Status::PresentFailed);
        }
        // SAFETY: both handles are live on their owning thread.
        let drawn = unsafe {
            let drawn = impeller::ImpellerSurfaceDrawDisplayList(surface, self.list);
            impeller::ImpellerSurfaceRelease(surface);
            drawn
        };
        if !drawn {
            return Err(Status::PresentFailed);
        }
        context
            .api
            .swap_buffers(context.display, window.surface)
            .map_err(|e| egl_error("swap buffers", e))
    }
}
