use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};

use devtools_protocol::{
    BinaryEnvelopeHeader, Channel, ChannelUpdate, FrameRecord, MAX_BINARY_PAYLOAD,
    PROTOCOL_VERSION_V2, SessionInfo,
};
use devtools_symbols::SymbolTable;
use impasto::ImpastoType;
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::discovery::{Target, Tunnel};

const IO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(ImpastoType)]
pub struct Snapshot {
    pub app_name: String,
    pub platform: String,
    pub pid: u32,
    pub width: f64,
    pub height: f64,
    pub refresh_rate: Option<f64>,
    pub frame_number: Option<String>,
    pub ui_ms: Option<f64>,
    pub raster_ms: Option<f64>,
    pub budget_ms: Option<f64>,
}

pub(crate) struct Connection {
    socket: WebSocket<TcpStream>,
    _tunnel: Option<Tunnel>,
    next_id: u64,
    last_frame: Option<FrameRecord>,
    pub(crate) symbols: Option<SymbolTable>,
}

impl Connection {
    pub(crate) fn open(target: &Target) -> Result<(Self, Snapshot), String> {
        let (port, token, tunnel) = target.connection()?;
        let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
        let deadline = Instant::now() + IO_TIMEOUT;
        let stream = loop {
            match TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
                Ok(stream) => break stream,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(30))
                }
                Err(error) => return Err(format!("Connect to target: {}", error.kind())),
            }
        };
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|error| error.to_string())?;
        let (socket, _) = tungstenite::client::client_with_config(
            format!("ws://{address}/devtools"),
            stream,
            Some(
                tungstenite::protocol::WebSocketConfig::default()
                    .max_message_size(Some(MAX_BINARY_PAYLOAD + 20))
                    .max_frame_size(Some(MAX_BINARY_PAYLOAD + 20)),
            ),
        )
        .map_err(|_| "DevTools handshake failed".to_owned())?;
        let mut connection = Self {
            socket,
            _tunnel: tunnel,
            next_id: 0,
            last_frame: None,
            symbols: None,
        };
        let auth = connection.call(
            "authenticate",
            json!({"token": token, "version": PROTOCOL_VERSION_V2}),
        )?;
        let info =
            serde_json::from_value(auth["session"].clone()).map_err(|error| error.to_string())?;
        let batch = connection.call("subscribe", json!({"channels": [Channel::Frames]}))?;
        connection.note_frames(&batch["frames"])?;
        connection.symbols = target
            .symbol_path()
            .and_then(|path| SymbolTable::load(path).ok());
        let snapshot = connection.snapshot_with(info);
        Ok((connection, snapshot))
    }

    pub(crate) fn snapshot(&mut self) -> Result<Snapshot, String> {
        let info = serde_json::from_value(self.call("session.info", Value::Null)?)
            .map_err(|error| error.to_string())?;
        Ok(self.snapshot_with(info))
    }

    fn snapshot_with(&self, info: SessionInfo) -> Snapshot {
        let frame = self.last_frame.as_ref();
        Snapshot {
            app_name: info.app_name,
            platform: info.platform,
            pid: info.pid,
            width: f64::from(info.width),
            height: f64::from(info.height),
            refresh_rate: info.refresh_rate.map(f64::from),
            frame_number: frame.map(|frame| frame.id.to_string()),
            ui_ms: frame.map(|frame| frame.ui_ms),
            raster_ms: frame.map(|frame| frame.raster_ms),
            budget_ms: frame.map(|frame| frame.budget_ms),
        }
    }

    fn note_frames(&mut self, update: &Value) -> Result<(), String> {
        let batch: ChannelUpdate<FrameRecord> =
            serde_json::from_value(update.clone()).map_err(|error| error.to_string())?;
        if batch.reset {
            self.last_frame = None;
        }
        if let Some(latest) = batch.entries.into_iter().max_by_key(|frame| frame.id)
            && self
                .last_frame
                .as_ref()
                .is_none_or(|frame| frame.id <= latest.id)
        {
            self.last_frame = Some(latest);
        }
        Ok(())
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0", "id": id, "method": method, "params": params,
                })
                .to_string()
                .into(),
            ))
            .map_err(|error| error.to_string())?;
        let started = Instant::now();
        while started.elapsed() < IO_TIMEOUT {
            match self.socket.read().map_err(|error| error.to_string())? {
                Message::Text(message) => {
                    let value: Value =
                        serde_json::from_str(&message).map_err(|error| error.to_string())?;
                    if value["id"] == id {
                        return value.get("result").cloned().ok_or_else(|| {
                            value["error"]["message"]
                                .as_str()
                                .unwrap_or("invalid DevTools response")
                                .to_owned()
                        });
                    }
                    if value["method"] == "stream.batch"
                        && let Some(frames) = value["params"].get("frames")
                    {
                        self.note_frames(frames)?;
                    }
                }
                Message::Binary(bytes) => {
                    let (header, _) =
                        BinaryEnvelopeHeader::decode(&bytes).map_err(|error| error.to_string())?;
                    if header.request_id == id {
                        return Err("unexpected binary response to a text request".to_owned());
                    }
                }
                Message::Close(_) => return Err("target disconnected".to_owned()),
                Message::Ping(_) => self.socket.flush().map_err(|error| error.to_string())?,
                _ => {}
            }
        }
        Err("DevTools request timed out".to_owned())
    }
}
