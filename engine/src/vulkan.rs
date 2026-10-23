//! Vulkan context and swapchain. Compiled only for Android.

use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use crate::Status;

use super::ffi;

const RTLD_NOW: i32 = 2;

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

type DestroySurface =
    unsafe extern "C" fn(instance: *mut c_void, surface: u64, allocator: *const c_void);

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

static DESIRED_WIDTH: AtomicU32 = AtomicU32::new(0);
static DESIRED_HEIGHT: AtomicU32 = AtomicU32::new(0);
static REAL_CREATE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static REAL_GET_DEVICE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

/// Pixel size of the `ANativeWindow` the next `vkCreateSwapchainKHR` should use.
///
/// The render thread is the only caller. The Vulkan proc callback has no other
/// place to carry this size: it was fixed to the `libvulkan.so` handle when the
/// context was created.
pub fn note_swapchain_extent(width: u32, height: u32) {
    DESIRED_WIDTH.store(width, Ordering::SeqCst);
    DESIRED_HEIGHT.store(height, Ordering::SeqCst);
}

pub struct VulkanSwapchain {
    raw: ffi::ImpellerVulkanSwapchain,
}

impl Drop for VulkanSwapchain {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `ImpellerVulkanSwapchainCreateNew`. Release is a
        // no-op on null and also releases the `VkSurfaceKHR` Impeller took ownership of.
        unsafe { ffi::ImpellerVulkanSwapchainRelease(self.raw) };
        self.raw = ptr::null_mut();
    }
}

pub(super) fn close_library(handle: *mut c_void) {
    // SAFETY: `handle` is the `dlopen` handle from `create_vulkan`, and the Vulkan
    // context that used it has already been released.
    unsafe { dlclose(handle) };
}

impl super::Engine {
    pub fn create_vulkan() -> Result<Self, Status> {
        // SAFETY: `libvulkan.so` is the platform loader. The handle stays in
        // `vulkan_lib` until this engine is dropped, after the context is released.
        let vulkan_lib = unsafe { dlopen(c"libvulkan.so".as_ptr(), RTLD_NOW) };
        if vulkan_lib.is_null() {
            return Err(Status::NotLinked);
        }
        let settings = ffi::ImpellerContextVulkanSettings {
            user_data: vulkan_lib,
            proc_address_callback: Some(proc_address),
            enable_vulkan_validation: false,
        };
        // SAFETY: `settings.proc_address_callback` points at `proc_address`, and
        // `user_data` is the library handle kept alive by this engine. A null
        // context means this SDK cannot create Vulkan. Release is a no-op on null.
        let (context, typography) = unsafe {
            let context = ffi::ImpellerContextCreateVulkanNew(ffi::IMPELLER_VERSION, &settings);
            let typography = ffi::ImpellerTypographyContextNew();
            if context.is_null() || typography.is_null() {
                ffi::ImpellerTypographyContextRelease(typography);
                ffi::ImpellerContextRelease(context);
                dlclose(vulkan_lib);
                return Err(Status::NotLinked);
            }
            (context, typography)
        };
        let mut engine = Self::blank(context, typography);
        engine.vulkan_lib = vulkan_lib;
        Ok(engine)
    }

    /// Builds a swapchain for `native_window`. Size changes keep this chain.
    /// The caller drops it when the window itself is gone or acquire cannot recover.
    pub fn create_swapchain(&self, native_window: *mut c_void) -> Result<VulkanSwapchain, Status> {
        if native_window.is_null() || self.vulkan_lib.is_null() {
            return Err(Status::NotReady);
        }
        let mut info = ffi::ImpellerContextVulkanInfo {
            vk_instance: ptr::null_mut(),
            vk_physical_device: ptr::null_mut(),
            vk_logical_device: ptr::null_mut(),
            graphics_queue_family_index: 0,
            graphics_queue_index: 0,
        };
        // SAFETY: `info` is a writable out-parameter. The context is the Vulkan
        // context created for this engine. Impeller keeps ownership of the handles.
        let found = unsafe { ffi::ImpellerContextGetVulkanInfo(self.context, &mut info) };
        if !found || info.vk_instance.is_null() {
            return Err(Status::NotReady);
        }
        let create = proc_symbol::<CreateAndroidSurface>(
            info.vk_instance,
            self.vulkan_lib,
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
            return Err(Status::NotReady);
        }
        // SAFETY: a successful create transfers the `VkSurfaceKHR` to Impeller.
        // On failure the surface is destroyed here so it is not leaked.
        let raw = unsafe {
            ffi::ImpellerVulkanSwapchainCreateNew(self.context, surface_khr as usize as *mut c_void)
        };
        if raw.is_null() {
            if let Some(destroy) = proc_symbol::<DestroySurface>(
                info.vk_instance,
                self.vulkan_lib,
                c"vkDestroySurfaceKHR".as_ptr(),
            ) {
                unsafe { destroy(info.vk_instance, surface_khr, ptr::null()) };
            }
            return Err(Status::NotReady);
        }
        Ok(VulkanSwapchain { raw })
    }

    /// Acquires the next swapchain image, draws the current display list, and presents.
    /// A null acquire is [`Status::NotReady`]: the caller drops the swapchain.
    pub fn draw_vulkan(&mut self, swapchain: &VulkanSwapchain) -> Result<(), Status> {
        if self.list.is_null() || swapchain.raw.is_null() {
            return Err(Status::NotReady);
        }
        // SAFETY: `swapchain.raw` is a live swapchain. Acquire may block until the
        // next image. The returned surface is released before this function returns.
        let surface = unsafe { ffi::ImpellerVulkanSwapchainAcquireNextSurfaceNew(swapchain.raw) };
        if surface.is_null() {
            return Err(Status::NotReady);
        }
        let presented = unsafe {
            let drew = ffi::ImpellerSurfaceDrawDisplayList(surface, self.list);
            let presented = drew && ffi::ImpellerSurfacePresent(surface);
            ffi::ImpellerSurfaceRelease(surface);
            presented
        };
        if presented {
            Ok(())
        } else {
            Err(Status::NotReady)
        }
    }
}

fn proc_symbol<T>(instance: *mut c_void, library: *mut c_void, name: *const c_char) -> Option<T> {
    // SAFETY: `name` is a NUL-terminated literal. `library` is the `libvulkan.so` handle.
    let address = unsafe { proc_address(instance, name, library) };
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
    // SAFETY: Vulkan passes a NUL-terminated entry-point name.
    let requested = unsafe { CStr::from_ptr(name) };
    if instance.is_null() {
        return unsafe { dlsym(user_data, name) };
    }
    let get_instance = unsafe { dlsym(user_data, c"vkGetInstanceProcAddr".as_ptr()) };
    if get_instance.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `vkGetInstanceProcAddr` is the loader entry point from `libvulkan.so`.
    let get_instance: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
        unsafe { std::mem::transmute(get_instance) };
    let resolved = unsafe { get_instance(instance, name) };
    if resolved.is_null() {
        return ptr::null_mut();
    }
    if requested == c"vkCreateSwapchainKHR" {
        REAL_CREATE.store(resolved, Ordering::SeqCst);
        // SAFETY: `create_swapchain` has the same signature as `vkCreateSwapchainKHR`.
        return unsafe { std::mem::transmute(create_swapchain as CreateSwapchain) };
    }
    if requested == c"vkGetDeviceProcAddr" {
        REAL_GET_DEVICE.store(resolved, Ordering::SeqCst);
        // SAFETY: `get_device_proc` has the same signature as `vkGetDeviceProcAddr`.
        return unsafe { std::mem::transmute(get_device_proc as GetDeviceProc) };
    }
    resolved
}

unsafe extern "C" fn get_device_proc(device: *mut c_void, name: *const c_char) -> *mut c_void {
    let get_device = REAL_GET_DEVICE.load(Ordering::SeqCst);
    if get_device.is_null() || name.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `get_device` is the real `vkGetDeviceProcAddr` stored by `proc_address`.
    let get_device: GetDeviceProc = unsafe { std::mem::transmute(get_device) };
    let resolved = unsafe { get_device(device, name) };
    // SAFETY: Vulkan passes a NUL-terminated entry-point name.
    let requested = unsafe { CStr::from_ptr(name) };
    if requested == c"vkCreateSwapchainKHR" && !resolved.is_null() {
        REAL_CREATE.store(resolved, Ordering::SeqCst);
        // SAFETY: `create_swapchain` has the same signature as `vkCreateSwapchainKHR`.
        return unsafe { std::mem::transmute(create_swapchain as CreateSwapchain) };
    }
    resolved
}

unsafe extern "C" fn create_swapchain(
    device: *mut c_void,
    info: *mut SwapchainCreateInfo,
    allocator: *const c_void,
    swapchain: *mut u64,
) -> i32 {
    let create = REAL_CREATE.load(Ordering::SeqCst);
    if create.is_null() {
        return -1;
    }
    let width = DESIRED_WIDTH.load(Ordering::SeqCst);
    let height = DESIRED_HEIGHT.load(Ordering::SeqCst);
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
    let Some((previous_width, previous_height)) = previous else {
        return result;
    };
    if result == 0 {
        return result;
    }
    // The window size was rejected. Restore the extent Impeller chose so the
    // chain still comes up, and leave a log for the size that was refused.
    let header = unsafe { &mut *info };
    header.extent_width = previous_width;
    header.extent_height = previous_height;
    if !swapchain.is_null() {
        unsafe { *swapchain = 0 };
    }
    log_swapchain(width, height, result);
    unsafe { create(device, info, allocator, swapchain) }
}

fn log_swapchain(width: u32, height: u32, result: i32) {
    log_uifw(
        6,
        &format!("swapchain extent {width}x{height} refused ({result})"),
    );
}

fn log_uifw(priority: i32, message: &str) {
    let mut bytes = message.replace('\0', " ").into_bytes();
    bytes.push(0);
    unsafe extern "C" {
        fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
    }
    // SAFETY: `bytes` is NUL-terminated and lives for this call. The tag is a literal.
    unsafe {
        __android_log_write(priority, c"uifw".as_ptr(), bytes.as_ptr().cast());
    }
}
