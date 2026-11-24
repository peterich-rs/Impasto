//! Leases a Java ImageReader frame until the Vulkan texture has retired.

use std::{
    ffi::c_void,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::{Arc, OnceLock},
};

use jni::{
    Env, JavaVM,
    objects::{Global, JByteArray, JObject},
};
use ndk_sys::{AHardwareBuffer, AHardwareBuffer_Desc, AHardwareBuffer_UsageFlags};
use raster::{GpuImage, PixelSize, Rasterizer, Status};

type FromHardwareBuffer =
    unsafe extern "C" fn(*mut jni::sys::JNIEnv, jni::sys::jobject) -> *mut AHardwareBuffer;
type DescribeHardwareBuffer =
    unsafe extern "C" fn(*const AHardwareBuffer, *mut AHardwareBuffer_Desc);

struct AndroidHardwareBuffer {
    _library: libloading::Library,
    from_java: FromHardwareBuffer,
    describe: DescribeHardwareBuffer,
}

static HARDWARE_BUFFER: OnceLock<Option<AndroidHardwareBuffer>> = OnceLock::new();

#[expect(
    unsafe_code,
    reason = "the app supports Android versions older than the HardwareBuffer JNI symbol"
)]
fn from_java(env: &Env<'_>, buffer: &JObject<'_>, size: PixelSize) -> Option<NonNull<c_void>> {
    if buffer.is_null() {
        return None;
    }
    let api = HARDWARE_BUFFER
        .get_or_init(|| {
            // SAFETY: this is Android's system library. Hold it while using its
            // optional API-26 symbol and use the signature from android/hardware_buffer_jni.h.
            let library = unsafe { libloading::Library::new("libandroid.so") }.ok()?;
            // SAFETY: this symbol has the signature in hardware_buffer_jni.h.
            let symbol =
                unsafe { library.get::<FromHardwareBuffer>(b"AHardwareBuffer_fromHardwareBuffer") }
                    .ok()?;
            let from_java = *symbol;
            // SAFETY: this symbol has the signature in hardware_buffer.h.
            let describe =
                *unsafe { library.get::<DescribeHardwareBuffer>(b"AHardwareBuffer_describe") }
                    .ok()?;
            Some(AndroidHardwareBuffer {
                _library: library,
                from_java,
                describe,
            })
        })
        .as_ref()?;
    // SAFETY: the Java wrapper passes a live android.hardware.HardwareBuffer.
    // Its owning Image stays alive during this call and the native import.
    let pointer = NonNull::new(unsafe { (api.from_java)(env.get_raw(), buffer.as_raw()) })?;
    let mut descriptor = MaybeUninit::uninit();
    // SAFETY: the hardware buffer is live, and describe initializes the full C descriptor.
    let descriptor = unsafe {
        (api.describe)(pointer.as_ptr(), descriptor.as_mut_ptr());
        descriptor.assume_init()
    };
    let has =
        |flag: AHardwareBuffer_UsageFlags| u128::from(descriptor.usage) & u128::from(flag.0) != 0;
    let compatible = descriptor.width == size.width
        && descriptor.height == size.height
        && descriptor.layers == 1
        && has(AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE)
        && !has(AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_PROTECTED_CONTENT);
    if !compatible {
        tracing::trace!(target: "impasto::gpu", usage = descriptor.usage, "Android HardwareBuffer descriptor requires CPU fallback");
    }
    compatible.then(|| pointer.cast())
}

pub(super) struct HardwareFrame {
    vm: JavaVM,
    object: Global<JObject<'static>>,
    size: PixelSize,
}

impl HardwareFrame {
    pub fn new(vm: JavaVM, object: Global<JObject<'static>>, size: PixelSize) -> Self {
        Self { vm, object, size }
    }

    #[expect(
        unsafe_code,
        reason = "the Kotlin frame holds a signaled, uncropped hardware buffer lease"
    )]
    pub fn image(self: &Arc<Self>, raster: &Rasterizer) -> Option<GpuImage> {
        self.vm.attach_current_thread_for_scope(|env| -> jni::errors::Result<_> {
            let buffer = env.call_method(self.object.as_obj(), jni::jni_str!("buffer"),
                jni::jni_sig!("()Landroid/hardware/HardwareBuffer;"), &[])?.l()?;
            if let Some(buffer) = from_java(env, &buffer, self.size) {
                let hold = Arc::clone(self);
                // SAFETY: Kotlin awaited the producer fence and the global
                // Image lease preserves the native frame for the texture's lifetime.
                match unsafe { raster.import_android_texture(buffer, self.size, move || drop(hold)) } {
                    Ok(Some(image)) => return Ok(Some(image)),
                    Err(Status::DeviceLost | Status::OutOfMemory) => return Ok(None),
                    Err(error) => {
                        tracing::debug!(target: "impasto::gpu", %error, "Android frame import failed");
                    }
                    Ok(None) => {}
                }
            }
            let Some(factory) = raster.texture_factory() else { return Ok(None) };
            let rgba = env.call_method(self.object.as_obj(), jni::jni_str!("rgba"),
                jni::jni_sig!("()[B"), &[])?.l()?;
            let rgba = env.cast_local::<JByteArray>(rgba)?;
            let rgba = env.convert_byte_array(&rgba)?;
            Ok(factory.upload_rgba(self.size.width, self.size.height, &rgba))
        }).ok().flatten()
    }
}

impl Drop for HardwareFrame {
    fn drop(&mut self) {
        let _ = self
            .vm
            .attach_current_thread_for_scope(|env| -> jni::errors::Result<()> {
                env.call_method(
                    self.object.as_obj(),
                    jni::jni_str!("release"),
                    jni::jni_sig!("()V"),
                    &[],
                )?;
                Ok(())
            });
    }
}
