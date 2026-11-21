//! Vulkan context and swapchain. Compiled only for Android.

use std::cell::Cell;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicPtr, AtomicU32, Ordering};

use crate::{PixelSize, Status, VulkanPhase};

const RTLD_NOW: i32 = 2;
const VK_ERROR_DEVICE_LOST: i32 = -4;
const VK_ERROR_SURFACE_LOST_KHR: i32 = -1_000_000_000;
const VK_ERROR_OUT_OF_DATE_KHR: i32 = -1_000_001_004;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
}

/// Android `VkStructureType` for `VkAndroidSurfaceCreateInfoKHR`.
const ANDROID_SURFACE_CREATE_INFO: i32 = 1000008000;

#[repr(C)]
struct AndroidSurfaceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    window: *mut c_void,
}

type CreateAndroidSurface = unsafe extern "C" fn(
    instance: *mut c_void,
    info: *const AndroidSurfaceCreateInfo,
    allocator: *const c_void,
    surface: *mut u64,
) -> i32;

/// `VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR`.
const SWAPCHAIN_CREATE_INFO: i32 = 1000001000;

/// Prefix of `VkSwapchainCreateInfoKHR` through `imageExtent`. The standalone
/// SDK asks for a 1×1 chain, and the Android implementation never replaces that
/// with `currentExtent`. The fields below match the C layout on aarch64.
#[repr(C)]
struct SwapchainCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    surface: u64,
    min_image_count: u32,
    image_format: i32,
    image_color_space: i32,
    extent_width: u32,
    extent_height: u32,
}

type CreateSwapchain = unsafe extern "C" fn(
    device: *mut c_void,
    info: *mut SwapchainCreateInfo,
    allocator: *const c_void,
    swapchain: *mut u64,
) -> i32;

type GetDeviceProc = unsafe extern "C" fn(device: *mut c_void, name: *const c_char) -> *mut c_void;
type AcquireNext = unsafe extern "C" fn(
    device: *mut c_void,
    swapchain: u64,
    timeout: u64,
    semaphore: u64,
    fence: u64,
    image_index: *mut u32,
) -> i32;
type QueuePresent = unsafe extern "C" fn(queue: *mut c_void, info: *const c_void) -> i32;

/// Proc table and swapchain extent for one Vulkan context.
///
/// `ImpellerContextCreateVulkanNew` stores this pointer as `user_data`.
/// `vkCreateSwapchainKHR` has no user-data argument, so the render thread
/// publishes this pointer on the calling thread for the duration of create
/// and acquire. A second engine does not see this extent.
pub(super) struct VulkanProcs {
    library: AtomicPtr<c_void>,
    extent_width: AtomicU32,
    extent_height: AtomicU32,
    real_create: AtomicPtr<c_void>,
    real_get_device: AtomicPtr<c_void>,
    real_acquire: AtomicPtr<c_void>,
    real_present: AtomicPtr<c_void>,
    last_window_status: AtomicI32,
}

impl VulkanProcs {
    fn new(library: *mut c_void) -> Self {
        Self {
            library: AtomicPtr::new(library),
            extent_width: AtomicU32::new(0),
            extent_height: AtomicU32::new(0),
            real_create: AtomicPtr::new(ptr::null_mut()),
            real_get_device: AtomicPtr::new(ptr::null_mut()),
            real_acquire: AtomicPtr::new(ptr::null_mut()),
            real_present: AtomicPtr::new(ptr::null_mut()),
            last_window_status: AtomicI32::new(i32::MIN),
        }
    }

    fn set_extent(&self, size: PixelSize) {
        if !size.is_usable() {
            return;
        }
        self.extent_width.store(size.width, Ordering::Release);
        self.extent_height.store(size.height, Ordering::Release);
    }

    pub(super) fn current(&self) -> CurrentProcs<'_> {
        let previous = CURRENT_PROCS.with(|cell| cell.replace(ptr::from_ref(self)));
        CurrentProcs {
            previous,
            _procs: self,
        }
    }

    fn window_result(&self, code: i32) -> i32 {
        if window_status(code).is_some() {
            // VkResult values order these failures from outdated to device lost.
            self.last_window_status.fetch_max(code, Ordering::Relaxed);
        }
        code
    }

    fn take_window_status(&self) -> Option<Status> {
        window_status(self.last_window_status.swap(i32::MIN, Ordering::Relaxed))
    }
}

fn window_status(code: i32) -> Option<Status> {
    match code {
        VK_ERROR_DEVICE_LOST => Some(Status::DeviceLost),
        VK_ERROR_SURFACE_LOST_KHR => Some(Status::SurfaceLost),
        VK_ERROR_OUT_OF_DATE_KHR => Some(Status::AcquireMiss),
        _ => None,
    }
}

impl Drop for VulkanProcs {
    fn drop(&mut self) {
        // SAFETY: this table owns the loader handle. Context and resource owners
        // each keep the table alive until after they release their Impeller handle.
        unsafe { dlclose(*self.library.get_mut()) };
    }
}

thread_local! {
    static CURRENT_PROCS: Cell<*const VulkanProcs> = const { Cell::new(ptr::null()) };
}

/// Publishes `procs` to the swapchain hook for this thread, then restores the
/// previous pointer. The hook has no user-data argument of its own.
pub(super) struct CurrentProcs<'a> {
    previous: *const VulkanProcs,
    _procs: &'a VulkanProcs,
}

impl Drop for CurrentProcs<'_> {
    fn drop(&mut self) {
        CURRENT_PROCS.with(|cell| cell.set(self.previous));
    }
}

fn current_procs() -> *const VulkanProcs {
    CURRENT_PROCS.with(|cell| cell.get())
}

pub struct VulkanSwapchain {
    raw: impeller::ImpellerVulkanSwapchain,
    extent: PixelSize,
    procs: Arc<VulkanProcs>,
}

impl VulkanSwapchain {
    pub fn extent(&self) -> PixelSize {
        self.extent
    }

    /// Remembers the drawable size. The next acquire copies it into this
    /// engine's proc table. A non-usable size leaves the previous extent.
    pub fn set_extent(&mut self, size: PixelSize) {
        if size.is_usable() {
            self.extent = size;
        }
    }
}

impl Drop for VulkanSwapchain {
    fn drop(&mut self) {
        let _guard = self.procs.current();
        // SAFETY: `raw` came from `ImpellerVulkanSwapchainCreateNew`. Release is a
        // no-op on null and also releases the `VkSurfaceKHR` Impeller took ownership of.
        unsafe { impeller::ImpellerVulkanSwapchainRelease(self.raw) };
        self.raw = ptr::null_mut();
    }
}

impl super::Engine {
    pub fn vulkan_has_safe_shutdown() -> bool {
        // SAFETY: libimpeller is already linked by this crate. The lookup only
        // checks whether the Android SDK includes Impasto's standalone fix.
        unsafe {
            libloading::Library::new("libimpeller.so").is_ok_and(|library| {
                library
                    .get::<unsafe extern "C" fn() -> bool>(b"ImpellerHasStandaloneVulkanShutdown")
                    .is_ok()
            })
        }
    }

    pub fn create_vulkan() -> Result<Self, Status> {
        if !super::impeller_version_matches() {
            return Err(Status::NotLinked);
        }
        // SAFETY: `libvulkan.so` is the platform loader. The handle stays in
        // `VulkanProcs` until its last context or resource owner releases it.
        let vulkan_lib = unsafe { dlopen(c"libvulkan.so".as_ptr(), RTLD_NOW) };
        if vulkan_lib.is_null() {
            return Err(Status::NotLinked);
        }
        let procs = Arc::new(VulkanProcs::new(vulkan_lib));
        let settings = impeller::ImpellerContextVulkanSettings {
            user_data: Arc::as_ptr(&procs).cast_mut().cast(),
            proc_address_callback: Some(proc_address),
            enable_vulkan_validation: false,
        };
        // SAFETY: `settings.proc_address_callback` points at `proc_address`, and
        // `user_data` points to the shared `VulkanProcs`. Impeller only reads it.
        // The guard publishes that pointer for hooks that have no user data.
        // A null context means this SDK cannot create Vulkan. Release is a no-op on null.
        let guard = procs.current();
        let context = unsafe {
            impeller::ImpellerContextCreateVulkanNew(impeller::IMPELLER_VERSION, &settings)
        };
        let typography = unsafe { impeller::ImpellerTypographyContextNew() };
        if context.is_null() || typography.is_null() {
            unsafe {
                impeller::ImpellerTypographyContextRelease(typography);
                impeller::ImpellerContextRelease(context);
            }
            return Err(Status::NotLinked);
        }
        drop(guard);
        let mut engine = Self::blank(context, typography);
        engine.vulkan_procs = Some(procs);
        Ok(engine)
    }

    /// Builds a swapchain for `native_window` at `pixels`. The caller drops it
    /// when the window is gone, the buffer changes size, or acquire cannot recover.
    pub fn create_swapchain(
        &self,
        native_window: *mut c_void,
        pixels: PixelSize,
    ) -> Result<VulkanSwapchain, Status> {
        let Some(procs) = &self.vulkan_procs else {
            return Err(Status::NotReady);
        };
        if native_window.is_null() {
            return Err(Status::NotReady);
        }
        procs.set_extent(pixels);
        let mut info = impeller::ImpellerContextVulkanInfo {
            vk_instance: ptr::null_mut(),
            vk_physical_device: ptr::null_mut(),
            vk_logical_device: ptr::null_mut(),
            graphics_queue_family_index: 0,
            graphics_queue_index: 0,
        };
        // SAFETY: `info` is a writable out-parameter. The context is the Vulkan
        // context created for this engine. Impeller keeps ownership of the handles.
        let found = unsafe { impeller::ImpellerContextGetVulkanInfo(self.context, &mut info) };
        if !found || info.vk_instance.is_null() {
            return Err(Status::NotReady);
        }
        let library = procs.library.load(Ordering::Relaxed);
        let create = proc_symbol::<CreateAndroidSurface>(
            info.vk_instance,
            library,
            c"vkCreateAndroidSurfaceKHR".as_ptr(),
        );
        let Some(create) = create else {
            return Err(Status::NotReady);
        };
        let create_info = AndroidSurfaceCreateInfo {
            s_type: ANDROID_SURFACE_CREATE_INFO,
            p_next: ptr::null(),
            flags: 0,
            window: native_window,
        };
        let mut surface_khr = 0u64;
        // SAFETY: `create_info.window` is the `ANativeWindow` the platform thread
        // keeps alive until `SurfaceDropped`. The instance is Impeller's.
        let result = unsafe {
            create(
                info.vk_instance,
                &create_info,
                ptr::null(),
                &mut surface_khr,
            )
        };
        if result != 0 || surface_khr == 0 {
            return Err(window_status(result).unwrap_or(Status::NotReady));
        }
        // SAFETY: the Vulkan interop constructor takes ownership of `VkSurfaceKHR`.
        // Its unique surface wrapper also releases it if swapchain setup fails.
        // The guard lets the extent hook see this engine's proc table.
        let _guard = procs.current();
        procs.take_window_status();
        let raw = unsafe {
            impeller::ImpellerVulkanSwapchainCreateNew(
                self.context,
                surface_khr as usize as *mut c_void,
            )
        };
        if raw.is_null() {
            return Err(procs.take_window_status().unwrap_or(Status::NotReady));
        }
        Ok(VulkanSwapchain {
            raw,
            extent: pixels,
            procs: Arc::clone(procs),
        })
    }

    /// Acquires the next swapchain image, draws the current display list, and presents.
    /// A null acquire is [`Status::AcquireMiss`]: skip this present. The caller
    /// keeps the chain until consecutive misses ask it to rebuild.
    /// No display list is [`Status::NotReady`].
    pub fn draw_vulkan(&mut self, swapchain: &VulkanSwapchain) -> Result<(), Status> {
        self.draw_vulkan_with_trace(swapchain, |_| ())
    }

    /// Calls `section` before each Vulkan phase and drops its guard on return.
    pub fn draw_vulkan_with_trace<G>(
        &mut self,
        swapchain: &VulkanSwapchain,
        mut section: impl FnMut(VulkanPhase) -> G,
    ) -> Result<(), Status> {
        if self.context.is_null()
            || self.list.is_null()
            || self
                .vulkan_procs
                .as_ref()
                .is_none_or(|procs| !Arc::ptr_eq(procs, &swapchain.procs))
        {
            return Err(Status::NotReady);
        }
        if swapchain.raw.is_null() {
            return Err(Status::AcquireMiss);
        }
        swapchain.procs.set_extent(swapchain.extent);
        // SAFETY: `swapchain.raw` is a live swapchain. Acquire may block until the
        // next image. The returned surface is released before this function returns.
        // The guard publishes this engine's extent to the swapchain-size hook.
        let _guard = swapchain.procs.current();
        swapchain.procs.take_window_status();
        let surface = {
            let _phase = section(VulkanPhase::Acquire);
            unsafe { impeller::ImpellerVulkanSwapchainAcquireNextSurfaceNew(swapchain.raw) }
        };
        if surface.is_null() {
            return Err(swapchain
                .procs
                .take_window_status()
                .unwrap_or(Status::AcquireMiss));
        }
        let presented = unsafe {
            let drew = {
                let _phase = section(VulkanPhase::Draw);
                impeller::ImpellerSurfaceDrawDisplayList(surface, self.list)
            };
            let presented = drew && {
                let _phase = section(VulkanPhase::Present);
                impeller::ImpellerSurfacePresent(surface)
            };
            impeller::ImpellerSurfaceRelease(surface);
            presented
        };
        let status = swapchain.procs.take_window_status();
        if matches!(status, Some(Status::DeviceLost | Status::SurfaceLost)) {
            Err(status.unwrap_or(Status::PresentFailed))
        } else if presented {
            Ok(())
        } else {
            Err(status.unwrap_or(Status::PresentFailed))
        }
    }
}

fn proc_symbol<T>(instance: *mut c_void, library: *mut c_void, name: *const c_char) -> Option<T> {
    // SAFETY: `name` is a NUL-terminated literal. `library` is the `libvulkan.so` handle.
    // This lookup does not install the swapchain hook; it only resolves a symbol.
    let address = unsafe { resolve_proc(instance, name, library) };
    if address.is_null() {
        None
    } else {
        // SAFETY: `address` is the Vulkan entry point named `name` from this instance.
        // `T` is the matching function-pointer type at the call site.
        Some(unsafe { std::mem::transmute_copy(&address) })
    }
}

unsafe extern "C" fn proc_address(
    instance: *mut c_void,
    name: *const c_char,
    user_data: *mut c_void,
) -> *mut c_void {
    if user_data.is_null() || name.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `user_data` is the `VulkanProcs` passed to `ImpellerContextCreateVulkanNew`.
    let procs = unsafe { &*(user_data.cast::<VulkanProcs>()) };
    // SAFETY: Vulkan passes a NUL-terminated entry-point name.
    let requested = unsafe { CStr::from_ptr(name) };
    let resolved = unsafe { resolve_proc(instance, name, procs.library.load(Ordering::Relaxed)) };
    if resolved.is_null() {
        return ptr::null_mut();
    }
    if requested == c"vkGetDeviceProcAddr" {
        procs.real_get_device.store(resolved, Ordering::Release);
        return get_device_proc as *mut c_void;
    }
    intercept_window_proc(procs, requested, resolved)
}

fn intercept_window_proc(procs: &VulkanProcs, name: &CStr, original: *mut c_void) -> *mut c_void {
    if original.is_null() {
        return original;
    }
    let (slot, shim) = match name {
        name if name == c"vkCreateSwapchainKHR" => {
            (&procs.real_create, create_swapchain as *mut c_void)
        }
        name if name == c"vkAcquireNextImageKHR" => {
            (&procs.real_acquire, acquire_next as *mut c_void)
        }
        name if name == c"vkQueuePresentKHR" => (&procs.real_present, queue_present as *mut c_void),
        _ => return original,
    };
    slot.store(original, Ordering::Release);
    shim
}

unsafe fn resolve_proc(
    instance: *mut c_void,
    name: *const c_char,
    library: *mut c_void,
) -> *mut c_void {
    if library.is_null() || name.is_null() {
        return ptr::null_mut();
    }
    if instance.is_null() {
        return unsafe { dlsym(library, name) };
    }
    let get_instance = unsafe { dlsym(library, c"vkGetInstanceProcAddr".as_ptr()) };
    if get_instance.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `vkGetInstanceProcAddr` is the loader entry point from `libvulkan.so`.
    let get_instance: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
        unsafe { std::mem::transmute(get_instance) };
    unsafe { get_instance(instance, name) }
}

unsafe extern "C" fn get_device_proc(device: *mut c_void, name: *const c_char) -> *mut c_void {
    let procs = current_procs();
    if procs.is_null() || name.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `procs` was published by `CurrentProcs` for this call and outlives it.
    let get_device = unsafe { (*procs).real_get_device.load(Ordering::Acquire) };
    if get_device.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `get_device` is the real `vkGetDeviceProcAddr` stored by `proc_address`.
    let get_device: GetDeviceProc = unsafe { std::mem::transmute(get_device) };
    let resolved = unsafe { get_device(device, name) };
    // SAFETY: Vulkan passes a NUL-terminated entry-point name.
    let requested = unsafe { CStr::from_ptr(name) };
    // SAFETY: the active calling thread owns this table through CurrentProcs.
    intercept_window_proc(unsafe { &*procs }, requested, resolved)
}

unsafe extern "C" fn create_swapchain(
    device: *mut c_void,
    info: *mut SwapchainCreateInfo,
    allocator: *const c_void,
    swapchain: *mut u64,
) -> i32 {
    let procs = current_procs();
    if procs.is_null() {
        return -1;
    }
    // SAFETY: `procs` was published by `CurrentProcs` for this call and outlives it.
    let create = unsafe { (*procs).real_create.load(Ordering::Acquire) };
    if create.is_null() {
        return -1;
    }
    let width = unsafe { (*procs).extent_width.load(Ordering::Acquire) };
    let height = unsafe { (*procs).extent_height.load(Ordering::Acquire) };
    // SAFETY: `info` is the `VkSwapchainCreateInfoKHR` Impeller built on the stack.
    // Only the extent fields are written, and only when the structure type matches.
    // The SDK asks for 1×1. Android does not replace that with the window size.
    let mut previous = None;
    if !info.is_null() && width > 1 && height > 1 {
        let header = unsafe { &mut *info };
        if header.s_type == SWAPCHAIN_CREATE_INFO
            && (header.extent_width != width || header.extent_height != height)
        {
            previous = Some((header.extent_width, header.extent_height));
            header.extent_width = width;
            header.extent_height = height;
        }
    }
    // SAFETY: `create` is the real `vkCreateSwapchainKHR` stored before this hook
    // was returned. `info` still points at Impeller's create-info.
    let create: CreateSwapchain = unsafe { std::mem::transmute(create) };
    let result = unsafe { create(device, info, allocator, swapchain) };
    // SAFETY: the active calling thread owns this table through CurrentProcs.
    let procs = unsafe { &*procs };
    let Some((previous_width, previous_height)) = previous else {
        return procs.window_result(result);
    };
    if result == 0 || window_status(result).is_some() {
        return procs.window_result(result);
    }
    // The window size was rejected. Restore the extent Impeller chose so the
    // chain still comes up, and leave a log for the size that was refused.
    let header = unsafe { &mut *info };
    header.extent_width = previous_width;
    header.extent_height = previous_height;
    if !swapchain.is_null() {
        // SAFETY: this non-null out parameter is writable for the whole Vulkan call.
        unsafe { *swapchain = 0 };
    }
    log_swapchain(width, height, result);
    // SAFETY: the original create-info and out parameter remain live for this retry.
    let retry = unsafe { create(device, info, allocator, swapchain) };
    procs.window_result(retry)
}

unsafe extern "C" fn acquire_next(
    device: *mut c_void,
    swapchain: u64,
    timeout: u64,
    semaphore: u64,
    fence: u64,
    index: *mut u32,
) -> i32 {
    // SAFETY: the WSI call is on the render thread with CurrentProcs active.
    let Some(procs) = (unsafe { current_procs().as_ref() }) else {
        return -1;
    };
    let original = procs.real_acquire.load(Ordering::Acquire);
    if original.is_null() {
        return -1;
    }
    // SAFETY: the loader supplied vkAcquireNextImageKHR; the arguments are forwarded unchanged.
    let acquire: AcquireNext = unsafe { std::mem::transmute(original) };
    // SAFETY: the handles and output pointer came from the caller's Vulkan command.
    let result = unsafe { acquire(device, swapchain, timeout, semaphore, fence, index) };
    procs.window_result(result)
}

unsafe extern "C" fn queue_present(queue: *mut c_void, info: *const c_void) -> i32 {
    // SAFETY: the WSI call is on the render thread with CurrentProcs active.
    let Some(procs) = (unsafe { current_procs().as_ref() }) else {
        return -1;
    };
    let original = procs.real_present.load(Ordering::Acquire);
    if original.is_null() {
        return -1;
    }
    // SAFETY: the loader supplied vkQueuePresentKHR; the arguments are forwarded unchanged.
    let present: QueuePresent = unsafe { std::mem::transmute(original) };
    // SAFETY: the queue and present info are the caller's unchanged Vulkan arguments.
    let result = unsafe { present(queue, info) };
    procs.window_result(result)
}

fn log_swapchain(width: u32, height: u32, result: i32) {
    log(
        6,
        &format!("swapchain extent {width}x{height} refused ({result})"),
    );
}

fn log(priority: i32, message: &str) {
    let mut bytes = message.replace('\0', " ").into_bytes();
    bytes.push(0);
    unsafe extern "C" {
        fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
    }
    // SAFETY: `bytes` is NUL-terminated and lives for this call. The tag is a literal.
    unsafe {
        __android_log_write(priority, c"impasto".as_ptr(), bytes.as_ptr().cast());
    }
}
