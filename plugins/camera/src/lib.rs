//! Camera reference plugin. Frames and QR recognition stay on the native side.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
#[cfg(any(target_os = "ios", target_os = "android"))]
use std::time::{Duration, Instant};

use futures_channel::{mpsc, oneshot};
use futures_util::StreamExt as _;
use impasto::{
    Bytes, CancelToken, ExternalTexture, ImpastoError, ImpastoType, Lifecycle, PluginContext,
    PluginInitError, PluginRegistrar, Subscription, TypedSink,
};

mod platform;

#[derive(ImpastoType, Clone, Copy)]
pub enum Facing {
    Front,
    Back,
}

#[derive(ImpastoType)]
pub struct Options {
    pub facing: Facing,
}

#[derive(ImpastoType, Clone)]
pub struct QrCode {
    pub text: String,
}

#[derive(Debug)]
pub struct CameraError {
    code: &'static str,
    message: String,
}

impl CameraError {
    fn new(code: &'static str, message: impl std::fmt::Display) -> Self {
        Self {
            code,
            message: message.to_string(),
        }
    }

    fn unavailable() -> Self {
        Self::new("unavailable", "Camera is unavailable on this host")
    }
}

impl ImpastoError for CameraError {
    fn scope(&self) -> &'static str {
        "camera"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}

struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Default)]
struct Capture {
    latest: Option<Arc<Frame>>,
    #[cfg(any(target_os = "ios", target_os = "android"))]
    last_scan: Option<Instant>,
    suspended: bool,
    subscriber: Option<mpsc::Sender<QrCode>>,
    #[cfg_attr(
        not(any(target_os = "ios", target_os = "android")),
        allow(
            dead_code,
            reason = "hardware callbacks only exist on supported target platforms"
        )
    )]
    ready: Option<oneshot::Sender<Result<(), CameraError>>>,
    closed: bool,
}

struct State {
    texture: ExternalTexture,
    capture: Mutex<Capture>,
}

static SESSIONS: LazyLock<Mutex<HashMap<u64, Weak<State>>>> = LazyLock::new(Mutex::default);

#[cfg(any(target_os = "ios", target_os = "android"))]
fn session(id: u64) -> Option<Arc<State>> {
    SESSIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&id)?
        .upgrade()
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn dimensions(width: u32, height: u32) -> Option<(usize, usize, usize)> {
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    let pixels = width.checked_mul(height)?;
    (pixels > 0 && pixels <= 2_073_600).then_some((width, height, pixels))
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn frame(id: u64, frame: Frame, publish: impl FnOnce(&ExternalTexture, &Frame)) {
    let Some(state) = session(id) else { return };
    let Some((width, height, pixels)) = dimensions(frame.width, frame.height) else {
        return;
    };
    if frame.rgba.len() != pixels * 4 {
        return;
    }
    let frame = Arc::new(frame);
    let scan = {
        let mut capture = state
            .capture
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if capture.closed || capture.suspended {
            return;
        }
        capture.latest = Some(Arc::clone(&frame));
        if let Some(ready) = capture.ready.take() {
            let _ = ready.send(Ok(()));
        }
        publish(&state.texture, &frame);
        let now = Instant::now();
        let scan = capture.subscriber.is_some()
            && capture
                .last_scan
                .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(150));
        if scan {
            capture.last_scan = Some(now);
        }
        scan
    };
    if scan {
        let gray = frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| {
                let intensity = (u16::from(pixel[0]) * 77
                    + u16::from(pixel[1]) * 150
                    + u16::from(pixel[2]) * 29)
                    >> 8;
                intensity.to_le_bytes()[0]
            })
            .collect::<Vec<_>>();
        let mut scanner = quircs::Quirc::default();
        let codes = scanner
            .identify(width, height, &gray)
            .flatten()
            .filter_map(|code| code.decode().ok())
            .filter_map(|code| String::from_utf8(code.payload).ok());
        let mut capture = state
            .capture
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(subscriber) = capture.subscriber.as_mut() {
            for text in codes {
                if subscriber.try_send(QrCode { text }).is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn failed(id: u64) {
    if let Some(state) = session(id) {
        let mut capture = state
            .capture
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        capture.closed = true;
        if let Some(ready) = capture.ready.take() {
            let _ = ready.send(Err(CameraError::new("open", "Unable to open the camera")));
        }
        capture.subscriber = None;
        capture.latest = None;
        state.texture.clear();
    }
}

#[impasto::object]
pub struct CameraSession {
    id: u64,
    state: Arc<State>,
    platform: platform::Camera,
    _lifecycle: Subscription,
}

impl Drop for CameraSession {
    fn drop(&mut self) {
        SESSIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
        {
            let mut capture = self
                .state
                .capture
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            capture.closed = true;
            capture.subscriber = None;
        }
        self.platform.stop(self.id);
        self.state.texture.clear();
    }
}

struct Watcher<'a>(&'a State);

impl Drop for Watcher<'_> {
    fn drop(&mut self) {
        self.0
            .capture
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .subscriber = None;
    }
}

#[impasto::export]
impl CameraSession {
    pub fn texture_key(&self) -> String {
        self.state.texture.name().to_owned()
    }

    #[impasto::method(blocking)]
    pub fn capture(&self) -> Result<Bytes, CameraError> {
        let frame = self
            .state
            .capture
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .latest
            .clone()
            .ok_or_else(|| CameraError::new("frame", "No camera frame is available"))?;
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 88)
            .encode(
                &frame.rgba,
                u16::try_from(frame.width).map_err(|error| CameraError::new("frame", error))?,
                u16::try_from(frame.height).map_err(|error| CameraError::new("frame", error))?,
                jpeg_encoder::ColorType::Rgba,
            )
            .map_err(|error| CameraError::new("encode", error))?;
        Ok(Bytes(jpeg))
    }

    pub async fn watch_qr_codes(
        &self,
        mut sink: TypedSink<QrCode>,
        cancel: CancelToken,
    ) -> Result<(), CameraError> {
        let (tx, mut rx) = mpsc::channel(1);
        {
            let mut capture = self
                .state
                .capture
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if capture.closed || capture.subscriber.is_some() {
                return Err(CameraError::new(
                    "subscribe",
                    "Camera cannot start a second QR subscriber",
                ));
            }
            capture.subscriber = Some(tx);
        }
        let _watcher = Watcher(&self.state);
        while let Some(code) = rx.next().await {
            if cancel.is_cancelled() {
                break;
            }
            sink.send(code)
                .await
                .map_err(|error| CameraError::new("stream", error))?;
        }
        Ok(())
    }
}

#[impasto::module(specifier = "@impasto/camera/native", init = from_context)]
pub struct CameraModule {
    context: PluginContext,
}

impl CameraModule {
    fn from_context(context: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            context: context.clone(),
        })
    }
}

#[impasto::export]
impl CameraModule {
    #[impasto::method(resource)]
    pub async fn open_camera(&self, options: Options) -> Result<CameraSession, CameraError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| CameraError::unavailable())?;
        let platform = platform::Camera::new(&self.context, options.facing)?;
        let (ready, opened) = oneshot::channel();
        let state = Arc::new(State {
            texture: self
                .context
                .external_textures()
                .allocate()
                .map_err(|error| CameraError::new("texture", error))?,
            capture: Mutex::new(Capture {
                ready: Some(ready),
                ..Capture::default()
            }),
        });
        SESSIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id, Arc::downgrade(&state));
        let lifecycle = self.context.on_lifecycle({
            let state = Arc::downgrade(&state);
            let platform = platform.clone();
            move |event| {
                let Some(state) = state.upgrade() else { return };
                match event {
                    Lifecycle::Paused | Lifecycle::Hidden | Lifecycle::Detached => {
                        let mut capture = state
                            .capture
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        capture.suspended = true;
                        capture.latest = None;
                        platform.stop(id);
                        state.texture.clear();
                    }
                    Lifecycle::Resumed => {
                        let mut capture = state
                            .capture
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        capture.suspended = false;
                        if !capture.closed {
                            platform.resume(id, state.texture.name());
                        }
                    }
                    Lifecycle::Inactive => {}
                }
            }
        });
        let camera = CameraSession {
            id,
            state,
            platform,
            _lifecycle: lifecycle,
        };
        camera
            .platform
            .start(id, camera.state.texture.name())
            .await?;
        opened
            .await
            .map_err(|error| CameraError::new("open", error))??;
        Ok(camera)
    }
}

pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(CameraModule::plugin());
}
