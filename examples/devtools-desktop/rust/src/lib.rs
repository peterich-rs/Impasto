use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLockReadGuard};

use devtools_protocol::{BinaryMsgType, Channel, ChannelUpdate, OverlayConfig};
use impasto::{
    AppPaths, CancelToken, ImpastoType, PluginContext, PluginInitError, PluginRegistrar, Runtime,
    TypedSink,
};
use serde_json::{Value, json};

mod actions;
mod analyzer;
mod archive;
mod client;
mod discovery;
mod error;
mod projection;
mod store;
mod types;

pub use error::DesktopError;

use analyzer::{audits::Finding, flamegraph};
use archive::{Artifact, ArtifactSummary};
use client::{Client, Payload};
use discovery::TargetSummary;
use projection::{Hit, ProjectedCanvas, Viewport};
use store::{
    ConnectionState, Shared, Store, frames,
    network::{self, Body, BodySummary, NetworkSlice, Side},
    views,
};
use types::{
    DetailField, FrameStats, Row, RowSlice, SessionOverview, TickSummary, fields, id, matches,
    number, string,
};

#[impasto::module(specifier = "@app/devtools-core", init = from_context)]
pub struct Desktop {
    runtime: Runtime,
    paths: AppPaths,
    state: Shared,
    client: Client,
    staging: Arc<tempfile::TempDir>,
    textures: impasto::ExternalTextureRegistry,
    profiles: Arc<Mutex<ProfileCache>>,
    previews: Mutex<std::collections::VecDeque<impasto::ExternalTexture>>,
}

#[derive(Default)]
struct ProfileCache {
    entries: std::collections::VecDeque<(ProfileKey, Arc<flamegraph::Tree>)>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct ProfileKey {
    generation: u64,
    stack_revision: u32,
    frame_revision: u32,
    frame: Option<u64>,
}

#[derive(ImpastoType)]
pub struct Overlay {
    pub highlight_node: Option<String>,
    pub pick_mode: bool,
    pub repaint_rainbow: bool,
    pub layout_scan: bool,
}

#[derive(ImpastoType)]
pub struct FrameComparison {
    pub selected: FrameStats,
    pub all: FrameStats,
    pub selected_ui_ms: f64,
    pub selected_raster_ms: f64,
    pub all_ui_ms: f64,
    pub all_raster_ms: f64,
}

impl Desktop {
    fn from_context(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        let state = Shared::default();
        let paths = ctx.paths().clone();
        let cache = paths.cache_dir().map_err(PluginInitError::new)?;
        std::fs::create_dir_all(cache).map_err(PluginInitError::new)?;
        let staging = tempfile::Builder::new()
            .prefix("devtools-")
            .tempdir_in(cache)
            .map_err(PluginInitError::new)?;
        Ok(Self {
            runtime: ctx.runtime().clone(),
            paths,
            client: Client::spawn(Arc::clone(&state)).map_err(PluginInitError::new)?,
            state,
            staging: Arc::new(staging),
            textures: ctx.external_textures().clone(),
            profiles: Arc::default(),
            previews: Mutex::default(),
        })
    }

    fn read(&self) -> RwLockReadGuard<'_, Store> {
        self.state.read().unwrap_or_else(|error| error.into_inner())
    }

    async fn io<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, DesktopError> + Send + 'static,
    ) -> Result<T, DesktopError> {
        self.runtime
            .spawn_blocking(work)
            .await
            .map_err(|error| DesktopError::with_code("io", error))?
    }

    async fn rpc(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<(u64, Payload), DesktopError> {
        let generation = self.read().generation;
        self.client
            .rpc(generation, method, params)
            .await
            .map(|payload| (generation, payload))
    }

    fn update<T>(
        &self,
        generation: u64,
        change: impl FnOnce(&mut Store) -> T,
    ) -> Result<T, DesktopError> {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if state.generation != generation {
            return Err(DesktopError::disconnected());
        }
        Ok(change(&mut state))
    }

    async fn refresh(
        &self,
        method: &'static str,
        params: Value,
        kind: Option<BinaryMsgType>,
        channel: Channel,
        assign: impl FnOnce(&mut Store, Value) + Send,
    ) -> Result<(), DesktopError> {
        let (generation, payload) = self.rpc(method, params).await?;
        let value = self.io(move || payload.json(kind)).await?;
        self.update(generation, |state| {
            assign(state, value);
            state.changed(channel);
        })
    }

    async fn trace_bytes(&self, id: u64) -> Result<Arc<[u8]>, DesktopError> {
        {
            let state = self.read();
            if id == 0
                && let Some(bytes) = &state.imported_trace
            {
                return Ok(Arc::clone(bytes));
            }
            if let Some((cached_id, bytes)) = &state.trace
                && *cached_id == id
            {
                return Ok(Arc::clone(bytes));
            }
        }
        let (generation, payload) = self
            .rpc("trace.export_pftrace", json!({"capture_id": id}))
            .await?;
        let bytes: Arc<[u8]> = payload.binary(BinaryMsgType::TraceExport)?.into();
        self.update(generation, |state| {
            state.trace = Some((id, Arc::clone(&bytes)));
        })?;
        Ok(bytes)
    }

    async fn stage(
        &self,
        suffix: &'static str,
        write: impl FnOnce(std::fs::File) -> Result<(), DesktopError> + Send + 'static,
    ) -> Result<String, DesktopError> {
        let base = self
            .paths
            .cache_dir()
            .map_err(DesktopError::new)?
            .to_owned();
        let staging = Arc::clone(&self.staging);
        self.io(move || {
            let file = tempfile::Builder::new()
                .prefix("capture-")
                .suffix(suffix)
                .tempfile_in(staging.path())?;
            write(file.reopen()?)?;
            let relative = file
                .path()
                .strip_prefix(&base)
                .map_err(DesktopError::new)?
                .to_string_lossy()
                .into_owned();
            file.keep().map_err(DesktopError::new)?;
            Ok(relative)
        })
        .await
    }

    async fn build_profile(
        &self,
        frame: Option<u64>,
    ) -> Result<Arc<flamegraph::Tree>, DesktopError> {
        let state = Arc::clone(&self.state);
        let cache = Arc::clone(&self.profiles);
        self.io(move || {
            let (key, mut stacks, captured, symbols) = {
                let state = state.read().unwrap_or_else(|error| error.into_inner());
                let (frame_revision, stack_revision) = state.profile_revision();
                let key = ProfileKey {
                    generation: state.generation,
                    stack_revision,
                    frame_revision: if frame.is_some() { frame_revision } else { 0 },
                    frame,
                };
                let cache = cache.lock().unwrap_or_else(|error| error.into_inner());
                if let Some((_, tree)) = cache
                    .entries
                    .iter()
                    .find(|(candidate, _)| *candidate == key)
                {
                    return Ok(Arc::clone(tree));
                }
                let stacks: Vec<Value> = state
                    .data
                    .stacks
                    .iter()
                    .filter(|sample| {
                        frame.is_none_or(|target| {
                            sample["value"]["frame_id"].as_u64() == Some(target)
                        })
                    })
                    .cloned()
                    .collect();
                let captured = frame.and_then(|id| state.data.spans.get(&id)).cloned();
                (key, stacks, captured, state.symbols.clone())
            };
            if stacks.is_empty()
                && let Some(capture) = captured
            {
                stacks.extend(
                    capture["spans"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|span| {
                            let stack = span["fields"]["js_stack"].as_str()?;
                            Some(json!({"value": {"frame_id": frame, "stack": stack}}))
                        }),
                );
            }
            let resolver = symbols.as_ref().map(|symbols| symbols.resolver());
            let tree = Arc::new(flamegraph::Tree::build(
                stacks.iter(),
                resolver.as_ref(),
                frame,
            ));
            let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
            cache.entries.retain(|(candidate, _)| {
                candidate.generation == key.generation && candidate.frame != key.frame
            });
            cache.entries.push_back((key, Arc::clone(&tree)));
            if cache.entries.len() > 3 {
                cache.entries.pop_front();
            }
            Ok(tree)
        })
        .await
    }
}

fn parse_id(key: &str) -> Result<u64, DesktopError> {
    key.parse()
        .map_err(|_| DesktopError::new("Invalid record identity"))
}

fn parse_side(side: &str) -> Result<Side, DesktopError> {
    Side::parse(side).ok_or_else(|| DesktopError::new("Invalid request body side"))
}

fn json_fields(value: impl serde::Serialize) -> Vec<DetailField> {
    fields(&serde_json::to_value(value).unwrap_or_default())
}

fn log_rows<'a>(
    entries: impl Iterator<Item = &'a Value>,
    search: &str,
    level: &str,
    source: &str,
    frame: &str,
    offset: u32,
    limit: u32,
) -> RowSlice {
    let search = search.to_lowercase();
    let entries = entries.filter(|entry| {
        (level.is_empty() || string(entry, "level").eq_ignore_ascii_case(level))
            && (source.is_empty() || entry["source"] == source)
            && (frame.is_empty() || id(&entry["frame"]) == frame)
            && (matches(string(entry, "message"), &search)
                || matches(string(entry, "target"), &search)
                || matches(string(entry, "stack"), &search))
    });
    RowSlice::page(entries, offset, limit, |entry| Row {
        id: id(&entry["seq"]),
        name: types::clip(string(entry, "message"), 320),
        kind: string(entry, "level").to_owned(),
        detail: string(entry, "source").to_owned(),
        meta: id(&entry["frame"]),
        at_ms: number(entry, "at_ms"),
        ..Row::default()
    })
}

#[impasto::export]
impl Desktop {
    pub fn preferred_appearance() -> String {
        match std::env::var("IMPASTO_DEVTOOLS_APPEARANCE").as_deref() {
            Ok("light") => "light",
            Ok("dark") => "dark",
            _ => "system",
        }
        .to_owned()
    }

    pub fn preferred_target() -> Option<String> {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--attach" {
                return args.next();
            }
        }
        None
    }

    pub async fn discover(&self) -> Result<Vec<TargetSummary>, DesktopError> {
        self.io(|| {
            Ok(discovery::targets()
                .iter()
                .map(discovery::Target::summary)
                .take(128)
                .collect())
        })
        .await
    }

    pub async fn attach(&self, target_id: String) -> Result<SessionOverview, DesktopError> {
        self.client.attach(target_id).await
    }

    pub async fn disconnect(&self) -> Result<(), DesktopError> {
        self.client.disconnect().await
    }

    pub async fn reconnect_now(&self) -> Result<(), DesktopError> {
        self.client.reconnect().await
    }

    pub async fn watch_ticks(
        &self,
        sink: TypedSink<TickSummary>,
        cancel: CancelToken,
    ) -> Result<(), DesktopError> {
        self.client.watch(sink, cancel.clone())?;
        cancel.cancelled().await;
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), DesktopError> {
        self.client.pause(paused)
    }

    pub fn clear_session(&self, scope: String) -> Result<(), DesktopError> {
        if self.read().state == ConnectionState::Offline {
            return Err(DesktopError::new("This recording is read-only"));
        }
        if !matches!(
            scope.as_str(),
            "all" | "frames" | "network" | "logs" | "repaints"
        ) {
            return Err(DesktopError::new("Invalid clear scope"));
        }
        self.client.clear(scope)
    }

    pub async fn set_active_channels(&self, channels: Vec<String>) -> Result<(), DesktopError> {
        let mut active = client::DEFAULT_CHANNELS.to_vec();
        for name in channels {
            let channel = store::CHANNELS
                .iter()
                .find(|(_, label)| *label == name)
                .map(|(channel, _)| *channel)
                .ok_or_else(|| DesktopError::new("Unknown telemetry channel"))?;
            if !active.contains(&channel) {
                active.push(channel);
            }
        }
        self.client.channels(active).await
    }

    pub async fn set_profile(
        &self,
        profile: String,
        repaint_tracking: Option<bool>,
    ) -> Result<(), DesktopError> {
        let mut params = json!({"profile": profile});
        if let Some(repaint_tracking) = repaint_tracking {
            params["repaint_tracking"] = json!(repaint_tracking);
        }
        let (generation, payload) = self.rpc("telemetry.set_profile", params).await?;
        payload.json(None)?;
        self.update(generation, |state| {
            state.profile = profile;
            state.touch();
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the viewport projection export takes canvas coordinates and selected records"
    )]
    pub fn project_frame_strip(
        &self,
        width: f64,
        height: f64,
        start_index: u32,
        visible_count: u32,
        selected: Option<String>,
        range_start: Option<String>,
        range_end: Option<String>,
    ) -> ProjectedCanvas {
        projection::frame_strip(
            &self.read().data,
            width,
            height,
            start_index,
            visible_count,
            selected.and_then(|id| id.parse().ok()),
            range_start
                .and_then(|id| id.parse().ok())
                .zip(range_end.and_then(|id| id.parse().ok())),
        )
    }

    pub fn hit_test_frame_strip(
        &self,
        x: f64,
        width: f64,
        start_index: u32,
        visible_count: u32,
    ) -> Option<Hit> {
        projection::hit_frame_strip(&self.read().data, x, width, start_index, visible_count)
    }

    pub fn get_frame_detail(&self, frame_id: String) -> Vec<DetailField> {
        frame_id
            .parse()
            .ok()
            .and_then(|id| {
                self.read()
                    .data
                    .frame(id)
                    .map(|frame| json_fields(frame.as_ref()))
            })
            .unwrap_or_default()
    }

    pub fn frame_revision(&self, frame_id: String) -> Option<String> {
        let id = frame_id.parse().ok()?;
        let state = self.read();
        let data = &state.data;
        let revision = data.frame_revision(id)?;
        let capture = data.capture["id"].as_u64().unwrap_or_default();
        let has_spans = data.spans.contains_key(&id);
        Some(format!(
            "{}:{id}:{revision}:{capture}:{has_spans}",
            state.generation
        ))
    }

    pub fn frame_context(&self, frame_id: String) -> Vec<Row> {
        let state = self.read();
        let Ok(frame) = frame_id.parse::<u64>() else {
            return Vec::new();
        };
        let Some(record) = state.data.frame(frame) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        if let Some(block) = &record.compositor_block
            && let Some(node) = &block.node
        {
            result.push(Row {
                id: node.clone(),
                name: block.reason.clone(),
                kind: "block".to_owned(),
                ..Row::default()
            });
        }
        for sample in state
            .data
            .stacks
            .iter()
            .rev()
            .filter(|sample| sample["value"]["frame_id"].as_u64() == Some(frame))
            .take(4)
        {
            let raw = string(&sample["value"], "stack");
            let stack = state
                .symbols
                .as_ref()
                .map_or_else(|| raw.to_owned(), |symbols| symbols.resolver().remap(raw));
            result.push(Row {
                id: id(&sample["seq"]),
                name: string(&sample["value"], "entry").to_owned(),
                detail: types::clip(&stack, 512),
                kind: "stack".to_owned(),
                ..Row::default()
            });
        }
        result
    }

    pub fn frame_near_time(&self, time: f64) -> Option<String> {
        self.read()
            .data
            .frames
            .entries()
            .iter()
            .min_by(|left, right| {
                (left.at_ms - time)
                    .abs()
                    .total_cmp(&(right.at_ms - time).abs())
            })
            .map(|frame| frame.id.to_string())
    }

    pub fn frame_at(&self, index: u32) -> Option<String> {
        self.read()
            .data
            .frames
            .entries()
            .get(index as usize)
            .map(|frame| frame.id.to_string())
    }

    pub fn step_frame(&self, frame_id: Option<String>, direction: f64) -> Option<String> {
        let store = self.read();
        let frames = store.data.frames.entries();
        let current = frame_id
            .and_then(|id| id.parse::<u64>().ok())
            .and_then(|id| frames.binary_search_by_key(&id, |frame| frame.id).ok());
        match (current, direction.is_sign_negative()) {
            (Some(index), true) => frames.get(index.saturating_sub(1)),
            (Some(index), false) => frames.get(index + 1).or_else(|| frames.back()),
            (None, _) => frames.back(),
        }
        .map(|frame| frame.id.to_string())
    }

    pub fn worst_frame(&self) -> Option<String> {
        self.read()
            .data
            .frames
            .entries()
            .iter()
            .max_by(|a, b| frames::stage_duration(a).total_cmp(&frames::stage_duration(b)))
            .map(|frame| frame.id.to_string())
    }

    pub fn compare_frame_range(
        &self,
        start: String,
        end: String,
    ) -> Result<FrameComparison, DesktopError> {
        let (a, b) = (parse_id(&start)?, parse_id(&end)?);
        let state = self.read();
        let all = state.data.frames.entries();
        let selected = || {
            all.iter()
                .filter(|frame| (a.min(b)..=a.max(b)).contains(&frame.id))
        };
        let selected_stats = frames::stats(selected(), state.data.frame_budget());
        let all_stats = frames::stats(all.iter(), state.data.frame_budget());
        let average = |sum: f64, count: u32| sum / f64::from(count.max(1));
        Ok(FrameComparison {
            selected_ui_ms: average(
                selected().map(|frame| frame.ui_ms).sum(),
                selected_stats.count,
            ),
            selected_raster_ms: average(
                selected().map(|frame| frame.raster_ms).sum(),
                selected_stats.count,
            ),
            all_ui_ms: average(all.iter().map(|frame| frame.ui_ms).sum(), all_stats.count),
            all_raster_ms: average(
                all.iter().map(|frame| frame.raster_ms).sum(),
                all_stats.count,
            ),
            selected: selected_stats,
            all: all_stats,
        })
    }

    pub fn project_frame_gantt(
        &self,
        frame_id: String,
        width: f64,
        height: f64,
        start: f64,
        end: f64,
    ) -> ProjectedCanvas {
        frame_id
            .parse()
            .ok()
            .map(|id| {
                projection::gantt(
                    &self.read().data,
                    id,
                    Viewport {
                        width,
                        height,
                        start,
                        end,
                    },
                    None,
                )
                .0
            })
            .unwrap_or_default()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the viewport projection export takes canvas coordinates and selected records"
    )]
    pub fn hit_test_frame_gantt(
        &self,
        frame_id: String,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        start: f64,
        end: f64,
    ) -> Option<Hit> {
        projection::gantt(
            &self.read().data,
            frame_id.parse().ok()?,
            Viewport {
                width,
                height,
                start,
                end,
            },
            Some((x, y)),
        )
        .1
    }

    pub async fn trace_start(
        &self,
        detail: String,
        gpu: bool,
        js_sampling: bool,
    ) -> Result<String, DesktopError> {
        let (generation, payload) = self
            .rpc(
                "trace.start",
                json!({"detail": detail, "gpu": gpu, "js_sampling": js_sampling}),
            )
            .await?;
        let id = payload.json(None)?["id"]
            .as_u64()
            .ok_or_else(|| DesktopError::new("Invalid capture identity"))?;
        let start_sampler = self.update(generation, |state| {
            state.capture_id = Some(id);
            state.touch();
            js_sampling && !state.sampler_enabled
        })?;
        if start_sampler && self.client.rpc(generation, "js.set_sampler_config",
            json!({"enabled": true, "interval_us": 1000, "max_depth": 32, "max_samples": 160})).await.is_ok() {
            self.update(generation, |state| { state.capture_sampler = state.capture_id == Some(id); })?;
        }
        Ok(id.to_string())
    }

    pub async fn trace_stop(&self) -> Result<(), DesktopError> {
        let (generation, payload) = self.rpc("trace.stop", Value::Null).await?;
        let capture = payload.json(None)?;
        let stop_sampler = self.update(generation, |state| {
            state.capture_id = None;
            state.data.capture = capture;
            state.data.spans.clear();
            state.trace = None;
            state.changed(Channel::Frames);
            std::mem::take(&mut state.capture_sampler)
        })?;
        if stop_sampler {
            self.client
                .rpc(
                    generation,
                    "js.set_sampler_config",
                    json!({"enabled": false}),
                )
                .await?
                .json(None)?;
        }
        Ok(())
    }

    pub fn capture_summary(&self, offset: u32, limit: u32) -> RowSlice {
        let state = self.read();
        let mut rows: Vec<_> = state.data.capture["frames"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        rows.sort_by(|a, b| number(b, "e2e_ms").total_cmp(&number(a, "e2e_ms")));
        RowSlice::page(rows, offset, limit, |frame| Row {
            id: id(&frame["id"]),
            name: format!("Frame #{}", id(&frame["id"])),
            count: number(frame, "e2e_ms"),
            detail: format!("{:.2} ms", number(frame, "e2e_ms")),
            ..Row::default()
        })
    }

    pub fn capture_detail(&self) -> Vec<DetailField> {
        let state = self.read();
        let capture = &state.data.capture;
        if capture.is_null() {
            return Vec::new();
        }
        let mut metadata = capture.clone();
        if let Some(map) = metadata.as_object_mut() {
            map.remove("frames");
            let count = map
                .remove("frame_ids")
                .and_then(|frames| frames.as_array().map(Vec::len))
                .unwrap_or(0);
            map.insert("frames".to_owned(), json!(count));
        }
        fields(&metadata)
    }

    pub fn capture_contains(&self, frame_id: String) -> bool {
        let Ok(id) = frame_id.parse::<u64>() else {
            return false;
        };
        self.read().data.capture["frame_ids"]
            .as_array()
            .is_some_and(|frames| frames.iter().any(|frame| frame.as_u64() == Some(id)))
    }

    pub async fn load_frame_spans(
        &self,
        capture_id: String,
        frame_id: String,
    ) -> Result<(), DesktopError> {
        let frame = parse_id(&frame_id)?;
        if self.read().data.spans.contains_key(&frame) {
            return Ok(());
        }
        self.refresh(
            "trace.get_frame_spans",
            json!({"capture_id": parse_id(&capture_id)?, "frame_id": frame}),
            None,
            Channel::Frames,
            move |state, value| {
                state.data.spans.insert(frame, value);
                store::trim_map(&mut state.data.spans, 96);
            },
        )
        .await
    }

    pub fn omitted_spans(&self, frame_id: String) -> f64 {
        frame_id
            .parse::<u64>()
            .ok()
            .and_then(|id| {
                self.read()
                    .data
                    .spans
                    .get(&id)
                    .map(|capture| number(capture, "omitted_spans"))
            })
            .unwrap_or_default()
    }

    pub async fn stage_pftrace_export(&self, capture_id: String) -> Result<String, DesktopError> {
        let bytes = self.trace_bytes(parse_id(&capture_id)?).await?;
        self.stage(".pftrace", move |mut file| {
            file.write_all(&bytes)?;
            Ok(())
        })
        .await
    }

    pub async fn open_in_perfetto(&self, capture_id: String) -> Result<(), DesktopError> {
        let relative = self.stage_pftrace_export(capture_id).await?;
        let file = self
            .paths
            .cache_dir()
            .map_err(DesktopError::new)?
            .join(relative);
        self.io(move || {
            actions::reveal(&file)?;
            actions::open("https://ui.perfetto.dev/")
        })
        .await
    }

    pub async fn refresh_view_snapshot(&self, max_nodes: u32) -> Result<(), DesktopError> {
        let (generation, payload) = self
            .rpc(
                "view.snapshot",
                json!({"max_nodes": max_nodes.clamp(1, 20_000)}),
            )
            .await?;
        let tree = self
            .io(move || {
                Ok(views::ViewIndex::new(
                    payload.json(Some(BinaryMsgType::ViewSnapshot))?,
                ))
            })
            .await?;
        self.update(generation, |state| {
            state.data.view = tree;
            state.changed(Channel::Repaints);
        })
    }

    pub fn query_view_tree(
        &self,
        expanded: Vec<String>,
        query: String,
        path: String,
        offset: u32,
        limit: u32,
    ) -> RowSlice {
        let state = self.read();
        state
            .data
            .view
            .rows(&state.data, expanded, &query, &path, offset, limit)
    }

    pub fn view_ancestors(&self, node: String) -> Vec<String> {
        self.read().data.view.ancestors(&node)
    }

    pub fn locate_view(&self, node: String, expanded: Vec<String>) -> Option<u32> {
        let state = self.read();
        state.data.view.position(&node, expanded)
    }

    pub fn get_view_node_detail(&self, node: String) -> Vec<DetailField> {
        let state = self.read();
        let mut value = state.data.view.detail(&node).unwrap_or_default();
        let mut history = Vec::new();
        let mut latest = None;
        for (frame, batch) in state.data.repaints.iter().rev() {
            if let Some(entry) = batch["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| entry["node_raw"] == node)
            {
                if latest.is_none() {
                    latest = Some(entry);
                }
                history.push(frame.to_string());
                if history.len() >= 60 {
                    break;
                }
            }
        }
        if let Some(latest) = latest {
            value["latest_repaint"] = latest.clone();
        }
        value["recent_repaint_frames"] = json!(history);
        fields(&value)
    }

    pub fn node_source(&self, node: String) -> Vec<DetailField> {
        let state = self.read();
        let source = state
            .data
            .view
            .detail(&node)
            .and_then(|value| value.get("source").cloned())
            .filter(|value| !value.is_null())
            .or_else(|| {
                state
                    .data
                    .repaints
                    .values()
                    .rev()
                    .flat_map(|batch| batch["entries"].as_array().into_iter().flatten())
                    .find(|entry| entry["node_raw"] == node)
                    .map(|entry| entry["source"].clone())
            });
        source.map(|source| fields(&source)).unwrap_or_default()
    }

    pub fn node_frames(&self, node: String) -> Vec<String> {
        self.read()
            .data
            .repaints
            .iter()
            .rev()
            .filter(|(_, batch)| {
                batch["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|entry| entry["node_raw"] == node)
            })
            .take(16)
            .map(|(frame, _)| frame.to_string())
            .collect()
    }

    pub async fn load_frame_repaints(&self, frame_id: String) -> Result<(), DesktopError> {
        let frame = parse_id(&frame_id)?;
        if self.read().data.repaints.contains_key(&frame) {
            return Ok(());
        }
        self.refresh(
            "frame.repaints",
            json!({"frame_id": frame}),
            Some(BinaryMsgType::FrameRepaints),
            Channel::Repaints,
            move |state, value| {
                state.data.repaints.insert(frame, value);
                store::trim_map(&mut state.data.repaints, 480);
            },
        )
        .await
    }

    pub fn query_frame_repaints(
        &self,
        frame_id: String,
        path: String,
        sort: String,
        offset: u32,
        limit: u32,
    ) -> Result<RowSlice, DesktopError> {
        Ok(views::repaints(
            &self.read().data,
            parse_id(&frame_id)?,
            &path,
            &sort,
            offset,
            limit,
        ))
    }

    pub fn frame_repaint_summary(&self, frame_id: String) -> Vec<DetailField> {
        let state = self.read();
        let Some(batch) = frame_id
            .parse::<u64>()
            .ok()
            .and_then(|id| state.data.repaints.get(&id))
        else {
            return Vec::new();
        };
        fields(&json!({
            "total_repainted_views": batch["total_repainted_views"], "overflow_count": batch["overflow_count"],
            "fast_patch": batch["fast_patch_views"], "picture_patch": batch["picture_patch_views"],
            "full_rebuild": batch["full_rebuild_views"], "canvas_redraw": batch["canvas_redraw_views"],
        }))
    }

    pub async fn refresh_repaint_leaderboard(&self, limit: u32) -> Result<(), DesktopError> {
        self.refresh(
            "view.repaint_leaderboard",
            json!({"limit": limit.min(256)}),
            None,
            Channel::Repaints,
            |state, value| state.data.leaderboard = value,
        )
        .await
    }

    pub async fn refresh_layout_issues(&self, inspect: bool) -> Result<(), DesktopError> {
        self.refresh(
            "view.layout_issues",
            json!({"inspect": inspect}),
            None,
            Channel::Repaints,
            |state, value| state.data.layouts = value,
        )
        .await
    }

    pub async fn load_layer_tree(&self, frame_id: Option<String>) -> Result<(), DesktopError> {
        let frame_id = frame_id.as_deref().map(parse_id).transpose()?;
        self.refresh(
            "frame.layer_tree",
            json!({"frame_id": frame_id}),
            Some(BinaryMsgType::ViewSnapshot),
            Channel::Repaints,
            |state, value| state.data.layers = value,
        )
        .await
    }

    pub fn query_view_inspection(&self, mode: String, offset: u32, limit: u32) -> RowSlice {
        let state = self.read();
        let value = match mode.as_str() {
            "leaderboard" => &state.data.leaderboard,
            "layout" => &state.data.layouts["issues"],
            "pictures" => &state.data.layers["pictures"],
            _ => &state.data.layers["layers"],
        };
        RowSlice::page(
            value.as_array().into_iter().flatten(),
            offset,
            limit,
            |entry| match mode.as_str() {
                "leaderboard" => Row {
                    id: id(&entry["node_raw"]),
                    name: format!("View {}", id(&entry["node_raw"])),
                    count: number(entry, "total_repaints"),
                    kind: string(entry, "last_path").to_owned(),
                    detail: format!(
                        "{:.0} / 60 frames · {:.0} layouts",
                        number(entry, "recent_repaints"),
                        number(entry, "total_layouts")
                    ),
                    ..Row::default()
                },
                "layout" => Row {
                    id: id(&entry["parent"]),
                    name: format!("Container {}", id(&entry["parent"])),
                    detail: format!(
                        "{:.2} used / {:.2} available · child {}",
                        number(entry, "used"),
                        number(entry, "parentSize"),
                        id(&entry["worstChild"])
                    ),
                    kind: string(entry, "axis").to_owned(),
                    count: number(entry, "excess"),
                    ..Row::default()
                },
                _ => Row {
                    id: id(&entry["id"]),
                    name: format!(
                        "{} {}",
                        entry["kind"].as_str().unwrap_or("Picture"),
                        id(&entry["id"])
                    ),
                    detail: if mode == "pictures" {
                        format!(
                            "{} commands · {} images",
                            entry["commands"], entry["images"]
                        )
                    } else {
                        format!(
                            "Parent {} · picture {}",
                            id(&entry["parent"]),
                            id(&entry["picture"])
                        )
                    },
                    ..Row::default()
                },
            },
        )
    }

    pub fn view_inspection_detail(&self, mode: String, key: String) -> Vec<DetailField> {
        let state = self.read();
        let (value, field) = match mode.as_str() {
            "leaderboard" => (&state.data.leaderboard, "node_raw"),
            "layout" => (&state.data.layouts["issues"], "parent"),
            "pictures" => (&state.data.layers["pictures"], "id"),
            _ => (&state.data.layers["layers"], "id"),
        };
        value
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| id(&entry[field]) == key)
            .map(fields)
            .unwrap_or_default()
    }

    pub async fn set_overlay(&self, config: Overlay) -> Result<(), DesktopError> {
        let config = OverlayConfig {
            highlight_node: config.highlight_node,
            pick_mode: config.pick_mode,
            repaint_rainbow: config.repaint_rainbow,
            layout_scan: config.layout_scan,
        };
        let (generation, payload) = self
            .rpc("view.set_overlay", serde_json::to_value(&config)?)
            .await?;
        payload.json(None)?;
        self.client.track_layout(generation, config.layout_scan)
    }

    pub async fn refresh_js_runtime(&self) -> Result<(), DesktopError> {
        let generation = self.read().generation;
        let memory = self.client.rpc(generation, "js.memory_detail", Value::Null);
        let resources = self.client.rpc(generation, "js.resources", Value::Null);
        let (memory, resources) = futures_util::try_join!(memory, resources)?;
        let memory = memory.json(None)?;
        let resources = resources.json(None)?;
        self.update(generation, |state| {
            state.data.memory = memory;
            state.data.async_resources = resources;
            state.changed(Channel::Runtime);
            state.changed(Channel::Resources);
        })
    }

    pub async fn run_js_gc(&self) -> Result<Vec<DetailField>, DesktopError> {
        let (generation, payload) = self.rpc("js.run_gc", Value::Null).await?;
        let value = payload.json(None)?;
        let summary = gc_fields(&value);
        self.update(generation, |state| {
            state.data.memory = value["after"].clone();
            state.data.gc.push_back(value);
            store::trim(&mut state.data.gc, 16);
            state.changed(Channel::Runtime);
        })?;
        Ok(summary)
    }

    pub fn query_gc_history(&self) -> Vec<Row> {
        self.read()
            .data
            .gc
            .iter()
            .rev()
            .enumerate()
            .map(|(i, gc)| Row {
                id: i.to_string(),
                name: format!("Collected {} bytes", gc["freed_bytes"]),
                detail: format!("{:.2} ms", number(gc, "duration_ms")),
                ..Row::default()
            })
            .collect()
    }

    pub fn gc_detail(&self, index: u32) -> Vec<DetailField> {
        self.read()
            .data
            .gc
            .iter()
            .rev()
            .nth(index as usize)
            .map(gc_fields)
            .unwrap_or_default()
    }

    pub async fn set_js_sampler(
        &self,
        enabled: bool,
        interval_us: u32,
        max_depth: u32,
        max_samples: u32,
    ) -> Result<(), DesktopError> {
        let (generation, payload) = self
            .rpc(
                "js.set_sampler_config",
                json!({"enabled": enabled, "interval_us": interval_us,
            "max_depth": max_depth, "max_samples": max_samples}),
            )
            .await?;
        payload.json(None)?;
        self.update(generation, |state| {
            state.sampler_enabled = enabled;
            state.capture_sampler = false;
        })
    }

    pub fn js_memory_detail(&self) -> Vec<DetailField> {
        fields(&self.read().data.memory)
    }

    pub fn js_resource_detail(&self) -> Vec<DetailField> {
        let state = self.read();
        let value = &state.data.async_resources;
        if value.is_null() {
            return Vec::new();
        }
        fields(
            &json!({"counts": value["counts"], "pending_promises": value["pending_promises"],
            "pending_modules": value["pending_modules"], "job_limit_hits": value["job_limit_hits"],
            "budget_overruns": value["budget_overruns"], "timers_truncated": value["timers_truncated"]}),
        )
    }

    pub fn query_timers(&self, offset: u32, limit: u32) -> RowSlice {
        let state = self.read();
        RowSlice::page(
            state.data.async_resources["active_timers"]
                .as_array()
                .into_iter()
                .flatten(),
            offset,
            limit,
            |timer| Row {
                id: id(&timer["id"]),
                name: format!("Timer {}", id(&timer["id"])),
                kind: string(timer, "kind").to_owned(),
                detail: format!(
                    "{:.1} ms until due · {} interval",
                    number(timer, "due_in_ms"),
                    timer["interval_ms"]
                ),
                ..Row::default()
            },
        )
    }

    pub fn project_js_memory_chart(&self, width: f64, height: f64) -> ProjectedCanvas {
        projection::heap(&self.read().data, width, height)
    }

    pub async fn project_flamegraph(
        &self,
        width: f64,
        height: f64,
        frame_id: Option<String>,
        zoom: u32,
        search: String,
    ) -> Result<ProjectedCanvas, DesktopError> {
        let tree = self
            .build_profile(frame_id.as_deref().map(parse_id).transpose()?)
            .await?;
        Ok(projection::flamegraph(&tree, width, height, zoom, &search, None).0)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the viewport projection export takes canvas coordinates and selected records"
    )]
    pub async fn hit_test_flamegraph(
        &self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        frame_id: Option<String>,
        zoom: u32,
        search: String,
    ) -> Result<Option<Hit>, DesktopError> {
        let tree = self
            .build_profile(frame_id.as_deref().map(parse_id).transpose()?)
            .await?;
        Ok(projection::flamegraph(&tree, width, height, zoom, &search, Some((x, y))).1)
    }

    pub async fn query_profile_table(
        &self,
        mode: String,
        search: String,
        frame_id: Option<String>,
        offset: u32,
        limit: u32,
    ) -> Result<RowSlice, DesktopError> {
        let tree = self
            .build_profile(frame_id.as_deref().map(parse_id).transpose()?)
            .await?;
        Ok(tree.table(&mode, &search, offset, limit))
    }

    pub fn query_logs_slice(
        &self,
        level: String,
        source: String,
        search: String,
        frame: String,
        offset: u32,
        limit: u32,
    ) -> RowSlice {
        log_rows(
            self.read().data.logs.iter(),
            &search,
            &level,
            &source,
            &frame,
            offset,
            limit,
        )
    }

    pub fn query_errors_slice(
        &self,
        search: String,
        source: String,
        frame: String,
        offset: u32,
        limit: u32,
    ) -> RowSlice {
        log_rows(
            self.read().data.errors.iter(),
            &search,
            "",
            &source,
            &frame,
            offset,
            limit,
        )
    }

    pub fn console_detail(&self, key: String, error: bool, raw: bool) -> Vec<DetailField> {
        let state = self.read();
        let source = if error {
            &state.data.errors
        } else {
            &state.data.logs
        };
        let Some(value) = source.iter().find(|entry| id(&entry["seq"]) == key) else {
            return Vec::new();
        };
        let mut result = value.clone();
        if error
            && !raw
            && let Some(stack) = state.data.resolved_errors.get(&key)
        {
            result["stack"] = json!(stack);
        }
        fields(&result)
    }

    pub fn console_stack(&self, key: String, raw: bool) -> Option<String> {
        let state = self.read();
        if !raw && let Some(stack) = state.data.resolved_errors.get(&key) {
            return Some(types::clip(stack, 32_768));
        }
        state
            .data
            .errors
            .iter()
            .find(|entry| id(&entry["seq"]) == key)
            .and_then(|entry| entry["stack"].as_str())
            .map(|stack| types::clip(stack, 32_768))
    }

    pub fn query_network_slice(
        &self,
        query: String,
        kind: String,
        status: String,
        sort: String,
        offset: u32,
        limit: u32,
    ) -> NetworkSlice {
        let state = self.read();
        network::slice(
            state.data.network.values(),
            network::Filter {
                query: &query,
                kind: &kind,
                status: &status,
                sort: &sort,
            },
            offset,
            limit,
            state.now_ms(),
        )
    }

    pub fn get_network_detail(&self, token: String, section: String) -> Vec<DetailField> {
        let state = self.read();
        let Some(record) = token
            .parse::<u64>()
            .ok()
            .and_then(|id| state.data.network.get(&id))
        else {
            return Vec::new();
        };
        match section.as_str() {
            "connection" => fields(&record["connection"]),
            "request_headers" | "response_headers" => record[&section]
                .as_array()
                .into_iter()
                .flatten()
                .take(256)
                .map(|header| DetailField {
                    name: header[0].as_str().unwrap_or_default().to_owned(),
                    value: types::clip(header[1].as_str().unwrap_or_default(), 1024),
                    depth: 0,
                })
                .collect(),
            "ws" => fields(
                &json!({"sent": record["ws_sent"], "received": record["ws_received"],
                "bytes_sent": record["ws_bytes_sent"], "bytes_received": record["ws_bytes_received"]}),
            ),
            _ => fields(
                &json!({"url": record["url"], "final_url": record["final_url"], "method": record["method"],
                "status": record["status"], "state": record["state"], "initiator": record["initiator"],
                "kind": record["kind"], "error": record["error"]}),
            ),
        }
    }

    pub fn query_redirects(&self, token: String, offset: u32, limit: u32) -> RowSlice {
        let state = self.read();
        let record = token
            .parse::<u64>()
            .ok()
            .and_then(|id| state.data.network.get(&id));
        let hops = record.and_then(|record| record["redirects"].as_array());
        RowSlice::page(
            hops.into_iter().flatten().enumerate(),
            offset,
            limit,
            |(ordinal, hop)| Row {
                id: ordinal.to_string(),
                name: string(hop, "url").to_owned(),
                kind: id(&hop["status"]),
                detail: string(hop, "location").to_owned(),
                ..Row::default()
            },
        )
    }

    pub fn project_network_waterfall(
        &self,
        token: String,
        hop: Option<u32>,
        width: f64,
        height: f64,
    ) -> ProjectedCanvas {
        let state = self.read();
        let Some(record) = token
            .parse::<u64>()
            .ok()
            .and_then(|id| state.data.network.get(&id))
        else {
            return ProjectedCanvas::default();
        };
        let record = hop
            .and_then(|hop| record["redirects"].get(hop as usize))
            .unwrap_or(record);
        projection::waterfall(&record["timing"], width, height)
    }

    pub async fn fetch_network_body(
        &self,
        token: String,
        side: String,
    ) -> Result<BodySummary, DesktopError> {
        let token = parse_id(&token)?;
        let side = parse_side(&side)?;
        let (generation, live, cached, metadata) = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let cached = state.bodies.touch((token, side));
            let empty = Value::Null;
            let record = state.data.network.get(&token).unwrap_or(&empty);
            let (preview, truncated, evicted) = match side {
                Side::Request => (
                    network::preview(&record["request_body_preview"]),
                    record["request_body_truncated"] == true,
                    record["request_body_evicted"] == true,
                ),
                Side::Response => (
                    network::preview(&record["response_preview"]),
                    record["response_body_truncated"] == true,
                    record["response_body_evicted"] == true,
                ),
                Side::Socket(seq) => (
                    state
                        .data
                        .ws
                        .get(&token)
                        .into_iter()
                        .flatten()
                        .find(|entry| entry["seq"].as_u64() == Some(seq))
                        .map(|frame| network::preview(&frame["preview"]))
                        .unwrap_or_default(),
                    false,
                    false,
                ),
            };
            (
                state.generation,
                state.state == ConnectionState::Connected,
                cached,
                (
                    network::content_type(record, side),
                    preview,
                    truncated,
                    evicted,
                ),
            )
        };
        let body = if let Some(body) = cached.filter(|body| !live || !body.preview_only) {
            body
        } else {
            let (mime, preview, truncated, evicted) = metadata;
            let params = match side {
                Side::Request => json!({"token": token, "side": "request"}),
                Side::Response => json!({"token": token, "side": "response"}),
                Side::Socket(seq) => json!({"token": token, "frame_seq": seq}),
            };
            let response = if live {
                match self.client.rpc(generation, "net.get_body", params).await {
                    Ok(payload) => Some(payload.binary(BinaryMsgType::NetBody)?),
                    Err(error) if preview.is_empty() && !evicted => return Err(error),
                    Err(_) => None,
                }
            } else {
                None
            };
            let limited = response.is_none();
            let bytes = response.unwrap_or(preview);
            if bytes.is_empty() && limited {
                return Ok(BodySummary {
                    mode: "text".to_owned(),
                    bytes: 0.,
                    truncated,
                    evicted,
                    available: false,
                    image_source: None,
                    image_width: None,
                    image_height: None,
                });
            }
            let body = self
                .io(move || {
                    Ok(Arc::new(Body::new(
                        bytes,
                        mime,
                        truncated || limited,
                        evicted,
                        limited,
                    )))
                })
                .await?;
            self.update(generation, |state| {
                state.bodies.insert((token, side), Arc::clone(&body))
            })?;
            body
        };
        let mut summary = body.summary();
        if body.image_extension().is_some() {
            if let Ok((frame, (width, height))) = self.io(move || body.decode_image()).await {
                if self.read().generation != generation {
                    return Err(DesktopError::disconnected());
                }
                let texture = self.textures.allocate().map_err(DesktopError::new)?;
                texture.publish_rgba(frame).map_err(DesktopError::new)?;
                summary.image_source = Some(texture.name().to_owned());
                summary.image_width = Some(width);
                summary.image_height = Some(height);
                let mut previews = self
                    .previews
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                previews.push_back(texture);
                if previews.len() > 8 {
                    previews.pop_front();
                }
            } else {
                summary.mode = "hex".to_owned();
            }
        }
        Ok(summary)
    }

    pub fn release_body_image(&self, source: String) {
        let mut previews = self
            .previews
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(index) = previews.iter().position(|image| image.name() == source) {
            previews.remove(index);
        }
    }

    pub fn read_body_chunk(
        &self,
        token: String,
        side: String,
        offset: u32,
        limit: u32,
        hex: bool,
    ) -> Result<String, DesktopError> {
        let key = (parse_id(&token)?, parse_side(&side)?);
        Ok(self
            .read()
            .bodies
            .get(key)
            .map(|body| body.chunk(offset, limit, hex))
            .unwrap_or_default())
    }

    pub async fn copy_body_text(
        &self,
        token: String,
        side: String,
    ) -> Result<String, DesktopError> {
        let key = (parse_id(&token)?, parse_side(&side)?);
        let body = self
            .read()
            .bodies
            .get(key)
            .cloned()
            .ok_or_else(|| DesktopError::new("Load a body before copying it"))?;
        self.io(move || Ok(String::from_utf8_lossy(&body.bytes).into_owned()))
            .await
    }

    pub fn query_json_rows(
        &self,
        token: String,
        side: String,
        expanded: Vec<u32>,
        search: String,
        offset: u32,
        limit: u32,
    ) -> Result<RowSlice, DesktopError> {
        let key = (parse_id(&token)?, parse_side(&side)?);
        let body = self.read().bodies.get(key).cloned();
        Ok(body.as_ref().and_then(|body| body.json.as_ref()).map_or(
            RowSlice {
                total: 0,
                rows: Vec::new(),
            },
            |index| index.slice(expanded, &search, offset, limit),
        ))
    }

    pub async fn refresh_ws_frames(&self, token: String) -> Result<(), DesktopError> {
        let token = parse_id(&token)?;
        let since = self
            .read()
            .data
            .ws
            .get(&token)
            .and_then(|frames| frames.back())
            .and_then(|frame| frame["seq"].as_u64());
        let (generation, payload) = self
            .rpc("net.get_ws_frames", json!({"token": token, "since": since}))
            .await?;
        let update: ChannelUpdate<Value> = serde_json::from_value(payload.json(None)?)?;
        self.update(generation, |state| {
            let frames = state.data.ws.entry(token).or_default();
            if update.reset {
                frames.clear();
            }
            let previous = frames
                .back()
                .and_then(|entry| entry["seq"].as_u64())
                .unwrap_or(0);
            frames.extend(
                update
                    .entries
                    .into_iter()
                    .filter(|entry| entry["seq"].as_u64().is_some_and(|seq| seq > previous)),
            );
            store::trim(frames, 2000);
            store::trim_map(&mut state.data.ws, 32);
            state.touch();
        })
    }

    pub fn query_ws_frames(
        &self,
        token: String,
        direction: String,
        search: String,
        offset: u32,
        limit: u32,
    ) -> Result<RowSlice, DesktopError> {
        let token = parse_id(&token)?;
        Ok(network::ws_rows(
            self.read().data.ws.get(&token).into_iter().flatten(),
            &direction,
            &search,
            offset,
            limit,
        ))
    }

    pub fn format_curl(&self, token: String) -> Result<String, DesktopError> {
        let state = self.read();
        let record = state
            .data
            .network
            .get(&parse_id(&token)?)
            .ok_or_else(|| DesktopError::new("Request is outside the retained window"))?;
        Ok(actions::format_curl(record))
    }

    pub fn overview_window_end(&self, track: u32) -> f64 {
        projection::overview_end(&self.read().data, track)
    }

    pub fn project_overview_track(
        &self,
        track: u32,
        width: f64,
        height: f64,
        seconds: u32,
        end_ms: f64,
    ) -> Option<ProjectedCanvas> {
        projection::overview_track(&self.read().data, track, width, height, seconds, end_ms)
    }

    pub fn get_live_audits(&self) -> Vec<Finding> {
        self.read().findings.clone()
    }

    pub fn audit_evidence(&self, finding: String) -> Vec<Row> {
        analyzer::audits::evidence(&self.read().data, &finding)
    }

    pub fn project_resource_trends(&self, width: f64, height: f64) -> Vec<ProjectedCanvas> {
        projection::resources(&self.read().data, width, height)
    }

    pub fn resource_detail(&self) -> Vec<DetailField> {
        let state = self.read();
        state
            .data
            .resource_samples
            .back()
            .map(json_fields)
            .unwrap_or_default()
    }

    pub fn project_bundle_treemap(
        &self,
        width: f64,
        height: f64,
        path: Vec<String>,
        metric: String,
    ) -> ProjectedCanvas {
        self.read()
            .data
            .bundle
            .as_ref()
            .map(|bundle| projection::bundle(bundle, width, height, &path, &metric, None).0)
            .unwrap_or_default()
    }

    pub fn hit_test_bundle_treemap(
        &self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        path: Vec<String>,
        metric: String,
    ) -> Option<Hit> {
        projection::bundle(
            self.read().data.bundle.as_ref()?,
            width,
            height,
            &path,
            &metric,
            Some((x, y)),
        )
        .1
    }

    pub fn bundle_summary(&self) -> Vec<DetailField> {
        let state = self.read();
        let Some(bundle) = state.data.bundle.as_ref() else {
            return Vec::new();
        };
        fields(
            &json!({"artifact_bytes": bundle.artifact_bytes, "startup_bytes": bundle.startup_bytes,
            "chunks": bundle.chunks.len(), "static_pool_dedup_saved_bytes": bundle.pools.values()
                .fold(0u64, |sum, pool| sum.saturating_add(pool.dedup_saved_bytes))}),
        )
    }

    pub fn query_bundle_rows(&self, assets: bool, offset: u32, limit: u32) -> RowSlice {
        let state = self.read();
        let Some(bundle) = state.data.bundle.as_ref() else {
            return RowSlice {
                total: 0,
                rows: Vec::new(),
            };
        };
        if assets {
            return RowSlice::page(&bundle.external_assets, offset, limit, |(path, bytes)| {
                Row {
                    id: path.clone(),
                    name: path.clone(),
                    detail: format!("{bytes} bytes"),
                    ..Row::default()
                }
            });
        }
        let mut modules: Vec<_> = bundle
            .chunks
            .values()
            .flat_map(|chunk| &chunk.modules)
            .collect();
        modules.sort_by_key(|module| {
            std::cmp::Reverse(
                module
                    .apportioned_bytecode_bytes
                    .unwrap_or(module.rendered_bytes),
            )
        });
        RowSlice::page(modules, offset, limit, |module| Row {
            id: module.path.clone(),
            name: module.path.clone(),
            kind: module.group.clone(),
            detail: format!(
                "{} bytes JS · {} bytes bytecode",
                module.rendered_bytes,
                module
                    .apportioned_bytecode_bytes
                    .map_or("—".to_owned(), |bytes| bytes.to_string())
            ),
            ..Row::default()
        })
    }

    pub async fn import_artifact(
        &self,
        relative_path: String,
    ) -> Result<ArtifactSummary, DesktopError> {
        let paths = self.paths.clone();
        let generation = self.read().generation;
        let (summary, artifact) = self
            .io(move || impasto_file_system::consume_import(&paths, &relative_path, archive::read))
            .await?;
        match artifact {
            Artifact::Session(data, trace) => self.client.offline(*data, trace).await?,
            Artifact::Bundle(bundle) => self.update(generation, |state| {
                state.data.bundle = Some(bundle);
                state.changed(Channel::Resources);
            })?,
            Artifact::Symbols(symbols) => {
                self.update(generation, |state| state.install_symbols(Arc::new(symbols)))?
            }
            Artifact::Trace(trace) => self.update(generation, |state| {
                state.imported_trace = Some(trace.into());
                state.touch();
            })?,
        }
        Ok(summary)
    }

    pub async fn stage_session_export(
        &self,
        format: Option<String>,
    ) -> Result<String, DesktopError> {
        let format = format.unwrap_or_else(|| "zip".to_owned());
        if !matches!(format.as_str(), "zip" | "gzip") {
            return Err(DesktopError::new("Invalid archive format"));
        }
        let state = Arc::clone(&self.state);
        let (mut data, mut trace, symbols, generation, live) = self
            .io(move || {
                let state = state.read().unwrap_or_else(|error| error.into_inner());
                let trace = state
                    .trace
                    .as_ref()
                    .filter(|(id, _)| state.data.capture["id"].as_u64() == Some(*id));
                Ok((
                    state.data.clone(),
                    trace.map(|(_, trace)| Arc::clone(trace)),
                    state.symbols.clone(),
                    state.generation,
                    state.state == ConnectionState::Connected,
                ))
            })
            .await?;
        if let Some(capture) = data.capture["id"].as_u64().filter(|_| live) {
            if format == "zip"
                && trace.is_none()
                && let Ok(payload) = self
                    .client
                    .rpc(
                        generation,
                        "trace.export_pftrace",
                        json!({"capture_id": capture}),
                    )
                    .await
                && let Ok(bytes) = payload.binary(BinaryMsgType::TraceExport)
            {
                trace = Some(bytes.into());
            }
            use futures_util::StreamExt as _;
            let pending: Vec<_> = data.capture["frame_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .rev()
                .filter(|id| !data.spans.contains_key(id))
                .take(96usize.saturating_sub(data.spans.len()))
                .collect();
            let results = futures_util::stream::iter(pending)
                .map(|frame_id| async move {
                    let payload = self
                        .client
                        .rpc(
                            generation,
                            "trace.get_frame_spans",
                            json!({"capture_id": capture, "frame_id": frame_id}),
                        )
                        .await
                        .ok()?;
                    Some((frame_id, payload.json(None).ok()?))
                })
                .buffer_unordered(8);
            futures_util::pin_mut!(results);
            while let Some(result) = results.next().await {
                if let Some((id, spans)) = result {
                    data.spans.insert(id, spans);
                }
            }
        }
        if !data.capture.is_null() {
            data.capture["exported_frame_spans"] = json!(data.spans.len());
            data.capture["archive_has_pftrace"] = json!(format == "zip" && trace.is_some());
        }
        let suffix = if format == "zip" {
            ".impasto-devtools.zip"
        } else {
            ".impasto-session"
        };
        self.stage(suffix, move |file| {
            if let Some(symbols) = symbols {
                let resolver = symbols.resolver();
                for sample in &mut data.stacks {
                    if let Some(raw) = sample["value"]["stack"].as_str() {
                        sample["value"]["stack"] = json!(resolver.remap(raw));
                    }
                }
            }
            archive::write_session(file, &data, trace.as_deref(), &format)
        })
        .await
    }

    pub async fn remove_staged(&self, relative: String) -> Result<(), DesktopError> {
        let base = self
            .paths
            .cache_dir()
            .map_err(DesktopError::new)?
            .to_owned();
        let staging = Arc::clone(&self.staging);
        self.io(move || {
            let path = base.join(relative);
            if path.parent() != Some(staging.path()) {
                return Err(DesktopError::new("Invalid staged artifact path"));
            }
            std::fs::remove_file(path)?;
            Ok(())
        })
        .await
    }

    pub async fn open_source_in_editor(
        &self,
        path: String,
        line: u32,
        column: u32,
        editor: String,
    ) -> Result<(), DesktopError> {
        let roots: Vec<_> = {
            let state = self.read();
            [&state.project_root, &state.framework_root]
                .into_iter()
                .filter_map(Clone::clone)
                .collect()
        };
        self.io(move || {
            let uri = actions::source_url(Path::new(&path), &roots, line, column, &editor)?;
            actions::open(uri.as_str())
        })
        .await
    }
}

fn gc_fields(value: &Value) -> Vec<DetailField> {
    let mut result =
        fields(&json!({"freed_bytes": value["freed_bytes"], "duration_ms": value["duration_ms"]}));
    if let Some(before) = value["before"].as_object() {
        result.extend(before.iter().filter_map(|(name, value_before)| {
            let before = value_before.as_f64()?;
            let after = value["after"][name].as_f64()?;
            Some(DetailField {
                name: name.replace('_', " "),
                value: format!("{before:.0} → {after:.0} ({:+.0})", after - before),
                depth: 0,
            })
        }));
    }
    result
}

pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(Desktop::plugin());
}
