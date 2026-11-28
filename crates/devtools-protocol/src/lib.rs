//! Wire types shared by DevTools agents and clients.

mod binary;
mod frame;

pub use binary::{
    BINARY_FLAG_ZSTD, BINARY_HEADER_LEN, BINARY_MAGIC, BinaryEnvelopeHeader, BinaryError,
    BinaryMsgType,
};
pub use frame::{CompositorBlock, FrameRecord, GpuPass, GpuTiming, RuntimeSample};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Legacy frames and inline trace capture.
pub const PROTOCOL_VERSION_V1: u8 = 1;
/// Multi-channel telemetry, UI queries and correlated binary payloads.
pub const PROTOCOL_VERSION_V2: u8 = 2;
/// Largest payload sent in a single binary message.
pub const MAX_BINARY_PAYLOAD: usize = 16 * 1024 * 1024;

/// Negotiated features of a specific agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    FrameSpans,
    CaptureStatus,
    TraceDetail,
    CaptureGpu,
    Inspector,
    MultiChannelSubscribe,
    BinaryFrames,
    ViewRepaintTracker,
    RemoteOverlay,
    NetworkTimings,
    NetworkBodies,
    JsRuntimeDetail,
    SourceMapSymbols,
}

/// Independently resumed telemetry streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Frames,
    Repaints,
    Network,
    Logs,
    Errors,
    Runtime,
    Resources,
}

/// Missing cursors start with a full snapshot for the requested channel.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChannelCursors {
    pub frames: Option<u64>,
    pub repaints: Option<u64>,
    pub network: Option<u64>,
    pub logs: Option<u64>,
    pub errors: Option<u64>,
    pub runtime: Option<u64>,
    pub resources: Option<u64>,
}

impl ChannelCursors {
    /// Access the cursor for the specified channel.
    pub fn cursor(&mut self, channel: Channel) -> &mut Option<u64> {
        match channel {
            Channel::Frames => &mut self.frames,
            Channel::Repaints => &mut self.repaints,
            Channel::Network => &mut self.network,
            Channel::Logs => &mut self.logs,
            Channel::Errors => &mut self.errors,
            Channel::Runtime => &mut self.runtime,
            Channel::Resources => &mut self.resources,
        }
    }
}

/// One channel's changes since its last acknowledged revision.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelUpdate<T> {
    pub revision: u64,
    pub reset: bool,
    pub entries: Vec<T>,
}

/// Process identity and current viewport, obtained after authentication.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub pid: u32,
    pub app_name: String,
    pub platform: String,
    pub os: String,
    pub framework_version: String,
    pub width: f32,
    pub height: f32,
    pub dpr: f32,
    pub refresh_rate: Option<f32>,
    pub topology: Option<String>,
}

/// Private local registration written by a live agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSessionEntry {
    pub pid: u32,
    pub app_name: String,
    pub platform: String,
    pub port: u16,
    pub token: String,
    pub protocol_version: u8,
    pub project_root: Option<PathBuf>,
    pub framework_root: Option<PathBuf>,
    pub source_map: Option<PathBuf>,
    pub started_at_ms: u64,
}

/// Private registration for a device whose agent is reached over a tunnel.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSessionEntry {
    pub device_kind: DeviceKind,
    pub serial: String,
    pub package: String,
    pub remote_port: u16,
    pub token: String,
    pub project_root: PathBuf,
    #[serde(default)]
    pub source_map: Option<PathBuf>,
}

/// Device transport selected by the CLI.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Android,
    Ios,
}

/// Diagnostics layers applied to the recorded scene.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    pub highlight_node: Option<String>,
    pub pick_mode: bool,
    pub repaint_rainbow: bool,
    pub layout_scan: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_device_registrations_default_to_no_source_map() -> Result<(), serde_json::Error> {
        let wire = r#"{"deviceKind":"android","serial":"emulator-5554","package":"dev.impasto",
            "remotePort":8765,"token":"test","projectRoot":"/project"}"#;
        let entry: DeviceSessionEntry = serde_json::from_str(wire)?;
        assert_eq!(entry.serial, "emulator-5554");
        assert_eq!(entry.source_map, None);
        Ok(())
    }
}
