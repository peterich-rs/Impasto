//! Asset bytes and one-shot RGBA decode. `AImageDecoder` is resolved when the device has it.

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

use ndk::asset::{Asset, AssetManager};

const RTLD_NOW: i32 = 2;
const RGBA_8888: i32 = 1;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

type CreateFromBuffer = unsafe extern "C" fn(
    buffer: *const c_void,
    length: usize,
    out_decoder: *mut *mut c_void,
) -> i32;
type HeaderInfo = unsafe extern "C" fn(decoder: *mut c_void) -> *const c_void;
type HeaderI32 = unsafe extern "C" fn(header: *const c_void) -> i32;
type SetFormat = unsafe extern "C" fn(decoder: *mut c_void, format: i32) -> i32;
type MinStride = unsafe extern "C" fn(decoder: *mut c_void) -> usize;
type DecodeImage = unsafe extern "C" fn(
    decoder: *mut c_void,
    pixels: *mut c_void,
    stride: usize,
    size: usize,
) -> i32;
type DeleteDecoder = unsafe extern "C" fn(decoder: *mut c_void);
type SetUnpremultiplied = unsafe extern "C" fn(decoder: *mut c_void, required: u8) -> i32;

struct ImageFns {
    create: CreateFromBuffer,
    header: HeaderInfo,
    width: HeaderI32,
    height: HeaderI32,
    set_format: SetFormat,
    unpremultiply: Option<SetUnpremultiplied>,
    stride: MinStride,
    decode: DecodeImage,
    delete: DeleteDecoder,
}

fn image_fns() -> Option<&'static ImageFns> {
    static FNS: OnceLock<Option<ImageFns>> = OnceLock::new();
    FNS.get_or_init(|| {
        // SAFETY: these two libraries are public NDK images. `AImageDecoder` is
        // exported by `libjnigraphics.so` on current devices; older images also
        // re-export it from `libandroid.so`. A missing symbol means this device
        // has no decoder, and decode then fails closed.
        unsafe {
            let library = decoder_library()?;
            let symbol = |name: &std::ffi::CStr| dlsym(library, name.as_ptr());
            let create = symbol(c"AImageDecoder_createFromBuffer");
            let header = symbol(c"AImageDecoder_getHeaderInfo");
            let width = symbol(c"AImageDecoderHeaderInfo_getWidth");
            let height = symbol(c"AImageDecoderHeaderInfo_getHeight");
            let set_format = symbol(c"AImageDecoder_setAndroidBitmapFormat");
            let unpremultiply = symbol(c"AImageDecoder_setUnpremultipliedRequired");
            let stride = symbol(c"AImageDecoder_getMinimumStride");
            let decode = symbol(c"AImageDecoder_decodeImage");
            let delete = symbol(c"AImageDecoder_delete");
            if create.is_null()
                || header.is_null()
                || width.is_null()
                || height.is_null()
                || set_format.is_null()
                || stride.is_null()
                || decode.is_null()
                || delete.is_null()
            {
                return None;
            }
            Some(ImageFns {
                create: std::mem::transmute(create),
                header: std::mem::transmute(header),
                width: std::mem::transmute(width),
                height: std::mem::transmute(height),
                set_format: std::mem::transmute(set_format),
                unpremultiply: if unpremultiply.is_null() {
                    None
                } else {
                    Some(std::mem::transmute(unpremultiply))
                },
                stride: std::mem::transmute(stride),
                decode: std::mem::transmute(decode),
                delete: std::mem::transmute(delete),
            })
        }
    })
    .as_ref()
}

fn decoder_library() -> Option<*mut c_void> {
    for name in [c"libjnigraphics.so", c"libandroid.so"] {
        // SAFETY: `name` is a static C string. A null handle means that library
        // is not in this process namespace; the next name is tried.
        let library = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
        if library.is_null() {
            continue;
        }
        let create = unsafe { dlsym(library, c"AImageDecoder_createFromBuffer".as_ptr()) };
        if !create.is_null() {
            return Some(library);
        }
    }
    None
}

pub fn read_bytes(assets: &AssetManager, path: &std::ffi::CStr) -> Option<Vec<u8>> {
    let mut asset: Asset = assets.open(path)?;
    let bytes = asset.buffer().ok()?;
    Some(bytes.to_vec())
}

pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn decode_rgba(bytes: &[u8]) -> Result<Rgba, String> {
    let fns = image_fns().ok_or_else(|| "AImageDecoder is missing".to_string())?;
    if bytes.is_empty() {
        return Err("empty image".to_string());
    }
    let mut decoder = std::ptr::null_mut();
    // SAFETY: `bytes` lives until `delete`. The function table was checked non-null.
    let created = unsafe { (fns.create)(bytes.as_ptr().cast(), bytes.len(), &mut decoder) };
    if created != 0 || decoder.is_null() {
        return Err(format!("create {created}"));
    }
    let image = unsafe {
        let header = (fns.header)(decoder);
        if header.is_null() {
            (fns.delete)(decoder);
            return Err("header is null".to_string());
        }
        let width = (fns.width)(header);
        let height = (fns.height)(header);
        if width <= 0 || height <= 0 {
            (fns.delete)(decoder);
            return Err(format!("size {width}x{height}"));
        }
        let set_format = (fns.set_format)(decoder, RGBA_8888);
        if set_format != 0 {
            (fns.delete)(decoder);
            return Err(format!("set_format {set_format} {width}x{height}"));
        }
        if let Some(unpremultiply) = fns.unpremultiply {
            // Straight alpha matches the PNG bytes Impeller samples. A refusal
            // leaves the decoder's default, which is premultiplied.
            let _ = unpremultiply(decoder, 1);
        }
        let stride = (fns.stride)(decoder);
        let Some(row) = (width as usize).checked_mul(4) else {
            (fns.delete)(decoder);
            return Err("row overflow".to_string());
        };
        if stride < row {
            (fns.delete)(decoder);
            return Err(format!(
                "stride {stride} < {row} set_format {set_format} {width}x{height}"
            ));
        }
        let Some(size) = stride.checked_mul(height as usize) else {
            (fns.delete)(decoder);
            return Err("size overflow".to_string());
        };
        let mut packed = vec![0u8; size];
        let status = (fns.decode)(decoder, packed.as_mut_ptr().cast(), stride, size);
        (fns.delete)(decoder);
        if status != 0 {
            return Err(format!("decode {status} set_format {set_format}"));
        }
        let pixels = if stride == row {
            packed
        } else {
            let mut tight = Vec::with_capacity(row * height as usize);
            for y in 0..height as usize {
                let start = y * stride;
                tight.extend_from_slice(&packed[start..start + row]);
            }
            tight
        };
        Rgba {
            width: width as u32,
            height: height as u32,
            pixels,
        }
    };
    Ok(image)
}
