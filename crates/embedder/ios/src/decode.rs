//! ImageIO decoding on the common host's image worker.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;

use embedder::{ImageDecoder, ImageError, ImageHandle, pixel_count_ok};
use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFNumber, CFString, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_image_io::{
    CGImageSource, kCGImagePropertyOrientation, kCGImagePropertyPixelHeight,
    kCGImagePropertyPixelWidth, kCGImageSourceCreateThumbnailFromImageAlways,
    kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceThumbnailMaxPixelSize,
};

pub(super) struct Decoder {
    root: PathBuf,
}

impl Decoder {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl ImageDecoder for Decoder {
    fn read_packaged_asset(&self, relative: &str, limit: usize) -> Result<Arc<[u8]>, String> {
        embedder::read_packaged_file(&self.root, relative, limit)
    }

    fn decode(&self, source: &str) -> Result<ImageHandle, ImageError> {
        self.decode_at(source, 1)
    }

    fn decode_at(&self, source: &str, scale_bucket: u8) -> Result<ImageHandle, ImageError> {
        self.decode_sized(source, scale_bucket, None)
    }

    fn decode_sized(
        &self,
        source: &str,
        scale_bucket: u8,
        target: Option<embedder::DecodeTarget>,
    ) -> Result<ImageHandle, ImageError> {
        for relative in embedder::asset_image_candidates(source, scale_bucket) {
            #[cfg(debug_assertions)]
            let path = match embedder::dev_channel::asset_override(&relative) {
                embedder::dev_channel::AssetOverride::File(path) => path,
                embedder::dev_channel::AssetOverride::Removed => continue,
                embedder::dev_channel::AssetOverride::Packaged => self.root.join(&relative),
            };
            #[cfg(not(debug_assertions))]
            let path = self.root.join(relative);
            match fs::read(&path) {
                Ok(bytes) => return self.decode_bytes_sized(source, &bytes, target),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ImageError::Failed(format!(
                        "读取 {} 失败。{error}",
                        path.display()
                    )));
                }
            }
        }
        Err(ImageError::Missing)
    }

    fn decode_bytes(&self, source: &str, bytes: &[u8]) -> Result<ImageHandle, ImageError> {
        self.decode_bytes_sized(source, bytes, None)
    }

    fn decode_bytes_sized(
        &self,
        source: &str,
        bytes: &[u8],
        target: Option<embedder::DecodeTarget>,
    ) -> Result<ImageHandle, ImageError> {
        objc2::rc::autoreleasepool(|_| decode(source, bytes, target))
    }
}

#[allow(
    unsafe_code,
    reason = "ImageIO and Core Graphics require native handles and bitmap buffers"
)]
fn decode(
    source: &str,
    bytes: &[u8],
    target: Option<embedder::DecodeTarget>,
) -> Result<ImageHandle, ImageError> {
    let data = CFData::from_bytes(bytes);
    // SAFETY: data remains retained during source creation; no options are supplied.
    let image_source = unsafe { CGImageSource::with_data(&data, None) }
        .ok_or_else(|| failed("ImageIO 无法识别图像"))?;
    // SAFETY: index 0 is the first image; no options are supplied.
    let properties = unsafe { image_source.properties_at_index(0, None) }
        .ok_or_else(|| failed("图像没有像素信息"))?;
    // SAFETY: ImageIO's properties dictionary has CFString keys and CF object values.
    let properties = unsafe { properties.cast_unchecked::<CFString, CFType>() };
    // SAFETY: ImageIO provides both initialized constant keys for the process lifetime.
    let (width_key, height_key) =
        unsafe { (kCGImagePropertyPixelWidth, kCGImagePropertyPixelHeight) };
    let mut source_size = [
        size_property(properties, width_key)?,
        size_property(properties, height_key)?,
    ];
    if source_size.contains(&0) {
        return Err(failed("图像没有尺寸"));
    }
    // SAFETY: ImageIO provides the orientation key for the process lifetime.
    if size_property(properties, unsafe { kCGImagePropertyOrientation }).is_ok_and(|exif| exif >= 5)
    {
        source_size.swap(0, 1);
    }
    let requested = target.map_or(source_size, |target| target.fit(source_size));
    if !pixel_count_ok(requested) {
        return Err(failed("图像超过像素预算"));
    }
    let max_side = CFNumber::new_i64(i64::from(requested[0].max(requested[1])));
    let flag = CFBoolean::new(true);
    // SAFETY: ImageIO provides these initialized constant keys for the process lifetime.
    let keys = unsafe {
        [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceCreateThumbnailWithTransform,
            kCGImageSourceThumbnailMaxPixelSize,
        ]
    };
    let options = CFDictionary::<CFString, CFType>::from_slices(&keys, &[flag, flag, &max_side]);
    // SAFETY: the ImageIO options have the documented Boolean and Number types.
    let image = unsafe { image_source.thumbnail_at_index(0, Some(options.as_opaque())) }
        .ok_or_else(|| failed("ImageIO 图像解码失败"))?;
    let width = CGImage::width(Some(&image));
    let height = CGImage::height(Some(&image));
    let size = [
        u32::try_from(width).map_err(|_| failed("图像宽度超出范围"))?,
        u32::try_from(height).map_err(|_| failed("图像高度超出范围"))?,
    ];
    if size.contains(&0) || !pixel_count_ok(size) {
        return Err(failed("图像超过像素预算"));
    }

    let row = width * 4;
    let mut pixels = vec![0; row * height];
    // SAFETY: Core Graphics initializes this color space name for the process lifetime.
    let srgb = unsafe { kCGColorSpaceSRGB };
    let space =
        CGColorSpace::with_name(Some(srgb)).ok_or_else(|| failed("无法创建 sRGB 色彩空间"))?;
    let bitmap = CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0;
    // SAFETY: the RGBA8 buffer has four bytes per pixel, and context drops before pixels.
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            8,
            row,
            Some(&space),
            bitmap,
        )
    }
    .ok_or_else(|| failed("无法创建图像位图上下文"))?;
    let rect = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(f64::from(size[0]), f64::from(size[1])),
    );
    CGContext::draw_image(Some(&context), rect, Some(&image));
    drop(context);
    embedder::unpremultiply_rgba(&mut pixels);
    Ok(ImageHandle {
        source: Arc::from(source),
        generation: 0,
        size,
        pixels: Arc::from(pixels),
        gpu: None,
    })
}

fn size_property(
    properties: &CFDictionary<CFString, CFType>,
    key: &CFString,
) -> Result<u32, ImageError> {
    properties
        .get(key)
        .and_then(|value| value.downcast_ref::<CFNumber>()?.as_i64())
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| failed("图像像素尺寸无效"))
}

fn failed(message: &str) -> ImageError {
    ImageError::Failed(message.into())
}
