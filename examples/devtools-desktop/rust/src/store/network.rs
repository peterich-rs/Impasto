use std::{collections::VecDeque, sync::Arc};

use impasto::ImpastoType;
use serde_json::Value;

use crate::analyzer::json_index;
use crate::types::{Row, RowSlice, clip, id, matches, number, string};

pub(crate) const TIMINGS: [(&str, &str); 8] = [
    ("queue_ms", "Queueing"),
    ("proxy_ms", "Proxy"),
    ("dns_ms", "DNS"),
    ("tcp_ms", "TCP"),
    ("tls_ms", "TLS"),
    ("request_sent_ms", "Request sent"),
    ("waiting_ttfb_ms", "TTFB"),
    ("download_ms", "Download"),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Side {
    Request,
    Response,
    Socket(u64),
}

impl Side {
    pub(crate) fn parse(side: &str) -> Option<Self> {
        match side {
            "request" => Some(Self::Request),
            "response" => Some(Self::Response),
            side => Some(Self::Socket(side.strip_prefix("ws:")?.parse().ok()?)),
        }
    }
}

#[derive(ImpastoType)]
pub struct BodySummary {
    pub mode: String,
    pub bytes: f64,
    pub truncated: bool,
    pub evicted: bool,
    pub available: bool,
    pub image_source: Option<String>,
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
}

pub(crate) struct Body {
    pub(crate) bytes: Vec<u8>,
    pub(crate) json: Option<json_index::Index>,
    pub(crate) mime: String,
    pub(crate) truncated: bool,
    pub(crate) evicted: bool,
    pub(crate) preview_only: bool,
    cost: usize,
}

impl Body {
    pub(crate) fn new(
        bytes: Vec<u8>,
        mime: String,
        truncated: bool,
        evicted: bool,
        preview_only: bool,
    ) -> Self {
        let json = json_index::Index::parse(&bytes);
        let cost = bytes.capacity() + json.as_ref().map_or(0, json_index::Index::memory_cost);
        Self {
            preview_only,
            cost,
            bytes,
            json,
            mime,
            truncated,
            evicted,
        }
    }

    pub(crate) fn summary(&self) -> BodySummary {
        let mode = if self.json.is_some() {
            "json"
        } else if self.image_extension().is_some() {
            "image"
        } else if std::str::from_utf8(&self.bytes).is_ok() && !self.bytes.contains(&0) {
            "text"
        } else {
            "hex"
        };
        BodySummary {
            mode: mode.to_owned(),
            bytes: count(self.bytes.len()),
            truncated: self.truncated,
            evicted: self.evicted,
            available: true,
            image_source: None,
            image_width: None,
            image_height: None,
        }
    }

    pub(crate) fn image_extension(&self) -> Option<&'static str> {
        match self.mime.split(';').next()? {
            "image/png" if self.bytes.starts_with(b"\x89PNG\r\n\x1a\n") => Some("png"),
            "image/jpeg" if self.bytes.starts_with(&[0xff, 0xd8]) => Some("jpg"),
            "image/webp" if self.bytes.get(8..12) == Some(b"WEBP") => Some("webp"),
            "image/gif" if self.bytes.starts_with(b"GIF8") => Some("gif"),
            _ => None,
        }
    }

    pub(crate) fn decode_image(
        &self,
    ) -> Result<(impasto::ExternalFrame, (u32, u32)), crate::DesktopError> {
        use image::GenericImageView as _;
        let mut reader =
            image::ImageReader::new(std::io::Cursor::new(&self.bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(32 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().map_err(crate::DesktopError::new)?;
        let size = image.dimensions();
        let preview = image.thumbnail(640, 640).into_rgba8();
        let frame = impasto::ExternalFrame {
            width: preview.width(),
            height: preview.height(),
            pixels: preview.into_raw(),
        };
        Ok((frame, size))
    }

    pub(crate) fn chunk(&self, offset: u32, limit: u32, hex: bool) -> String {
        use std::fmt::Write as _;
        let start = (offset as usize).min(self.bytes.len());
        let end = (start + limit.min(16_384) as usize).min(self.bytes.len());
        if !hex {
            if let Ok(text) = std::str::from_utf8(&self.bytes) {
                return text[text.ceil_char_boundary(start)..text.ceil_char_boundary(end)]
                    .to_owned();
            }
            return String::from_utf8_lossy(&self.bytes[start..end]).into_owned();
        }
        let mut result = String::new();
        for (index, row) in self.bytes[start..end].chunks(16).enumerate() {
            let _ = write!(result, "{:08x}  ", start + index * 16);
            for byte in row {
                let _ = write!(result, "{byte:02x} ");
            }
            for _ in row.len()..16 {
                result.push_str("   ");
            }
            result.push_str(" |");
            result.extend(row.iter().map(|byte| {
                if byte.is_ascii_graphic() || *byte == b' ' {
                    char::from(*byte)
                } else {
                    '.'
                }
            }));
            result.push_str("|\n");
        }
        result
    }
}

#[derive(Default)]
pub(crate) struct Bodies {
    entries: VecDeque<((u64, Side), Arc<Body>)>,
    bytes: usize,
}

impl Bodies {
    pub(crate) fn get(&self, key: (u64, Side)) -> Option<&Arc<Body>> {
        self.entries
            .iter()
            .find(|(entry, _)| *entry == key)
            .map(|(_, body)| body)
    }

    pub(crate) fn touch(&mut self, key: (u64, Side)) -> Option<Arc<Body>> {
        let index = self.entries.iter().position(|(entry, _)| *entry == key)?;
        let (key, body) = self.entries.remove(index)?;
        self.entries.push_back((key, Arc::clone(&body)));
        Some(body)
    }

    pub(crate) fn insert(&mut self, key: (u64, Side), body: Arc<Body>) {
        if let Some(index) = self.entries.iter().position(|(entry, _)| *entry == key)
            && let Some((_, old)) = self.entries.remove(index)
        {
            self.bytes -= old.cost;
        }
        self.bytes += body.cost;
        self.entries.push_back((key, body));
        // The entry cap also bounds the number of retained JSON indexes.
        while self.bytes > 64 * 1024 * 1024 || self.entries.len() > 64 {
            if let Some((_, body)) = self.entries.pop_front() {
                self.bytes -= body.cost;
            }
        }
    }
}

pub(crate) fn active(record: &Value) -> bool {
    matches!(
        string(record, "state"),
        "Pending" | "Streaming" | "pending" | "streaming"
    )
}

pub(crate) fn failed(record: &Value) -> bool {
    matches!(string(record, "state"), "Failed" | "failed")
        || record["status"]
            .as_u64()
            .is_some_and(|code| (400..600).contains(&code))
}

pub(crate) fn elapsed(record: &Value) -> f64 {
    record["done_ms"].as_f64().map_or_else(
        || number(&record["timing"], "total_ms"),
        |done| {
            // done_ms and at_ms share the agent's monotonic time origin.
            (done - number(record, "at_ms")).max(number(&record["timing"], "total_ms"))
        },
    )
}

pub(crate) fn filter(record: &Value, query: &str, kind: &str, status: &str) -> bool {
    let by_kind = match kind {
        "" | "all" => true,
        "image" => string(record, "initiator").to_lowercase().contains("image"),
        kind => record["kind"] == kind,
    };
    let by_status = match status {
        "" | "all" => true,
        "pending" => active(record),
        "error" => failed(record),
        "failed" => matches!(string(record, "state"), "Failed" | "failed"),
        bucket => bucket
            .as_bytes()
            .first()
            .and_then(|ch| ch.checked_sub(b'0'))
            .is_some_and(|code| {
                record["status"]
                    .as_u64()
                    .is_some_and(|status| status / 100 == u64::from(code))
            }),
    };
    by_kind
        && by_status
        && (query.is_empty()
            || ["url", "method", "initiator", "final_url", "error"]
                .iter()
                .any(|key| matches(string(record, key), query))
            || ["request_headers", "response_headers"]
                .iter()
                .any(|key| matches(&record[key].to_string(), query))
            || ["request_body_preview", "response_preview"]
                .iter()
                .any(|key| matches(&String::from_utf8_lossy(&preview(&record[key])), query)))
}

#[derive(ImpastoType)]
pub struct NetworkRow {
    pub id: String,
    pub method: String,
    pub url: String,
    pub kind: String,
    pub initiator: String,
    pub protocol: String,
    pub reused: bool,
    pub status: String,
    pub bytes: f64,
    pub elapsed_ms: f64,
    pub segments: Vec<f64>,
}

#[derive(ImpastoType)]
pub struct NetworkSlice {
    pub total: u32,
    pub bytes: f64,
    pub mean_ttfb: f64,
    pub rows: Vec<NetworkRow>,
}

pub(crate) struct Filter<'a> {
    pub query: &'a str,
    pub kind: &'a str,
    pub status: &'a str,
    pub sort: &'a str,
}

pub(crate) fn slice<'a>(
    records: impl Iterator<Item = &'a Value>,
    filter_by: Filter<'_>,
    offset: u32,
    limit: u32,
    now: f64,
) -> NetworkSlice {
    let Filter {
        query,
        kind,
        status,
        sort,
    } = filter_by;
    let query = query.to_lowercase();
    let mut filtered: Vec<_> = records
        .filter(|record| filter(record, &query, kind, status))
        .collect();
    if sort != "start" {
        let score = |record: &Value| match sort {
            "time" => elapsed(record),
            "size" => number(record, "response_bytes"),
            "status" => number(record, "status"),
            _ => number(record, "at_ms"),
        };
        filtered.sort_by(|a, b| score(b).total_cmp(&score(a)));
    }
    let mut sum = 0.;
    let mut timed = 0;
    let mut bytes = 0.;
    let elapsed_at = |record: &Value| {
        if active(record) {
            (now - number(record, "at_ms")).max(elapsed(record))
        } else {
            elapsed(record)
        }
    };
    let waterline = filtered
        .iter()
        .map(|record| elapsed_at(record))
        .fold(1_000., f64::max);
    for record in &filtered {
        bytes += number(record, "response_bytes");
        if let Some(ttfb) = record["timing"]["waiting_ttfb_ms"].as_f64() {
            sum += ttfb;
            timed += 1;
        }
    }
    NetworkSlice {
        total: u32::try_from(filtered.len()).unwrap_or(u32::MAX),
        mean_ttfb: if timed > 0 {
            sum / f64::from(timed)
        } else {
            0.
        },
        bytes,
        rows: filtered
            .into_iter()
            .skip(offset as usize)
            .take(limit.min(256) as usize)
            .map(|record| {
                let elapsed_ms = elapsed_at(record);
                let timing = &record["timing"];
                let total = TIMINGS
                    .iter()
                    .map(|(key, _)| number(timing, key).max(0.))
                    .sum::<f64>()
                    .max(waterline);
                let mut start = 0.;
                let mut segments = Vec::new();
                for (index, (key, _)) in TIMINGS.iter().enumerate() {
                    let duration = number(timing, key).max(0.) / total;
                    if duration > 0. {
                        segments.extend([start, duration, 16. + count(index)]);
                        start += duration;
                    }
                }
                if active(record) && start < elapsed_ms / total {
                    segments.extend([start, elapsed_ms / total - start, 16.]);
                }
                if failed(record) && segments.is_empty() {
                    segments.extend([0., 1., 24.]);
                }
                NetworkRow {
                    id: id(&record["token"]),
                    method: string(record, "method").to_owned(),
                    url: clip(string(record, "url"), 512),
                    kind: string(record, "kind").to_owned(),
                    initiator: string(record, "initiator").to_owned(),
                    protocol: string(&record["connection"], "protocol").to_owned(),
                    reused: record["connection"]["reused"] == true,
                    status: record["status"].as_u64().map_or_else(
                        || string(record, "state").to_owned(),
                        |code| code.to_string(),
                    ),
                    bytes: number(record, "response_bytes"),
                    elapsed_ms,
                    segments,
                }
            })
            .collect(),
    }
}

pub(crate) fn ws_rows<'a>(
    entries: impl Iterator<Item = &'a Value>,
    direction: &str,
    query: &str,
    offset: u32,
    limit: u32,
) -> RowSlice {
    let query = query.to_lowercase();
    let entries = entries.filter_map(|entry| {
        if !direction.is_empty() && entry["direction"] != direction {
            return None;
        }
        let bytes = preview(&entry["preview"]);
        let preview = String::from_utf8_lossy(&bytes);
        (matches(&preview, &query) || matches(string(entry, "opcode"), &query))
            .then(|| (entry, clip(&preview, 160)))
    });
    RowSlice::page(entries, offset, limit, |(entry, preview)| Row {
        id: id(&entry["seq"]),
        name: string(entry, "opcode").to_owned(),
        kind: string(entry, "direction").to_owned(),
        detail: preview,
        at_ms: number(entry, "at_ms"),
        count: number(entry, "bytes_len"),
        ..Row::default()
    })
}

pub(crate) fn preview(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(4096)
        .filter_map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

pub(crate) fn content_type(record: &Value, side: Side) -> String {
    let key = match side {
        Side::Request => "request_headers",
        Side::Response => "response_headers",
        Side::Socket(_) => return String::new(),
    };
    record[key]
        .as_array()
        .into_iter()
        .flatten()
        .find(|header| {
            header[0]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("content-type"))
        })
        .and_then(|header| header[1].as_str())
        .unwrap_or_default()
        .to_lowercase()
}

fn count(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}
