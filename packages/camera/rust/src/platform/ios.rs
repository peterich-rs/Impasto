use std::{ffi::c_void, ptr::NonNull};

use impasto::{MetalTexture, PluginContext};

use crate::{CameraError, Facing, Frame};

#[derive(Clone)]
pub struct Camera {
    facing: Facing,
}

// Implemented by the Swift source linked into the app. Both functions enqueue
// work on the camera's serial queue and accept only an owned session ID.
unsafe extern "C" {
    fn impasto_camera_start(id: u64, front: bool);
    fn impasto_camera_stop(id: u64);
}

impl Camera {
    pub fn new(_: &PluginContext, facing: Facing) -> Result<Self, CameraError> {
        Ok(Self { facing })
    }

    pub async fn start(&self, id: u64, name: &str) -> Result<(), CameraError> {
        self.resume(id, name);
        Ok(())
    }

    pub fn stop(&self, id: u64) {
        // SAFETY: the bridge owns its queue and accepts any session ID.
        unsafe { impasto_camera_stop(id) };
    }

    pub fn resume(&self, id: u64, _: &str) {
        // SAFETY: the bridge captures only scalar parameters on its queue.
        unsafe { impasto_camera_start(id, matches!(self.facing, Facing::Front)) };
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn impasto_camera_ios_frame(
    id: u64,
    width: u32,
    height: u32,
    stride: usize,
    pixels: *const u8,
    len: usize,
    texture: *mut c_void,
    owner: usize,
    release: extern "C" fn(usize),
) {
    let lease = NonNull::new(texture).map(|texture| {
        // SAFETY: Swift retains the source pixel buffer and Metal view until
        // the release callback. The camera finished writing before delivery.
        unsafe { MetalTexture::from_raw(texture, move || release(owner)) }
    });
    let Some((width_px, height_px, count)) = crate::dimensions(width, height) else {
        return;
    };
    if pixels.is_null()
        || stride < width_px * 4
        || stride
            .checked_mul(height_px)
            .is_none_or(|need| need > len || need > isize::MAX.unsigned_abs())
    {
        return;
    }
    // SAFETY: the Swift capture delegate locks the CVPixelBuffer for this call.
    // Geometry and the maximum slice extent were checked above.
    let bgra = unsafe { std::slice::from_raw_parts(pixels, stride * height_px) };
    let mut rgba = Vec::with_capacity(count * 4);
    for row in bgra.chunks_exact(stride) {
        for pixel in row[..width_px * 4].as_chunks::<4>().0 {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    crate::frame(
        id,
        Frame {
            width,
            height,
            rgba,
        },
        move |texture, frame| {
            let pixels = impasto::ExternalFrame {
                width: frame.width,
                height: frame.height,
                pixels: frame.rgba.clone(),
            };
            let _ = match lease {
                Some(lease) => texture.publish_metal(lease, pixels),
                None => texture.publish_rgba(pixels),
            };
        },
    );
}

#[unsafe(no_mangle)]
extern "C" fn impasto_camera_ios_failed(id: u64) {
    crate::failed(id);
}
