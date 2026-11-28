//! System media selection and bounded offline image processing.

use image::{
    DynamicImage, ImageDecoder as _, ImageEncoder, ImageFormat, ImageReader, metadata::Orientation,
};
use impasto::os::{NativePlugin, NativePluginError};
use impasto::{ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar};
use impasto_file_system::{Scope, resolve_path};
use impasto_permissions::{Permission, PermissionStatus};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use tokio::sync::Semaphore;
use uuid::Uuid;

#[derive(Clone, Copy, ImpastoType, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Kind {
    Image,
    Video,
}
#[derive(Clone, Copy, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
enum MediaType {
    Image,
    Video,
    Any,
}
#[derive(Clone, Copy, Default, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
enum OutputScope {
    #[default]
    Cache,
    Temp,
}
impl From<OutputScope> for Scope {
    fn from(scope: OutputScope) -> Self {
        match scope {
            OutputScope::Cache => Self::Cache,
            OutputScope::Temp => Self::Temp,
        }
    }
}
#[derive(Clone, Copy, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
enum Format {
    Original,
    Jpeg,
    Png,
    Webp,
}
#[derive(Clone, Copy, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
enum Camera {
    Front,
    Back,
}
#[derive(Clone, Copy, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
enum VideoQuality {
    Low,
    Medium,
    High,
}

#[derive(Clone, Default, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
struct Process {
    max_width: Option<u32>,
    max_height: Option<u32>,
    quality: Option<u8>,
    output_format: Option<Format>,
    fix_orientation: Option<bool>,
    strip_metadata: Option<bool>,
}
#[derive(Default, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
struct Pick {
    media_type: Option<MediaType>,
    multiple: Option<bool>,
    max_selection: Option<u32>,
    process: Option<Process>,
    output_scope: Option<OutputScope>,
}
#[derive(Default, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
struct Photo {
    preferred_camera: Option<Camera>,
    process: Option<Process>,
    output_scope: Option<OutputScope>,
}
#[derive(Default, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
struct Video {
    preferred_camera: Option<Camera>,
    max_duration_sec: Option<u32>,
    quality: Option<VideoQuality>,
    output_scope: Option<OutputScope>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Staged {
    kind: Kind,
    path: PathBuf,
    file_name: String,
    mime_type: String,
    width: Option<u32>,
    height: Option<u32>,
    duration_ms: Option<f64>,
}
#[derive(ImpastoType)]
struct PickedMedia {
    kind: Kind,
    relative_path: String,
    scope: OutputScope,
    file_name: String,
    mime_type: String,
    size: f64,
    #[impasto(nullable)]
    width: Option<u32>,
    #[impasto(nullable)]
    height: Option<u32>,
    #[impasto(nullable)]
    duration_ms: Option<f64>,
}

#[derive(Debug)]
struct MediaError(&'static str, String);
impl MediaError {
    fn new(code: &'static str, message: &str) -> Self {
        Self(code, message.to_owned())
    }
    fn io() -> Self {
        Self::new("io", "Media could not be imported")
    }
    fn unsupported() -> Self {
        Self::new(
            "unsupported-media",
            "Unsupported media format or dimensions",
        )
    }
}
impl ImpastoError for MediaError {
    fn scope(&self) -> &'static str {
        "image-picker"
    }
    fn code(&self) -> &'static str {
        self.0
    }
    fn message(&self) -> String {
        self.1.clone()
    }
}
impl From<NativePluginError> for MediaError {
    fn from(error: NativePluginError) -> Self {
        Self(
            match error.code.as_str() {
                "permission-denied" => "permission-denied",
                "camera-unavailable" => "camera-unavailable",
                "unsupported-media" => "unsupported-media",
                "not-supported" => "not-supported",
                "cancelled" => "cancelled",
                "busy" => "busy",
                "outside-scope" => "outside-scope",
                "host-unavailable" => "host-unavailable",
                _ => "io",
            },
            error.message,
        )
    }
}

fn path(ctx: &PluginContext, scope: Scope, relative: &str) -> Result<PathBuf, MediaError> {
    resolve_path(ctx, scope, relative)
        .map_err(|_| MediaError::new("outside-scope", "Media path is outside its scope"))
}

fn validated(process: Option<&Process>) -> Result<(), MediaError> {
    if let Some(process) = process
        && (process.max_width == Some(0)
            || process.max_height == Some(0)
            || process
                .quality
                .is_some_and(|quality| !(1..=100).contains(&quality)))
    {
        return Err(MediaError::unsupported());
    }
    Ok(())
}

fn staging(ctx: &PluginContext) -> Result<TempDir, MediaError> {
    let root = path(ctx, Scope::Cache, "impasto-image-picker-staging")?;
    fs::create_dir_all(&root).map_err(|_| MediaError::io())?;
    tempfile::Builder::new()
        .prefix("pick-")
        .tempdir_in(root)
        .map_err(|_| MediaError::io())
}

fn valid_source(staging: &Path, source: &Path) -> Result<PathBuf, MediaError> {
    let root = fs::canonicalize(staging).map_err(|_| MediaError::io())?;
    let source = fs::canonicalize(source).map_err(|_| MediaError::io())?;
    if !source.starts_with(root) || !source.is_file() {
        return Err(MediaError::new(
            "outside-scope",
            "Native media escaped its staging directory",
        ));
    }
    Ok(source)
}

fn ext(source: &Path) -> &str {
    source
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 10
                && value.bytes().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or("bin")
}

#[derive(Default)]
struct ImageMetadata {
    exif: Option<Vec<u8>>,
    icc: Option<Vec<u8>>,
}

impl ImageMetadata {
    fn apply(self, encoder: &mut impl ImageEncoder) -> Result<(), MediaError> {
        if let Some(exif) = self.exif {
            encoder
                .set_exif_metadata(exif)
                .map_err(|_| MediaError::io())?;
        }
        if let Some(icc) = self.icc {
            encoder.set_icc_profile(icc).map_err(|_| MediaError::io())?;
        }
        Ok(())
    }
}

fn process_image(
    path: &Path,
    options: &Process,
) -> Result<(DynamicImage, ImageFormat, ImageMetadata), MediaError> {
    let mut reader = ImageReader::open(path)
        .map_err(|_| MediaError::io())?
        .with_guessed_format()
        .map_err(|_| MediaError::unsupported())?;
    let original = reader.format().ok_or_else(MediaError::unsupported)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(160 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| MediaError::unsupported())?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > 40_000_000 {
        return Err(MediaError::unsupported());
    }
    let orientation = decoder
        .orientation()
        .map_err(|_| MediaError::unsupported())?;
    let mut metadata = if options.strip_metadata == Some(false) {
        ImageMetadata {
            exif: decoder
                .exif_metadata()
                .map_err(|_| MediaError::unsupported())?,
            icc: decoder
                .icc_profile()
                .map_err(|_| MediaError::unsupported())?,
        }
    } else {
        ImageMetadata::default()
    };
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| MediaError::unsupported())?;
    if options.fix_orientation.unwrap_or(true) {
        if let Some(exif) = metadata.exif.as_mut() {
            let _ = Orientation::remove_from_exif_chunk(exif);
        }
        image.apply_orientation(orientation);
    }
    let width = options.max_width.unwrap_or(image.width());
    let height = options.max_height.unwrap_or(image.height());
    if image.width() > width || image.height() > height {
        image = image.resize(width, height, image::imageops::FilterType::Lanczos3);
    }
    let chosen = match options.output_format.unwrap_or(Format::Original) {
        Format::Jpeg => ImageFormat::Jpeg,
        Format::Png => ImageFormat::Png,
        Format::Webp => ImageFormat::WebP,
        Format::Original => match original {
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP => original,
            _ => ImageFormat::Jpeg,
        },
    };
    Ok((image, chosen, metadata))
}

fn finish(
    ctx: &PluginContext,
    staging: &Path,
    scope: OutputScope,
    process: Option<&Process>,
    staged: Staged,
) -> Result<PickedMedia, MediaError> {
    let source = valid_source(staging, &staged.path)?;
    let mut mime = staged.mime_type;
    let mut width = staged.width;
    let mut height = staged.height;
    let root = path(ctx, scope.into(), "image-picker")?;
    fs::create_dir_all(&root).map_err(|_| MediaError::io())?;
    let mut extension = ext(&source).to_ascii_lowercase();
    let processing = process.filter(|options| {
        options.fix_orientation.unwrap_or(true)
            || options.strip_metadata.unwrap_or(true)
            || options.max_width.is_some()
            || options.max_height.is_some()
            || options.quality.is_some()
            || !matches!(options.output_format, None | Some(Format::Original))
    });
    let (name, size) = if let (Kind::Image, Some(process)) = (staged.kind, processing) {
        let (image, format, metadata) = process_image(&source, process)?;
        width = Some(image.width());
        height = Some(image.height());
        extension = format
            .extensions_str()
            .first()
            .copied()
            .unwrap_or("bin")
            .to_owned();
        mime = format.to_mime_type().to_owned();
        let mut file = tempfile::NamedTempFile::new_in(&root).map_err(|_| MediaError::io())?;
        let mut output = BufWriter::new(file.as_file_mut());
        encode(
            &mut output,
            image,
            format,
            metadata,
            process.quality.unwrap_or(85),
        )?;
        output.flush().map_err(|_| MediaError::io())?;
        drop(output);
        save(&root, &extension, file)?
    } else {
        if matches!(staged.kind, Kind::Image)
            && width.is_none()
            && let Ok(reader) =
                ImageReader::open(&source).and_then(ImageReader::with_guessed_format)
            && let Ok((w, h)) = reader.into_dimensions()
        {
            width = Some(w);
            height = Some(h);
        }
        let size = source.metadata().map_err(|_| MediaError::io())?.len() as f64;
        let name = format!("{}.{}", Uuid::new_v4(), extension);
        match fs::hard_link(&source, root.join(&name)) {
            Ok(()) => (name, size),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::CrossesDevices | std::io::ErrorKind::Unsupported
                ) =>
            {
                let mut file =
                    tempfile::NamedTempFile::new_in(&root).map_err(|_| MediaError::io())?;
                std::io::copy(
                    &mut File::open(&source).map_err(|_| MediaError::io())?,
                    &mut file,
                )
                .map_err(|_| MediaError::io())?;
                save(&root, &extension, file)?
            }
            Err(_) => return Err(MediaError::io()),
        }
    };
    Ok(PickedMedia {
        kind: staged.kind,
        relative_path: format!("image-picker/{name}"),
        scope,
        file_name: staged.file_name,
        mime_type: mime,
        size,
        width,
        height,
        duration_ms: staged.duration_ms,
    })
}

fn encode(
    output: &mut impl Write,
    image: DynamicImage,
    format: ImageFormat,
    metadata: ImageMetadata,
    quality: u8,
) -> Result<(), MediaError> {
    let width = image.width();
    let height = image.height();
    match format {
        ImageFormat::Jpeg => {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(output, quality);
            metadata.apply(&mut encoder)?;
            encoder
                .encode_image(&image.into_rgb8())
                .map_err(|_| MediaError::io())
        }
        ImageFormat::Png => {
            let mut encoder = image::codecs::png::PngEncoder::new(output);
            metadata.apply(&mut encoder)?;
            encoder
                .write_image(
                    &image.into_rgba8(),
                    width,
                    height,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|_| MediaError::io())
        }
        ImageFormat::WebP => {
            let pixels = webp_rust::ImageBuffer {
                width: width as usize,
                height: height as usize,
                rgba: image.into_rgba8().into_raw(),
            };
            let options = webp_rust::LossyEncodingConfig {
                quality: f32::from(quality),
                ..Default::default()
            };
            let encoded =
                webp_rust::encode_lossy_with_config(&pixels, &options, metadata.exif.as_deref())
                    .map_err(|_| MediaError::io())?;
            if let Some(icc) = metadata.icc {
                write_profiled_webp(output, &encoded, &icc, width, height)
            } else {
                output.write_all(&encoded).map_err(|_| MediaError::io())
            }
        }
        _ => Err(MediaError::unsupported()),
    }
}

fn write_profiled_webp(
    output: &mut impl Write,
    encoded: &[u8],
    profile: &[u8],
    width: u32,
    height: u32,
) -> Result<(), MediaError> {
    // RFC 9649 requires the extended header and ICCP before the image bitstream.
    let chunks = encoded.get(12..).ok_or_else(MediaError::io)?;
    let mut extended = [0u8; 10];
    let chunks = if chunks.starts_with(b"VP8X\x0a\0\0\0") {
        extended.copy_from_slice(chunks.get(8..18).ok_or_else(MediaError::io)?);
        chunks.get(18..).ok_or_else(MediaError::io)?
    } else {
        extended[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
        extended[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
        chunks
    };
    extended[0] |= 0x20;
    let pad = profile.len() % 2;
    let profile_length = u32::try_from(profile.len()).map_err(|_| MediaError::io())?;
    let riff_length =
        u32::try_from(30 + profile.len() + pad + chunks.len()).map_err(|_| MediaError::io())?;
    (|| -> std::io::Result<()> {
        output.write_all(b"RIFF")?;
        output.write_all(&riff_length.to_le_bytes())?;
        output.write_all(b"WEBPVP8X\x0a\0\0\0")?;
        output.write_all(&extended)?;
        output.write_all(b"ICCP")?;
        output.write_all(&profile_length.to_le_bytes())?;
        output.write_all(profile)?;
        output.write_all(&[0][..pad])?;
        output.write_all(chunks)
    })()
    .map_err(|_| MediaError::io())
}

fn save(
    root: &Path,
    extension: &str,
    file: tempfile::NamedTempFile,
) -> Result<(String, f64), MediaError> {
    let name = format!("{}.{}", Uuid::new_v4(), extension);
    let size = file
        .as_file()
        .metadata()
        .map_err(|_| MediaError::io())?
        .len() as f64;
    file.persist_noclobber(root.join(&name))
        .map_err(|_| MediaError::io())?;
    Ok((name, size))
}

#[impasto::module(name = "ImagePicker", specifier = "@impasto/image-picker/native", init = init)]
struct ImagePicker {
    ctx: PluginContext,
    native: NativePlugin,
}

impl ImagePicker {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            ctx: ctx.clone(),
            native: NativePlugin::new(ctx, "@impasto/image-picker"),
        })
    }

    async fn capture_permissions(&self, video: bool) -> Result<(), MediaError> {
        if !self.native.has_channel() {
            return Err(MediaError::new(
                "host-unavailable",
                "Media capture requires a native window",
            ));
        }
        if cfg!(target_os = "macos") {
            return Err(MediaError::new(
                "not-supported",
                "System camera capture is unavailable on macOS",
            ));
        }
        self.native.request::<()>("checkCamera", &()).await?;
        let permissions = if video {
            vec![Permission::Camera, Permission::Microphone]
        } else {
            vec![Permission::Camera]
        };
        for permission in permissions {
            let result = impasto_permissions::ensure(&self.ctx, permission)
                .await
                .map_err(|_| {
                    MediaError::new(
                        "permission-denied",
                        "Camera or microphone permission is unavailable",
                    )
                })?;
            if result != PermissionStatus::Granted {
                return Err(MediaError::new(
                    "permission-denied",
                    "Camera or microphone permission is denied",
                ));
            }
        }
        Ok(())
    }

    async fn select(
        &self,
        operation: &str,
        options: impl Serialize,
        scope: OutputScope,
        process: Option<Process>,
    ) -> Result<Vec<PickedMedia>, MediaError> {
        if !self.native.has_channel() {
            return Err(MediaError::new(
                "host-unavailable",
                "Media selection requires a native window",
            ));
        }
        validated(process.as_ref())?;
        let ctx = self.ctx.clone();
        let staging = tokio::task::spawn_blocking(move || staging(&ctx))
            .await
            .map_err(|_| MediaError::io())??;
        let mut payload = serde_json::to_value(options).map_err(|_| MediaError::unsupported())?;
        payload["staging"] = staging.path().to_string_lossy().into_owned().into();
        let selected: Vec<Staged> = self.native.request(operation, &payload).await?;
        let ctx = self.ctx.clone();
        static DECODERS: Semaphore = Semaphore::const_new(2);
        let permit = DECODERS.acquire().await.map_err(|_| MediaError::io())?;
        let result = tokio::task::spawn_blocking(move || {
            let mut imported = Vec::new();
            for staged in selected {
                match finish(&ctx, staging.path(), scope, process.as_ref(), staged) {
                    Ok(file) => imported.push(file),
                    Err(e) => {
                        for file in imported {
                            if let Ok(path) = path(&ctx, file.scope.into(), &file.relative_path) {
                                let _ = fs::remove_file(path);
                            }
                        }
                        return Err(e);
                    }
                }
            }
            Ok(imported)
        })
        .await
        .map_err(|_| MediaError::io())?;
        drop(permit);
        result
    }
}

#[impasto::export]
impl ImagePicker {
    pub fn is_image_picker_host_available(&self) -> bool {
        self.native.is_hosted()
    }
    #[impasto::method(awaits_user)]
    pub async fn pick_media(&self, options: Option<Pick>) -> Result<Vec<PickedMedia>, MediaError> {
        let options = options.unwrap_or_default();
        if options.max_selection == Some(0) {
            return Err(MediaError::unsupported());
        }
        self.select(
            "pick",
            &options,
            options.output_scope.unwrap_or_default(),
            options.process.clone(),
        )
        .await
    }
    #[impasto::method(awaits_user)]
    pub async fn capture_photo(
        &self,
        options: Option<Photo>,
    ) -> Result<Option<PickedMedia>, MediaError> {
        self.capture_permissions(false).await?;
        let options = options.unwrap_or_default();
        Ok(self
            .select(
                "photo",
                &options,
                options.output_scope.unwrap_or_default(),
                options.process.clone(),
            )
            .await?
            .into_iter()
            .next())
    }
    #[impasto::method(awaits_user)]
    pub async fn capture_video(
        &self,
        options: Option<Video>,
    ) -> Result<Option<PickedMedia>, MediaError> {
        self.capture_permissions(true).await?;
        let options = options.unwrap_or_default();
        Ok(self
            .select(
                "video",
                &options,
                options.output_scope.unwrap_or_default(),
                None,
            )
            .await?
            .into_iter()
            .next())
    }
    #[impasto::method(blocking)]
    pub fn cleanup_cache(&self) -> Result<(), MediaError> {
        for scope in [Scope::Cache, Scope::Temp] {
            let directory = path(&self.ctx, scope, "image-picker")?;
            match fs::remove_dir_all(directory) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(MediaError::io()),
            }
        }
        Ok(())
    }
}
/// Registers the image picker bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(ImagePicker::plugin());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizes_images_and_writes_lossy_webp_at_the_requested_quality() -> Result<(), MediaError> {
        let directory = tempfile::tempdir().map_err(|_| MediaError::io())?;
        let original = directory.path().join("source.png");
        image::RgbImage::from_fn(128, 64, |x, y| {
            image::Rgb([x ^ y, x * 2, y * 4].map(|value| u8::try_from(value).unwrap_or(u8::MAX)))
        })
        .save(&original)
        .map_err(|_| MediaError::io())?;
        let options = Process {
            max_width: Some(64),
            max_height: Some(64),
            output_format: Some(Format::Webp),
            ..Process::default()
        };
        let (image, format, metadata) = process_image(&original, &options)?;
        assert_eq!((image.width(), image.height()), (64, 32));
        let mut low = Vec::new();
        let mut high = Vec::new();
        encode(&mut low, image.clone(), format, metadata, 10)?;
        let profile = vec![1, 2, 3];
        let metadata = ImageMetadata {
            exif: None,
            icc: Some(profile.clone()),
        };
        encode(&mut high, image, format, metadata, 95)?;
        let mut decoder = ImageReader::with_format(std::io::Cursor::new(&high), ImageFormat::WebP)
            .into_decoder()
            .map_err(|_| MediaError::io())?;
        assert_eq!(
            decoder.icc_profile().map_err(|_| MediaError::io())?,
            Some(profile)
        );
        assert_ne!(low, high);
        let decoded = image::load_from_memory_with_format(&high, ImageFormat::WebP)
            .map_err(|_| MediaError::io())?;
        assert_eq!((decoded.width(), decoded.height()), (64, 32));
        Ok(())
    }
}
