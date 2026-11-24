//! Screen canvas session. One paint turn re-records every invalidated canvas
//! after the final layout and installs the result with `SetCanvas`.
//!
//! Path2D and gradients charge the session budget directly. The active recorder
//! borrows that budget for the draw and refunds on drop. JavaScript values are
//! not dropped while the session `RefCell` is borrowed.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use rquickjs::class::Trace;
use rquickjs::function::{Opt, Rest};
use rquickjs::{
    Array, Class, Ctx, Function, IntoJs, JsLifetime, Object, Persistent, TypedArray, Value,
};

use crate::canvas::{
    Budget, CanvasError, CanvasErrorCode, CanvasSize, Deadline, FillRule, Gradient, LineCap,
    LineJoin, PaintEnv, Path2D, Recorder, TextAlign, TextBaseline, TextDirection,
};
use crate::reactive::Id;
use crate::scene::{CanvasComposite, CanvasOp};
use crate::tree::NodeId;

use super::super::frame::elapsed_ms;
use super::super::mutation;
use super::super::reactive::{Cleanup, JsEval, Value as GraphValue};
use super::{Host, Inner, exception_text, host_from_ctx, type_error};

const PATH_NEW: usize = 256;
const PATH_STEP: usize = 64;
const GRADIENT_NEW: usize = 128;
const STOP_STEP: usize = 32;
const MAX_PENDING: usize = 32;

fn binding_index(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::js::player) struct CanvasStats {
    pub(in crate::js::player) record_ms: f64,
    pub(in crate::js::player) prepare_ms: f64,
    pub(in crate::js::player) ops: u32,
    pub(in crate::js::player) segments: u32,
    pub(in crate::js::player) redraws: u32,
    pub(in crate::js::player) callbacks: u32,
    pub(in crate::js::player) preparations: u32,
    pub(in crate::js::player) reused: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Snap {
    width: f32,
    height: f32,
    ratio: f32,
    scale: f32,
    reduce: bool,
    rtl: bool,
    hidden: bool,
    present: bool,
    offscreen: bool,
}

struct Binding {
    generation: u32,
    node: u64,
    paint: u64,
    revision: u64,
    alive: bool,
    pending: bool,
    geometry_epoch: Option<u64>,
    defer_when_offscreen: bool,
    snap: Snap,
    serial: f64,
    draw: Option<Persistent<Value<'static>>>,
    on_error: Option<Persistent<Value<'static>>>,
}

#[derive(Clone)]
struct Labels {
    fill: String,
    stroke: String,
    font: String,
    fill_gradient: Option<Persistent<Value<'static>>>,
    stroke_gradient: Option<Persistent<Value<'static>>>,
}

impl Labels {
    fn fresh() -> Self {
        Self {
            fill: "#000000".to_owned(),
            stroke: "#000000".to_owned(),
            font: "10px Sans".to_owned(),
            fill_gradient: None,
            stroke_gradient: None,
        }
    }
}

struct ActiveDraw {
    index: u32,
    generation: u32,
    epoch: u64,
    node: u64,
    deadline: Deadline,
    recorder: Recorder<'static>,
    labels: Labels,
    stack: Vec<Labels>,
}

struct PendingError {
    index: u32,
    generation: u32,
    code: String,
    api: Option<String>,
    message: String,
}

struct Bump {
    index: u32,
    generation: u32,
    revision: u64,
    serial: f64,
}

pub(in crate::js::player) struct CanvasSession {
    bindings: Vec<Binding>,
    free: Vec<u32>,
    active: Option<ActiveDraw>,
    epoch: u64,
    stop: bool,
    retried: bool,
    closed: bool,
    blocked: bool,
    pending_errors: Vec<PendingError>,
    deadline: Deadline,
    stats: CanvasStats,
    parked: Vec<Labels>,
}

impl CanvasSession {
    pub(in crate::js::player) fn new() -> Self {
        Self {
            bindings: Vec::new(),
            free: Vec::new(),
            active: None,
            epoch: 0,
            stop: false,
            retried: false,
            closed: false,
            blocked: false,
            pending_errors: Vec::new(),
            deadline: Deadline::after(Duration::from_millis(128)),
            stats: CanvasStats::default(),
            parked: Vec::new(),
        }
    }
}

struct PathState {
    path: Path2D,
    charged: usize,
}

struct GradientState {
    gradient: Gradient,
    charged: usize,
}

enum PaintLabel {
    Color(String),
    Gradient(Persistent<Value<'static>>),
}

fn session_budget(inner: &Inner) -> &'static Budget {
    let pointer = {
        let budget = inner.canvas_budget.borrow();
        std::ptr::from_ref(&*budget)
    };
    // SAFETY: `canvas_budget` is a field of `Inner`. The session keeps Inner in an
    // Rc allocation and never moves it. The RefCell is never replaced. Recorders live in
    // `canvas`, which is declared before `canvas_budget` and therefore drops
    // first. Callers use only `&Budget`. Nothing calls `borrow_mut` on
    // `canvas_budget` while a recorder exists.
    unsafe { &*pointer }
}

fn code_name(code: CanvasErrorCode) -> &'static str {
    match code {
        CanvasErrorCode::InvalidArgument => "InvalidArgument",
        CanvasErrorCode::LimitExceeded => "LimitExceeded",
        CanvasErrorCode::ContextClosed => "ContextClosed",
        CanvasErrorCode::DrawSideEffect => "DrawSideEffect",
        CanvasErrorCode::AsyncDraw => "AsyncDraw",
        CanvasErrorCode::TextUnavailable => "TextUnavailable",
        CanvasErrorCode::Unsupported => "Unsupported",
        CanvasErrorCode::Interrupted => "Interrupted",
    }
}

fn closed_error() -> CanvasError {
    CanvasError::new(
        CanvasErrorCode::ContextClosed,
        "canvas",
        "canvas context is closed",
    )
}

fn side_effect_error() -> CanvasError {
    CanvasError::new(
        CanvasErrorCode::DrawSideEffect,
        "canvas",
        "draw cannot change the scene or reactive graph",
    )
}

fn raise(ctx: &Ctx<'_>, error: &CanvasError) -> rquickjs::Error {
    if error.code == CanvasErrorCode::Interrupted {
        mark_interrupted(ctx);
    }
    let object = match Object::new(ctx.clone()) {
        Ok(object) => object,
        Err(error) => return error,
    };
    if object.set("name", "CanvasError").is_err()
        || object.set("code", code_name(error.code)).is_err()
        || object.set("message", error.message.as_str()).is_err()
    {
        return type_error(ctx, &error.message);
    }
    if let Some(api) = error.api {
        let _ = object.set("api", api);
    }
    ctx.throw(object.into_value())
}

fn mark_interrupted(ctx: &Ctx<'_>) {
    let Ok(host) = host_from_ctx(ctx) else {
        return;
    };
    let mut session = host.inner().canvas.borrow_mut();
    let slot = session
        .active
        .as_ref()
        .map(|active| (active.index, active.generation));
    session.stop = true;
    if let Some((index, generation)) = slot
        && let Some(binding) = session.bindings.get_mut(binding_index(index))
        && binding.generation == generation
    {
        binding.pending = true;
    }
}

fn js_string(value: &Value<'_>) -> Option<String> {
    value.as_string().and_then(|text| text.to_string().ok())
}

fn numbers<const N: usize>(args: &[Value<'_>], api: &'static str) -> Result<[f64; N], CanvasError> {
    if args.len() < N {
        return Err(CanvasError::argument(api, "missing numeric argument"));
    }
    let mut out = [0.0; N];
    for (output, value) in out.iter_mut().zip(args) {
        *output = value
            .as_number()
            .ok_or_else(|| CanvasError::argument(api, "expected a number"))?;
    }
    Ok(out)
}

fn optional_bool(value: Option<&Value<'_>>, api: &'static str) -> Result<bool, CanvasError> {
    match value {
        None => Ok(false),
        Some(value) if value.is_undefined() || value.is_null() => Ok(false),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| CanvasError::argument(api, "expected a boolean")),
    }
}

fn rule_at(args: &[Value<'_>], index: usize, api: &'static str) -> Result<FillRule, CanvasError> {
    let Some(value) = args.get(index) else {
        return Ok(FillRule::NonZero);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(FillRule::NonZero);
    }
    match js_string(value).as_deref() {
        Some("nonzero") => Ok(FillRule::NonZero),
        Some("evenodd") => Ok(FillRule::EvenOdd),
        _ => Err(CanvasError::argument(api, "invalid fill rule")),
    }
}

fn is_path(value: &Value<'_>) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.instance_of::<JsPath>())
}

fn is_gradient(value: &Value<'_>) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.instance_of::<JsGradient>())
}

fn cloned_path(value: &Value<'_>) -> Option<Path2D> {
    let class = value.as_object()?.into_class::<JsPath>().ok()?;
    let borrowed = class.try_borrow().ok()?;
    Some(borrowed.state.borrow().path.clone())
}

fn cloned_gradient(value: &Value<'_>) -> Option<Gradient> {
    let class = value.as_object()?.into_class::<JsGradient>().ok()?;
    let borrowed = class.try_borrow().ok()?;
    Some(borrowed.state.borrow().gradient.clone())
}

fn is_thenable(value: &Value<'_>) -> bool {
    value
        .as_object()
        .and_then(|object| object.get::<_, Value<'_>>("then").ok())
        .is_some_and(|then| then.is_function())
}

fn is_stop(code: &str, message: &str) -> bool {
    code == "Interrupted"
        || message.contains("interrupted")
        || message.contains("canvas draw exceeded")
        || message.to_ascii_lowercase().contains("out of memory")
}

fn pixel_ratio(ratio: f32) -> f64 {
    let ratio = f64::from(ratio);
    if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0e-4
    }
}

fn is_drawable(snap: Snap) -> bool {
    snap.present && !snap.hidden && !snap.offscreen && snap.width > 0.0 && snap.height > 0.0
}

fn binding_at(session: &CanvasSession, index: u32, generation: u32) -> Option<&Binding> {
    session
        .bindings
        .get(binding_index(index))
        .filter(|binding| binding.generation == generation)
}

fn binding_at_mut(
    session: &mut CanvasSession,
    index: u32,
    generation: u32,
) -> Option<&mut Binding> {
    session
        .bindings
        .get_mut(binding_index(index))
        .filter(|binding| binding.generation == generation)
}

fn read_snap(inner: &Inner, tree: &crate::tree::Tree, binding: &Binding) -> Snap {
    let mut snap = binding.snap;
    if binding.geometry_epoch != Some(tree.canvas_geometry_epoch()) {
        let node = NodeId::from_raw(binding.node);
        snap = match tree.canvas_surface(node) {
            Some(surface) => Snap {
                width: surface.width,
                height: surface.height,
                rtl: surface.rtl,
                hidden: surface.hidden,
                present: true,
                offscreen: binding.defer_when_offscreen && tree.canvas_is_offscreen(node),
                ..Snap::default()
            },
            None => Snap {
                hidden: true,
                ..Snap::default()
            },
        };
    }
    snap.ratio = tree.pixel_scale();
    snap.scale = inner.text_scale.get();
    snap.reduce = inner.reduce_motion.get();
    snap
}

fn dispose_id(inner: &Inner, raw: u64) {
    if raw == 0 {
        return;
    }
    let id = Id::from_raw(raw);
    if inner.graph.is_live(id) {
        inner.graph.dispose(JsEval, id);
    }
}

fn push_error(session: &mut CanvasSession, error: PendingError) {
    if session.pending_errors.len() < MAX_PENDING {
        session.pending_errors.push(error);
    }
}

fn queue_parts(
    host: &Host,
    index: u32,
    generation: u32,
    code: &str,
    api: Option<&str>,
    message: &str,
    stop: bool,
) {
    let paused = host.paused.get() || host.inner().paused.get();
    {
        let mut session = host.inner().canvas.borrow_mut();
        if stop {
            session.stop = true;
            if let Some(binding) = binding_at_mut(&mut session, index, generation) {
                binding.pending = true;
            }
        }
        push_error(
            &mut session,
            PendingError {
                index,
                generation,
                code: code.to_owned(),
                api: api.map(str::to_owned),
                message: message.to_owned(),
            },
        );
    }
    if stop || paused {
        return;
    }
    host.wants_frame.set(true);
}

fn queue_canvas(host: &Host, index: u32, generation: u32, error: &CanvasError) {
    let stop = is_stop(code_name(error.code), &error.message);
    queue_parts(
        host,
        index,
        generation,
        code_name(error.code),
        error.api,
        &error.message,
        stop,
    );
}

fn add_ms(slot: &mut f64, value: f64) {
    *slot += value;
}

fn note_record(host: &Host, millis: f64) {
    add_ms(
        &mut host.inner().canvas.borrow_mut().stats.record_ms,
        millis,
    );
}

fn note_prepare(host: &Host, millis: f64) {
    add_ms(
        &mut host.inner().canvas.borrow_mut().stats.prepare_ms,
        millis,
    );
}

fn note_redraw(host: &Host, ops: u32, segments: u32) {
    let stats = &mut host.inner().canvas.borrow_mut().stats;
    stats.redraws = stats.redraws.saturating_add(1);
    stats.ops = stats.ops.saturating_add(ops);
    stats.segments = stats.segments.saturating_add(segments);
}

fn tally(list: &crate::scene::CanvasList) -> (u32, u32) {
    let ops = u32::try_from(list.ops().len()).unwrap_or(u32::MAX);
    let mut segments = 0_usize;
    for op in list.ops() {
        match op {
            CanvasOp::Clip { path } | CanvasOp::Fill { path, .. } => {
                segments = segments.saturating_add(path.verbs.len());
            }
            _ => {}
        }
    }
    (ops, u32::try_from(segments).unwrap_or(u32::MAX))
}

fn clear_picture(host: &Host, node: u64) {
    if node == 0 {
        return;
    }
    let _ = mutation::mutate(
        host,
        mutation::Mutation::SetCanvas {
            id: NodeId::from_raw(node),
            list: None,
        },
    );
}

fn release_binding(host: &Host, index: u32, generation: u32) {
    let inner = host.inner();
    let taken = {
        let mut session = inner.canvas.borrow_mut();
        let Some(binding) = binding_at_mut(&mut session, index, generation) else {
            return;
        };
        if !binding.alive
            && binding.paint == 0
            && binding.revision == 0
            && binding.draw.is_none()
            && binding.on_error.is_none()
        {
            return;
        }
        binding.alive = false;
        binding.pending = false;
        let node = binding.node;
        let paint = binding.paint;
        let revision = binding.revision;
        binding.paint = 0;
        binding.revision = 0;
        binding.node = 0;
        let draw = binding.draw.take();
        let on_error = binding.on_error.take();
        if !session.free.contains(&index) {
            session.free.push(index);
        }
        (node, paint, revision, draw, on_error)
    };
    drop(taken.3);
    drop(taken.4);
    dispose_id(inner, taken.1);
    dispose_id(inner, taken.2);
    clear_picture(host, taken.0);
}

fn abandon_slot(inner: &Inner, index: u32) {
    let taken = {
        let mut session = inner.canvas.borrow_mut();
        let Some(binding) = session.bindings.get_mut(binding_index(index)) else {
            return;
        };
        binding.alive = false;
        let paint = binding.paint;
        let revision = binding.revision;
        binding.paint = 0;
        binding.revision = 0;
        let draw = binding.draw.take();
        let on_error = binding.on_error.take();
        if !session.free.contains(&index) {
            session.free.push(index);
        }
        (paint, revision, draw, on_error)
    };
    drop(taken.2);
    drop(taken.3);
    dispose_id(inner, taken.0);
    dispose_id(inner, taken.1);
}

struct SlotGuard<'a> {
    inner: &'a Inner,
    index: u32,
    keep: bool,
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        if !self.keep {
            abandon_slot(self.inner, self.index);
        }
    }
}

fn allocate_slot(
    inner: &Inner,
    node: u64,
    defer_when_offscreen: bool,
) -> Result<SlotGuard<'_>, CanvasError> {
    let (index, leftover) = {
        let mut session = inner.canvas.borrow_mut();
        if let Some(index) = session.free.pop() {
            let Some(binding) = session.bindings.get_mut(binding_index(index)) else {
                return Err(CanvasError::limit("bindCanvas"));
            };
            let draw = binding.draw.take();
            let on_error = binding.on_error.take();
            binding.generation = binding.generation.wrapping_add(1).max(1);
            binding.node = node;
            binding.paint = 0;
            binding.revision = 0;
            binding.alive = false;
            binding.pending = false;
            binding.geometry_epoch = None;
            binding.defer_when_offscreen = defer_when_offscreen;
            binding.snap = Snap::default();
            binding.serial = 0.0;
            (index, (draw, on_error))
        } else {
            let index = u32::try_from(session.bindings.len())
                .map_err(|_| CanvasError::limit("bindCanvas"))?;
            session.bindings.push(Binding {
                generation: 1,
                node,
                paint: 0,
                revision: 0,
                alive: false,
                pending: false,
                geometry_epoch: None,
                defer_when_offscreen,
                snap: Snap::default(),
                serial: 0.0,
                draw: None,
                on_error: None,
            });
            (index, (None, None))
        }
    };
    drop(leftover);
    Ok(SlotGuard {
        inner,
        index,
        keep: false,
    })
}

fn plan_sync(host: &Host) -> Vec<Bump> {
    let mut session = host.inner().canvas.borrow_mut();
    let tree = host.inner().tree.borrow();
    let epoch = tree.canvas_geometry_epoch();
    let mut planned = Vec::new();
    for (index, binding) in session.bindings.iter_mut().enumerate() {
        if !binding.alive || binding.revision == 0 {
            continue;
        }
        let first = binding.geometry_epoch.is_none();
        let before = binding.snap;
        let snap = read_snap(host.inner(), &tree, binding);
        binding.geometry_epoch = Some(epoch);
        binding.snap = snap;
        let needs_paint = if is_drawable(snap) {
            first || binding.pending || before != snap
        } else {
            // Run the effect once on exit so it drops the author's
            // dependencies and keeps only the internal revision.
            first || is_drawable(before)
        };
        if !needs_paint {
            continue;
        }
        let Ok(index) = u32::try_from(index) else {
            continue;
        };
        planned.push(Bump {
            index,
            generation: binding.generation,
            revision: binding.revision,
            serial: binding.serial + 1.0,
        });
    }
    planned
}

fn sync_revisions(host: &Host, ctx: &Ctx<'_>) {
    let planned = plan_sync(host);
    for bump in planned {
        let Ok(value) = bump.serial.into_js(ctx) else {
            continue;
        };
        let saved = GraphValue::save(ctx, host, value);
        let _ = host
            .inner()
            .graph
            .set(JsEval, Id::from_raw(bump.revision), saved);
        let mut session = host.inner().canvas.borrow_mut();
        if let Some(binding) = binding_at_mut(&mut session, bump.index, bump.generation)
            && binding.alive
        {
            binding.serial = bump.serial;
            binding.pending = false;
        }
    }
}

fn finish_turn(host: &Host) -> CanvasStats {
    let (stats, request) = {
        let mut session = host.inner().canvas.borrow_mut();
        let request = if session.stop && !session.retried {
            session.retried = true;
            true
        } else {
            if !session.stop {
                session.retried = false;
            }
            false
        };
        session.stop = false;
        let alive = session
            .bindings
            .iter()
            .filter(|binding| binding.alive)
            .count();
        let alive = u32::try_from(alive).unwrap_or(u32::MAX);
        session.stats.reused = alive.saturating_sub(session.stats.redraws);
        (session.stats, request)
    };
    if request && !host.paused.get() && !host.inner().paused.get() {
        host.wants_frame.set(true);
    }
    let _ = host.inner().canvas_budget.borrow().used();
    stats
}

fn take_active(host: &Host) -> Option<ActiveDraw> {
    host.inner().canvas.borrow_mut().active.take()
}

fn still_alive(host: &Host, index: u32, generation: u32) -> bool {
    host.inner()
        .canvas
        .borrow()
        .bindings
        .get(binding_index(index))
        .is_some_and(|binding| binding.alive && binding.generation == generation)
}

fn commit(host: &Host, index: u32, generation: u32) {
    if !still_alive(host, index, generation) {
        drop(take_active(host));
        return;
    }
    let Some(active) = take_active(host) else {
        return;
    };
    let node = active.node;
    let deadline = active.deadline;
    let started = std::time::Instant::now();
    let recording = active.recorder.finish();
    note_record(host, elapsed_ms(started));
    let started = std::time::Instant::now();
    host.inner().canvas.borrow_mut().stats.preparations += 1;
    let prepare_trace = tracing::debug_span!(target: "impasto::canvas", "canvas_prepare").entered();
    let prepared = host
        .inner()
        .tree
        .borrow_mut()
        .prepare_canvas(recording, &deadline);
    drop(prepare_trace);
    note_prepare(host, elapsed_ms(started));
    match prepared {
        Ok(list) => {
            let (ops, segments) = tally(&list);
            let mutation = mutation::Mutation::SetCanvas {
                id: NodeId::from_raw(node),
                list: Some(list),
            };
            match mutation::mutate(host, mutation) {
                Ok(_) => {
                    note_redraw(host, ops, segments);
                    let height = {
                        let tree = host.inner().tree.borrow();
                        tree.canvas_surface(NodeId::from_raw(node))
                            .map_or(0.0, |surface| surface.height)
                    };
                    tracing::debug!(
                        target: "impasto::canvas",
                        node,
                        height,
                        ops,
                        "canvas_commit"
                    );
                }
                Err(error) => queue_parts(
                    host,
                    index,
                    generation,
                    "Unsupported",
                    Some("canvas"),
                    &error.to_string(),
                    false,
                ),
            }
        }
        Err(error) => queue_canvas(host, index, generation, &error),
    }
}

fn classify_exception(ctx: &Ctx<'_>, error: rquickjs::Error) -> (String, Option<String>, String) {
    if !error.is_exception() {
        let message = error.to_string();
        let code = if is_stop("", &message) {
            "Interrupted"
        } else {
            "Unsupported"
        };
        return (code.to_owned(), None, message);
    }
    let value = ctx.catch();
    let code = value
        .as_object()
        .and_then(|object| object.get::<_, String>("code").ok());
    let api = value
        .as_object()
        .and_then(|object| object.get::<_, String>("api").ok());
    let (message, _) = exception_text(ctx, value);
    let code = match code {
        Some(code) if !code.is_empty() => code,
        _ if is_stop("", &message) => "Interrupted".to_owned(),
        _ => "Unsupported".to_owned(),
    };
    (code, api, message)
}

fn paint_binding(host: &Host, ctx: &Ctx<'_>, index: u32, generation: u32) {
    let revision = {
        let session = host.inner().canvas.borrow();
        let Some(binding) = binding_at(&session, index, generation) else {
            return;
        };
        if !binding.alive {
            return;
        }
        binding.revision
    };
    if revision != 0 {
        let _ = host.inner().graph.get(JsEval, Id::from_raw(revision));
    }
    if host.inner().canvas.borrow().stop {
        let mut session = host.inner().canvas.borrow_mut();
        if let Some(binding) = binding_at_mut(&mut session, index, generation) {
            binding.pending = true;
        }
        return;
    }
    let ready = {
        let session = host.inner().canvas.borrow();
        let Some(binding) = binding_at(&session, index, generation) else {
            return;
        };
        if !binding.alive {
            return;
        }
        is_drawable(binding.snap).then(|| (binding.node, binding.snap, binding.draw.clone()))
    };
    let Some((node, snap, draw)) = ready else {
        return;
    };
    let Some(draw) = draw else {
        return;
    };
    let epoch = {
        let mut session = host.inner().canvas.borrow_mut();
        session.epoch = session.epoch.wrapping_add(1).max(1);
        session.blocked = false;
        let epoch = session.epoch;
        let ratio = pixel_ratio(snap.ratio);
        let recorder = Recorder::new(
            CanvasSize {
                width: f64::from(snap.width),
                height: f64::from(snap.height),
            },
            PaintEnv {
                pixel_ratio: ratio,
                direction_rtl: snap.rtl,
            },
            session_budget(host.inner()),
        );
        session.active = Some(ActiveDraw {
            index,
            generation,
            epoch,
            node,
            deadline: session.deadline,
            recorder,
            labels: Labels::fresh(),
            stack: Vec::new(),
        });
        epoch
    };
    let _ = host.inner().graph.take_paint_blocked();
    host.retarget("canvas", node);
    let started = std::time::Instant::now();
    host.inner().canvas.borrow_mut().stats.callbacks += 1;
    let draw_trace = tracing::debug_span!(target: "impasto::canvas", "canvas_draw").entered();
    let called = (|| -> rquickjs::Result<Value<'_>> {
        let function = Function::from_value(draw.restore(ctx)?)?;
        let context = Class::instance(
            ctx.clone(),
            DrawContext {
                index,
                generation,
                epoch,
            },
        )?;
        let size = Object::new(ctx.clone())?;
        size.set("width", f64::from(snap.width))?;
        size.set("height", f64::from(snap.height))?;
        size.set("pixelRatio", pixel_ratio(snap.ratio))?;
        size.set("textScale", f64::from(snap.scale))?;
        size.set("reduceMotion", snap.reduce)?;
        function.call((context, size))
    })();
    drop(draw_trace);
    note_record(host, elapsed_ms(started));
    let blocked = host.inner().canvas.borrow().blocked || host.inner().graph.take_paint_blocked();
    let stop = host.inner().canvas.borrow().stop;
    match called {
        Ok(value) if is_thenable(&value) => {
            drop(take_active(host));
            queue_parts(
                host,
                index,
                generation,
                "AsyncDraw",
                Some("draw"),
                "draw must be synchronous",
                false,
            );
        }
        Ok(_) if blocked || stop => {
            drop(take_active(host));
            if stop {
                queue_parts(
                    host,
                    index,
                    generation,
                    "Interrupted",
                    None,
                    "canvas draw exceeded its time budget",
                    true,
                );
            }
        }
        Ok(_) => commit(host, index, generation),
        Err(error) => {
            drop(take_active(host));
            let (code, api, message) = classify_exception(ctx, error);
            let stop = is_stop(&code, &message);
            queue_parts(
                host,
                index,
                generation,
                &code,
                api.as_deref(),
                &message,
                stop,
            );
        }
    }
}

impl CanvasSession {
    pub(in crate::js::player) fn observes_visibility(&self) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.alive && binding.defer_when_offscreen)
    }
}

/// Run invalidated screen canvases. A paused or closed session does not paint.
pub(in crate::js::player) fn paint(host: &Host) -> CanvasStats {
    if host.paused.get() || host.inner().paused.get() || host.inner().canvas.borrow().closed {
        return CanvasStats::default();
    }
    let _turn = host.budget("canvas", 0);
    {
        let mut session = host.inner().canvas.borrow_mut();
        session.deadline = Deadline::after(Duration::from_millis(128));
        session.stats = CanvasStats::default();
        session.blocked = false;
    }
    let _ = host.inner().graph.take_paint_blocked();
    host.with(|ctx| {
        sync_revisions(host, &ctx);
        host.inner().graph.run_paints(JsEval);
    });
    finish_turn(host)
}

/// Drop the active recorder before owner cleanup so its charge is refunded.
pub(in crate::js::player) fn begin_shutdown(inner: &Inner) {
    let active = {
        let mut session = inner.canvas.borrow_mut();
        session.closed = true;
        session.active.take()
    };
    if let Some(active) = active {
        let ActiveDraw {
            recorder,
            labels,
            stack,
            ..
        } = active;
        drop(recorder);
        let mut session = inner.canvas.borrow_mut();
        session.parked.push(labels);
        session.parked.extend(stack);
    }
}

/// Drop saved callbacks and clear pictures that no owner cleanup released.
pub(in crate::js::player) fn finish_shutdown(host: &Host) {
    let inner = host.inner();
    let (parked, nodes, saved) = {
        let mut session = inner.canvas.borrow_mut();
        let parked = std::mem::take(&mut session.parked);
        session.pending_errors.clear();
        let mut nodes = Vec::new();
        let mut saved = Vec::new();
        for binding in &mut session.bindings {
            if binding.alive {
                nodes.push(binding.node);
            }
            binding.alive = false;
            binding.paint = 0;
            binding.revision = 0;
            binding.node = 0;
            if let Some(draw) = binding.draw.take() {
                saved.push(draw);
            }
            if let Some(on_error) = binding.on_error.take() {
                saved.push(on_error);
            }
        }
        session.free.clear();
        (parked, nodes, saved)
    };
    drop(parked);
    drop(saved);
    for node in nodes {
        clear_picture(host, node);
    }
}

pub(super) fn forget_nodes(host: &Host, ids: &[u64]) {
    let inner = host.inner();
    let matched: Vec<(u32, u32)> = {
        let session = inner.canvas.borrow();
        session
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(index, binding)| {
                if binding.alive && ids.contains(&binding.node) {
                    u32::try_from(index)
                        .ok()
                        .map(|index| (index, binding.generation))
                } else {
                    None
                }
            })
            .collect()
    };
    for (index, generation) in matched {
        release_binding(host, index, generation);
    }
}

pub(in crate::js::player) fn deliver_errors(host: &Host) -> Vec<String> {
    let pending = {
        let mut session = host.inner().canvas.borrow_mut();
        std::mem::take(&mut session.pending_errors)
    };
    if pending.is_empty() {
        return Vec::new();
    }
    let _budget = host.budget("canvas-error", 0);
    host.with(|ctx| {
        let mut notes = Vec::new();
        let mut deferred = Vec::new();
        let mut rest = pending.into_iter();
        while let Some(error) = rest.next() {
            let looked = {
                let session = host.inner().canvas.borrow();
                binding_at(&session, error.index, error.generation)
                    .filter(|binding| binding.alive)
                    .map(|binding| (binding.node, binding.on_error.clone()))
            };
            let Some((node, callback)) = looked else {
                continue;
            };
            if host.past_deadline() {
                deferred.push(error);
                deferred.extend(rest);
                host.wants_frame.set(true);
                break;
            }
            host.retarget("canvas-error", node);
            let delivered =
                callback.map_or(Ok(false), |saved| invoke_error(host, &ctx, saved, &error));
            let message = match delivered {
                Ok(true) => continue,
                Ok(false) => format!("{}: {}", error.code, error.message),
                Err(message) => message,
            };
            notes.push(record_canvas_fault(host, node, &message));
        }
        if !deferred.is_empty() {
            let mut session = host.inner().canvas.borrow_mut();
            let fresh = std::mem::take(&mut session.pending_errors);
            let available = MAX_PENDING.saturating_sub(deferred.len());
            deferred.extend(fresh.into_iter().take(available));
            session.pending_errors = deferred;
        }
        notes
    })
}

fn record_canvas_fault(host: &Host, node: u64, message: &str) -> String {
    let message = format!("canvas node {node}: {message}");
    host.inner()
        .faults
        .borrow_mut()
        .push(crate::session::ScriptFault {
            source: "canvas",
            message: message.clone(),
            stack: String::new(),
        });
    message
}

fn invoke_error(
    host: &Host,
    ctx: &Ctx<'_>,
    callback: Persistent<Value<'static>>,
    error: &PendingError,
) -> Result<bool, String> {
    host.with_current(ctx, || {
        let function = match callback.restore(ctx).and_then(Function::from_value) {
            Ok(function) => function,
            Err(failure) => return Err(failure.to_string()),
        };
        let object = match Object::new(ctx.clone()) {
            Ok(object) => object,
            Err(failure) => return Err(failure.to_string()),
        };
        if object.set("name", "CanvasError").is_err()
            || object.set("code", error.code.as_str()).is_err()
            || object.set("message", error.message.as_str()).is_err()
        {
            return Err(error.message.clone());
        }
        if let Some(api) = &error.api {
            let _ = object.set("api", api.as_str());
        }
        let result = function.call::<_, Value<'_>>((object,));
        if host.past_deadline() {
            if ctx.has_exception() {
                drop(ctx.catch());
            }
            return Err("Interrupted: onDrawError exceeded its time budget".into());
        }
        match result {
            Ok(value) => {
                if ctx.has_exception() {
                    let (message, _) = exception_text(ctx, ctx.catch());
                    Err(message)
                } else {
                    Ok(value.as_bool() != Some(false))
                }
            }
            Err(failure) => {
                if failure.is_exception() {
                    let (message, _) = exception_text(ctx, ctx.catch());
                    Err(message)
                } else {
                    Err(failure.to_string())
                }
            }
        }
    })
}

pub(super) fn side_effect(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let host = host_from_ctx(ctx)?;
    check_side_effect(&host, ctx)
}

pub(super) fn check_side_effect(host: &Host, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    if !host.inner().graph.is_painting() {
        return Ok(());
    }
    host.inner().canvas.borrow_mut().blocked = true;
    Err(raise(ctx, &side_effect_error()))
}

pub(super) fn install_exports<'js>(ctx: &Ctx<'js>, core: &Object<'js>) -> rquickjs::Result<()> {
    let _ = Class::<DrawContext>::prototype(ctx)?;
    let _ = Class::<JsGradient>::prototype(ctx)?;
    let _ = Class::<Controller>::prototype(ctx)?;
    let _ = Class::<JsPath>::prototype(ctx)?;
    if let Some(constructor) = Class::<JsPath>::create_constructor(ctx)? {
        core.set("Path2D", constructor)?;
    }
    let function = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let host = host_from_ctx(&ctx)?;
        host.with_current(&ctx, || bind_canvas(&host, &ctx, &args.0))
    })?;
    function.set_name("bindCanvas")?;
    core.set("bindCanvas", function)
}

fn bind_canvas<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    if host.inner().canvas.borrow().closed {
        return Err(raise(
            ctx,
            &CanvasError::new(
                CanvasErrorCode::ContextClosed,
                "bindCanvas",
                "canvas is closed",
            ),
        ));
    }
    if host.inner().graph.is_painting() {
        host.inner().canvas.borrow_mut().blocked = true;
        return Err(raise(ctx, &side_effect_error()));
    }
    let node = node_of(args.first()).map_err(|error| raise(ctx, &error))?;
    let draw = args
        .get(1)
        .cloned()
        .filter(|value| value.is_function())
        .ok_or_else(|| {
            raise(
                ctx,
                &CanvasError::argument("bindCanvas", "draw must be a function"),
            )
        })?;
    let on_error = match args.get(2) {
        None => None,
        Some(value) if value.is_undefined() || value.is_null() => None,
        Some(value) if value.is_function() => Some(value.clone()),
        Some(_) => {
            return Err(raise(
                ctx,
                &CanvasError::argument("bindCanvas", "onDrawError must be a function"),
            ));
        }
    };
    let defer_when_offscreen =
        optional_bool(args.get(3), "bindCanvas").map_err(|error| raise(ctx, &error))?;
    let mut guard = allocate_slot(host.inner(), node, defer_when_offscreen)
        .map_err(|error| raise(ctx, &error))?;
    let index = guard.index;
    let generation = {
        let session = host.inner().canvas.borrow();
        session.bindings[binding_index(index)].generation
    };
    let paint_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, _: Rest<Value<'_>>| -> rquickjs::Result<()> {
            let host = host_from_ctx(&ctx)?;
            host.with_current(&ctx, || paint_binding(&host, &ctx, index, generation));
            Ok(())
        },
    )?;
    let cleanup_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, _: Rest<Value<'_>>| -> rquickjs::Result<()> {
            let host = host_from_ctx(&ctx)?;
            host.with_current(&ctx, || release_binding(&host, index, generation));
            Ok(())
        },
    )?;
    let draw_saved = Persistent::save(ctx, draw);
    let error_saved = on_error.map(|value| Persistent::save(ctx, value));
    let zero = 0.0_f64.into_js(ctx)?;
    let revision = host
        .inner()
        .graph
        .create_signal(GraphValue::save(ctx, host, zero));
    let paint =
        host.inner()
            .graph
            .create_paint_effect(GraphValue::save(ctx, host, paint_fn.into_value()));
    host.inner()
        .graph
        .on_cleanup(Cleanup::Script(GraphValue::save(
            ctx,
            host,
            cleanup_fn.into_value(),
        )));
    {
        let mut session = host.inner().canvas.borrow_mut();
        let binding = &mut session.bindings[binding_index(index)];
        binding.revision = revision.to_raw();
        binding.paint = paint.to_raw();
        binding.draw = Some(draw_saved);
        binding.on_error = error_saved;
    }
    let controller = Class::instance(
        ctx.clone(),
        Controller {
            index,
            generation,
            released: Cell::new(false),
        },
    )?;
    {
        let mut session = host.inner().canvas.borrow_mut();
        if let Some(binding) = session.bindings.get_mut(binding_index(index)) {
            binding.alive = true;
        }
    }
    guard.keep = true;
    Ok(controller.into_value())
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "node ids are finite integers inside the JavaScript safe integer range"
)]
fn node_of(value: Option<&Value<'_>>) -> Result<u64, CanvasError> {
    let Some(number) = value.and_then(Value::as_number) else {
        return Err(CanvasError::argument("bindCanvas", "expected a node id"));
    };
    if number.is_finite()
        && number >= 0.0
        && number.fract() == 0.0
        && number < 9_007_199_254_740_992.0
    {
        Ok(number as u64)
    } else {
        Err(CanvasError::argument("bindCanvas", "expected a node id"))
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "CanvasRenderingContext2D")]
struct DrawContext {
    index: u32,
    generation: u32,
    epoch: u64,
}

impl DrawContext {
    fn edit<T>(
        &self,
        ctx: &Ctx<'_>,
        body: impl FnOnce(&mut ActiveDraw) -> Result<T, CanvasError>,
    ) -> rquickjs::Result<T> {
        let host = host_from_ctx(ctx)?;
        let outcome = {
            let mut session = host.inner().canvas.borrow_mut();
            match session.active.as_mut() {
                Some(active)
                    if active.index == self.index
                        && active.generation == self.generation
                        && active.epoch == self.epoch =>
                {
                    body(active)
                }
                _ => Err(closed_error()),
            }
        };
        outcome.map_err(|error| raise(ctx, &error))
    }

    fn batch(
        &self,
        ctx: &Ctx<'_>,
        array: &TypedArray<'_, f64>,
        api: &'static str,
        edit: impl FnOnce(&mut Recorder<'_>, &[f64]) -> Result<(), CanvasError>,
    ) -> rquickjs::Result<()> {
        self.edit(ctx, |active| {
            let raw = array
                .as_raw()
                .ok_or_else(|| CanvasError::argument(api, "detached coordinate buffer"))?;
            #[allow(unsafe_code, reason = "synchronous read from a native Float64Array")]
            // SAFETY: QuickJS provides the typed and aligned memory for `array`.
            // The draw recorder is pure Rust and does not call back into QuickJS.
            let values = unsafe {
                std::slice::from_raw_parts(raw.cast::<f64>().as_ptr(), raw.len() / size_of::<f64>())
            };
            edit(&mut active.recorder, values)
        })
    }

    fn view<T>(&self, ctx: &Ctx<'_>, body: impl FnOnce(&ActiveDraw) -> T) -> rquickjs::Result<T> {
        self.edit(ctx, |active| Ok(body(active)))
    }

    fn paint_label(&self, ctx: &Ctx<'_>, stroke: bool) -> rquickjs::Result<PaintLabel> {
        self.view(ctx, |active| {
            let (text, gradient) = if stroke {
                (&active.labels.stroke, &active.labels.stroke_gradient)
            } else {
                (&active.labels.fill, &active.labels.fill_gradient)
            };
            if let Some(gradient) = gradient {
                PaintLabel::Gradient(gradient.clone())
            } else {
                PaintLabel::Color(text.clone())
            }
        })
    }

    fn assign_style<'js>(
        &self,
        ctx: &Ctx<'js>,
        value: Value<'js>,
        stroke: bool,
    ) -> rquickjs::Result<()> {
        if let Some(text) = js_string(&value) {
            return self.edit(ctx, |active| {
                let before = active.recorder.author();
                if stroke {
                    active.recorder.set_stroke_style(&text);
                } else {
                    active.recorder.set_fill_style(&text);
                }
                let after = active.recorder.author();
                let changed = if stroke {
                    before.stroke_color != after.stroke_color
                        || before.stroke_gradient != after.stroke_gradient
                } else {
                    before.fill_color != after.fill_color
                        || before.fill_gradient != after.fill_gradient
                };
                if changed {
                    if stroke {
                        active.labels.stroke = text.clone();
                        active.labels.stroke_gradient = None;
                    } else {
                        active.labels.fill = text.clone();
                        active.labels.fill_gradient = None;
                    }
                }
                Ok(())
            });
        }
        if is_gradient(&value) {
            let Some(gradient) = cloned_gradient(&value) else {
                return Ok(());
            };
            let saved = Persistent::save(ctx, value);
            return self.edit(ctx, move |active| {
                if stroke {
                    active.recorder.set_stroke_gradient(&gradient);
                    active.labels.stroke_gradient = Some(saved);
                } else {
                    active.recorder.set_fill_gradient(&gradient);
                    active.labels.fill_gradient = Some(saved);
                }
                Ok(())
            });
        }
        Ok(())
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl DrawContext {
    #[qjs(get, rename = "canvas")]
    fn get_canvas<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let size = self.view(&ctx, |active| active.recorder.size())?;
        let object = Object::new(ctx)?;
        object.set("width", size.width)?;
        object.set("height", size.height)?;
        Ok(object)
    }

    #[qjs(get, rename = "fillStyle")]
    fn get_fill_style<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self.paint_label(&ctx, false)? {
            PaintLabel::Color(color) => color.into_js(&ctx),
            PaintLabel::Gradient(gradient) => gradient.restore(&ctx),
        }
    }

    #[qjs(set, rename = "fillStyle")]
    fn set_fill_style<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        self.assign_style(&ctx, value, false)
    }

    #[qjs(get, rename = "strokeStyle")]
    fn get_stroke_style<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self.paint_label(&ctx, true)? {
            PaintLabel::Color(color) => color.into_js(&ctx),
            PaintLabel::Gradient(gradient) => gradient.restore(&ctx),
        }
    }

    #[qjs(set, rename = "strokeStyle")]
    fn set_stroke_style<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        self.assign_style(&ctx, value, true)
    }

    #[qjs(get, rename = "lineWidth")]
    fn get_line_width(&self, ctx: Ctx<'_>) -> rquickjs::Result<f64> {
        self.view(&ctx, |active| active.recorder.line_width())
    }

    #[qjs(set, rename = "lineWidth")]
    fn set_line_width<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(width) = value.as_number() else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_line_width(width);
            Ok(())
        })
    }

    #[qjs(get, rename = "lineCap")]
    fn get_line_cap(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        self.view(&ctx, |active| {
            cap_name(active.recorder.author().cap).to_owned()
        })
    }

    #[qjs(set, rename = "lineCap")]
    fn set_line_cap<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(cap) = js_string(&value).and_then(|text| parse_cap(&text)) else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_line_cap(cap);
            Ok(())
        })
    }

    #[qjs(get, rename = "lineJoin")]
    fn get_line_join(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        self.view(&ctx, |active| {
            join_name(active.recorder.author().join).to_owned()
        })
    }

    #[qjs(set, rename = "lineJoin")]
    fn set_line_join<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(join) = js_string(&value).and_then(|text| parse_join(&text)) else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_line_join(join);
            Ok(())
        })
    }

    #[qjs(get, rename = "miterLimit")]
    fn get_miter_limit(&self, ctx: Ctx<'_>) -> rquickjs::Result<f64> {
        self.view(&ctx, |active| active.recorder.author().miter_limit)
    }

    #[qjs(set, rename = "miterLimit")]
    fn set_miter_limit<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(limit) = value.as_number() else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_miter_limit(limit);
            Ok(())
        })
    }

    #[qjs(get, rename = "globalAlpha")]
    fn get_global_alpha(&self, ctx: Ctx<'_>) -> rquickjs::Result<f64> {
        self.view(&ctx, |active| active.recorder.global_alpha())
    }

    #[qjs(set, rename = "globalAlpha")]
    fn set_global_alpha<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(alpha) = value.as_number() else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_global_alpha(alpha);
            Ok(())
        })
    }

    #[qjs(get, rename = "globalCompositeOperation")]
    fn get_composite(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        self.view(&ctx, |active| {
            composite_name(active.recorder.author().composite).to_owned()
        })
    }

    #[qjs(set, rename = "globalCompositeOperation")]
    fn set_composite<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(composite) = js_string(&value).and_then(|text| parse_composite(&text)) else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_composite(composite);
            Ok(())
        })
    }

    #[qjs(get, rename = "lineDashOffset")]
    fn get_line_dash_offset(&self, ctx: Ctx<'_>) -> rquickjs::Result<f64> {
        self.view(&ctx, |active| active.recorder.author().dash_offset)
    }

    #[qjs(set, rename = "lineDashOffset")]
    fn set_line_dash_offset<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(offset) = value.as_number() else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_line_dash_offset(offset);
            Ok(())
        })
    }

    #[qjs(get, rename = "font")]
    fn get_font(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        self.view(&ctx, |active| active.labels.font.clone())
    }

    #[qjs(set, rename = "font")]
    fn set_font<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(text) = js_string(&value) else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            let before = active.recorder.font().clone();
            active.recorder.set_font(&text);
            if active.recorder.font() != &before {
                active.labels.font = text.clone();
            }
            Ok(())
        })
    }

    #[qjs(get, rename = "textAlign")]
    fn get_text_align(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        self.view(&ctx, |active| {
            align_name(active.recorder.author().align).to_owned()
        })
    }

    #[qjs(set, rename = "textAlign")]
    fn set_text_align<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(align) = js_string(&value).and_then(|text| parse_align(&text)) else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_text_align(align);
            Ok(())
        })
    }

    #[qjs(get, rename = "textBaseline")]
    fn get_text_baseline(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        self.view(&ctx, |active| {
            baseline_name(active.recorder.author().baseline).to_owned()
        })
    }

    #[qjs(set, rename = "textBaseline")]
    fn set_text_baseline<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(baseline) = js_string(&value).and_then(|text| parse_baseline(&text)) else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_text_baseline(baseline);
            Ok(())
        })
    }

    #[qjs(get, rename = "direction")]
    fn get_direction(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        self.view(&ctx, |active| {
            direction_name(active.recorder.author().direction).to_owned()
        })
    }

    #[qjs(set, rename = "direction")]
    fn set_direction<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(direction) = js_string(&value).and_then(|text| parse_direction(&text)) else {
            return Ok(());
        };
        self.edit(&ctx, |active| {
            active.recorder.set_direction(direction);
            Ok(())
        })
    }

    #[qjs(get, rename = "imageSmoothingEnabled")]
    fn get_smoothing(&self, ctx: Ctx<'_>) -> rquickjs::Result<bool> {
        self.view(&ctx, |active| active.recorder.author().smoothing)
    }

    #[qjs(set, rename = "imageSmoothingEnabled")]
    fn set_smoothing<'js>(&self, ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
        let Some(enabled) = value.as_bool() else {
            return Err(raise(
                &ctx,
                &CanvasError::argument("imageSmoothingEnabled", "expected a boolean"),
            ));
        };
        self.edit(&ctx, |active| {
            active.recorder.set_image_smoothing(enabled);
            Ok(())
        })
    }

    fn save(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.edit(&ctx, |active| {
            active.recorder.save()?;
            active.stack.push(active.labels.clone());
            Ok(())
        })
    }

    fn restore(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.edit(&ctx, |active| {
            if let Some(labels) = active.stack.pop() {
                active.labels = labels;
            }
            active.recorder.restore();
            Ok(())
        })
    }

    fn begin_path(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.edit(&ctx, |active| {
            active.recorder.begin_path();
            Ok(())
        })
    }

    fn close_path(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.edit(&ctx, |active| active.recorder.close_path())
    }

    fn move_to<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<2>(&args.0, "moveTo").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| active.recorder.move_to(nums[0], nums[1]))
    }

    fn line_to<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<2>(&args.0, "lineTo").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| active.recorder.line_to(nums[0], nums[1]))
    }

    fn polyline<'js>(&self, ctx: Ctx<'js>, points: TypedArray<'js, f64>) -> rquickjs::Result<()> {
        self.batch(&ctx, &points, "polyline", |recorder, values| {
            recorder.polyline(values)
        })
    }

    fn line_segments<'js>(
        &self,
        ctx: Ctx<'js>,
        points: TypedArray<'js, f64>,
    ) -> rquickjs::Result<()> {
        self.batch(&ctx, &points, "lineSegments", |recorder, values| {
            recorder.line_segments(values)
        })
    }

    fn fill_rects<'js>(&self, ctx: Ctx<'js>, rects: TypedArray<'js, f64>) -> rquickjs::Result<()> {
        self.batch(&ctx, &rects, "fillRects", |recorder, values| {
            recorder.fill_rects(values)
        })
    }

    fn quadratic_curve_to<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<()> {
        let nums =
            numbers::<4>(&args.0, "quadraticCurveTo").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active.recorder.quad_to(nums[0], nums[1], nums[2], nums[3])
        })
    }

    fn bezier_curve_to<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<6>(&args.0, "bezierCurveTo").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .cubic_to(nums[0], nums[1], nums[2], nums[3], nums[4], nums[5])
        })
    }

    fn arc_to<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<5>(&args.0, "arcTo").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .arc_to(nums[0], nums[1], nums[2], nums[3], nums[4])
        })
    }

    fn rect<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<4>(&args.0, "rect").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active.recorder.rect(nums[0], nums[1], nums[2], nums[3])
        })
    }

    fn round_rect<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<4>(&args.0, "roundRect").map_err(|error| raise(&ctx, &error))?;
        let radii = radii_of(&ctx, args.0.get(4))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .round_rect(nums[0], nums[1], nums[2], nums[3], &radii)
        })
    }

    fn arc<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<5>(&args.0, "arc").map_err(|error| raise(&ctx, &error))?;
        let ccw = optional_bool(args.0.get(5), "arc").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .arc(nums[0], nums[1], nums[2], nums[3], nums[4], ccw)
        })
    }

    fn ellipse<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<7>(&args.0, "ellipse").map_err(|error| raise(&ctx, &error))?;
        let ccw = optional_bool(args.0.get(7), "ellipse").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active.recorder.ellipse(
                nums[0], nums[1], nums[2], nums[3], nums[4], nums[5], nums[6], ccw,
            )
        })
    }

    fn translate<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<2>(&args.0, "translate").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active.recorder.translate(nums[0], nums[1]);
            Ok(())
        })
    }

    fn rotate<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<1>(&args.0, "rotate").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active.recorder.rotate(nums[0]);
            Ok(())
        })
    }

    fn scale<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<2>(&args.0, "scale").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active.recorder.scale(nums[0], nums[1]);
            Ok(())
        })
    }

    fn transform<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<6>(&args.0, "transform").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .transform(nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]);
            Ok(())
        })
    }

    fn set_transform<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<6>(&args.0, "setTransform").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .set_transform(nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]);
            Ok(())
        })
    }

    fn reset_transform(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.edit(&ctx, |active| {
            active.recorder.reset_transform();
            Ok(())
        })
    }

    fn get_transform<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let coeffs = self.view(&ctx, |active| active.recorder.get_transform())?;
        let object = Object::new(ctx)?;
        object.set("a", coeffs[0])?;
        object.set("b", coeffs[1])?;
        object.set("c", coeffs[2])?;
        object.set("d", coeffs[3])?;
        object.set("e", coeffs[4])?;
        object.set("f", coeffs[5])?;
        Ok(object)
    }

    fn set_line_dash<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let Some(array) = args.0.first().and_then(Value::as_array) else {
            return Err(raise(
                &ctx,
                &CanvasError::argument("setLineDash", "expected an array"),
            ));
        };
        let Some(len) = small_array_len(array)?.filter(|len| {
            *len <= crate::canvas::MAX_DASH && (len % 2 == 0 || *len <= crate::canvas::MAX_DASH / 2)
        }) else {
            return Err(raise(&ctx, &CanvasError::limit("setLineDash")));
        };
        self.view(&ctx, |_| ())?;
        let mut segments = [0.0; crate::canvas::MAX_DASH];
        let segments = &mut segments[..len];
        for (index, segment) in segments.iter_mut().enumerate() {
            let item: Value<'_> = array.get(index)?;
            let Some(number) = item.as_number() else {
                return Err(raise(
                    &ctx,
                    &CanvasError::argument("setLineDash", "expected a number"),
                ));
            };
            *segment = number;
        }
        self.edit(&ctx, |active| active.recorder.set_line_dash(segments))
    }

    fn get_line_dash<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Array<'js>> {
        let segments = self.view(&ctx, |active| active.recorder.line_dash().to_vec())?;
        let array = Array::new(ctx)?;
        for (index, value) in segments.iter().copied().enumerate() {
            array.set(index, value)?;
        }
        Ok(array)
    }

    fn fill<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.first().is_some_and(is_path) {
            let Some(path) = args.0.first().and_then(cloned_path) else {
                return Err(raise(
                    &ctx,
                    &CanvasError::argument("fill", "expected a path"),
                ));
            };
            let rule = rule_at(&args.0, 1, "fill").map_err(|error| raise(&ctx, &error))?;
            self.edit(&ctx, |active| active.recorder.fill_path(&path, rule))
        } else {
            let rule = rule_at(&args.0, 0, "fill").map_err(|error| raise(&ctx, &error))?;
            self.edit(&ctx, |active| active.recorder.fill(rule))
        }
    }

    fn stroke<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.first().is_some_and(is_path) {
            let Some(path) = args.0.first().and_then(cloned_path) else {
                return Err(raise(
                    &ctx,
                    &CanvasError::argument("stroke", "expected a path"),
                ));
            };
            self.edit(&ctx, |active| active.recorder.stroke_path(&path))
        } else if args.0.first().is_some_and(|value| !value.is_undefined()) {
            Err(raise(
                &ctx,
                &CanvasError::argument("stroke", "expected a path"),
            ))
        } else {
            self.edit(&ctx, |active| active.recorder.stroke())
        }
    }

    fn clip<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        if args.0.first().is_some_and(is_path) {
            let Some(path) = args.0.first().and_then(cloned_path) else {
                return Err(raise(
                    &ctx,
                    &CanvasError::argument("clip", "expected a path"),
                ));
            };
            let rule = rule_at(&args.0, 1, "clip").map_err(|error| raise(&ctx, &error))?;
            self.edit(&ctx, |active| active.recorder.clip_path(&path, rule))
        } else {
            let rule = rule_at(&args.0, 0, "clip").map_err(|error| raise(&ctx, &error))?;
            self.edit(&ctx, |active| active.recorder.clip(rule))
        }
    }

    fn fill_rect<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<4>(&args.0, "fillRect").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .fill_rect(nums[0], nums[1], nums[2], nums[3])
        })
    }

    fn stroke_rect<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<4>(&args.0, "strokeRect").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .stroke_rect(nums[0], nums[1], nums[2], nums[3])
        })
    }

    fn clear_rect<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<4>(&args.0, "clearRect").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .clear_rect(nums[0], nums[1], nums[2], nums[3])
        })
    }

    fn fill_text<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let Some(text) = args.0.first().and_then(js_string) else {
            return Err(raise(
                &ctx,
                &CanvasError::argument("fillText", "expected a string"),
            ));
        };
        let nums = numbers::<2>(&args.0[1..], "fillText").map_err(|error| raise(&ctx, &error))?;
        let max_width = match args.0.get(3) {
            None => None,
            Some(value) if value.is_undefined() => None,
            Some(value) => Some(
                value
                    .as_number()
                    .ok_or_else(|| CanvasError::argument("fillText", "expected a number"))
                    .map_err(|error| raise(&ctx, &error))?,
            ),
        };
        self.edit(&ctx, |active| {
            active
                .recorder
                .fill_text(&text, nums[0], nums[1], max_width)
        })
    }

    fn measure_text<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let Some(text) = args.0.first().and_then(js_string) else {
            return Err(raise(
                &ctx,
                &CanvasError::argument("measureText", "expected a string"),
            ));
        };
        let (font, rtl) = self.view(&ctx, |active| {
            (active.recorder.font().clone(), active.recorder.text_rtl())
        })?;
        let host = host_from_ctx(&ctx)?;
        let metrics = host
            .inner()
            .tree
            .borrow_mut()
            .canvas_text_metrics(&text, &font, rtl)
            .map_err(|error| raise(&ctx, &error))?;
        let object = Object::new(ctx)?;
        object.set("width", metrics.width)?;
        object.set("fontBoundingBoxAscent", metrics.font_bounding_box_ascent)?;
        object.set("fontBoundingBoxDescent", metrics.font_bounding_box_descent)?;
        Ok(object)
    }

    fn draw_image<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let Some(source) = args.0.first().and_then(js_string) else {
            return Err(raise(
                &ctx,
                &CanvasError::argument("drawImage", "expected a source"),
            ));
        };
        let parsed = parse_image(&args.0, &source).map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |active| {
            active
                .recorder
                .draw_image(&source, parsed.src, parsed.x, parsed.y, parsed.w, parsed.h)
        })
    }

    fn create_linear_gradient<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        let nums =
            numbers::<4>(&args.0, "createLinearGradient").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |_| Ok(()))?;
        let gradient = Gradient::linear(nums[0], nums[1], nums[2], nums[3])
            .map_err(|error| raise(&ctx, &error))?;
        spawn_gradient(&ctx, gradient).map(Class::into_value)
    }

    fn create_radial_gradient<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        let nums =
            numbers::<6>(&args.0, "createRadialGradient").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |_| Ok(()))?;
        let gradient = Gradient::radial(nums[0], nums[1], nums[2], nums[3], nums[4], nums[5])
            .map_err(|error| raise(&ctx, &error))?;
        spawn_gradient(&ctx, gradient).map(Class::into_value)
    }

    fn create_conic_gradient<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<Value<'js>> {
        let nums =
            numbers::<3>(&args.0, "createConicGradient").map_err(|error| raise(&ctx, &error))?;
        self.edit(&ctx, |_| Ok(()))?;
        let gradient =
            Gradient::conic(nums[0], nums[1], nums[2]).map_err(|error| raise(&ctx, &error))?;
        spawn_gradient(&ctx, gradient).map(Class::into_value)
    }

    fn is_point_in_path<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<bool> {
        if args.0.first().is_some_and(is_path) {
            let Some(path) = args.0.first().and_then(cloned_path) else {
                return Err(raise(
                    &ctx,
                    &CanvasError::argument("isPointInPath", "expected a path"),
                ));
            };
            let nums =
                numbers::<2>(&args.0[1..], "isPointInPath").map_err(|error| raise(&ctx, &error))?;
            let rule = rule_at(&args.0, 3, "isPointInPath").map_err(|error| raise(&ctx, &error))?;
            self.view(&ctx, |active| {
                active
                    .recorder
                    .is_point_in_path_of(&path, nums[0], nums[1], rule)
            })
        } else {
            let nums =
                numbers::<2>(&args.0, "isPointInPath").map_err(|error| raise(&ctx, &error))?;
            let rule = rule_at(&args.0, 2, "isPointInPath").map_err(|error| raise(&ctx, &error))?;
            self.view(&ctx, |active| {
                active.recorder.is_point_in_path(nums[0], nums[1], rule)
            })
        }
    }

    fn is_point_in_stroke<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<bool> {
        let host = host_from_ctx(&ctx)?;
        let deadline = host.inner().canvas.borrow().deadline;
        if args.0.first().is_some_and(is_path) {
            let Some(path) = args.0.first().and_then(cloned_path) else {
                return Err(raise(
                    &ctx,
                    &CanvasError::argument("isPointInStroke", "expected a path"),
                ));
            };
            let nums = numbers::<2>(&args.0[1..], "isPointInStroke")
                .map_err(|error| raise(&ctx, &error))?;
            self.edit(&ctx, |active| {
                active
                    .recorder
                    .is_point_in_stroke_of(&path, nums[0], nums[1], &deadline)
            })
        } else {
            let nums =
                numbers::<2>(&args.0, "isPointInStroke").map_err(|error| raise(&ctx, &error))?;
            self.edit(&ctx, |active| {
                active
                    .recorder
                    .is_point_in_stroke(nums[0], nums[1], &deadline)
            })
        }
    }
}

struct ImageArgs {
    src: Option<[f64; 4]>,
    x: f64,
    y: f64,
    w: Option<f64>,
    h: Option<f64>,
}

fn parse_image(args: &[Value<'_>], _source: &str) -> Result<ImageArgs, CanvasError> {
    match args.len() {
        3 => {
            let nums = numbers::<2>(&args[1..], "drawImage")?;
            Ok(ImageArgs {
                src: None,
                x: nums[0],
                y: nums[1],
                w: None,
                h: None,
            })
        }
        5 => {
            let nums = numbers::<4>(&args[1..], "drawImage")?;
            Ok(ImageArgs {
                src: None,
                x: nums[0],
                y: nums[1],
                w: Some(nums[2]),
                h: Some(nums[3]),
            })
        }
        9 => {
            let nums = numbers::<8>(&args[1..], "drawImage")?;
            Ok(ImageArgs {
                src: Some([nums[0], nums[1], nums[2], nums[3]]),
                x: nums[4],
                y: nums[5],
                w: Some(nums[6]),
                h: Some(nums[7]),
            })
        }
        _ => Err(CanvasError::argument(
            "drawImage",
            "expected 3, 5, or 9 arguments",
        )),
    }
}

fn small_array_len(array: &Array<'_>) -> rquickjs::Result<Option<usize>> {
    // rquickjs Array::len() asserts that length fits a JS int. Sparse arrays
    // may use a double for larger lengths, which these APIs can reject early.
    let length: Value<'_> = array.as_object().get("length")?;
    Ok(length.as_int().and_then(|len| usize::try_from(len).ok()))
}

fn radii_of(ctx: &Ctx<'_>, value: Option<&Value<'_>>) -> rquickjs::Result<Vec<f64>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_undefined() || value.is_null() {
        return Ok(Vec::new());
    }
    if let Some(radius) = value.as_number() {
        return Ok(vec![radius]);
    }
    let Some(array) = value.as_array() else {
        return Err(raise(
            ctx,
            &CanvasError::argument("roundRect", "radii must be a number or an array"),
        ));
    };
    let Some(len) = small_array_len(array)?.filter(|len| (1..=4).contains(len)) else {
        return Err(raise(
            ctx,
            &CanvasError::argument("roundRect", "radii length must be 1..=4"),
        ));
    };
    let mut radii = Vec::with_capacity(len);
    for index in 0..len {
        let item: Value<'_> = array.get(index)?;
        let Some(radius) = item.as_number() else {
            return Err(raise(
                ctx,
                &CanvasError::argument("roundRect", "expected a number"),
            ));
        };
        radii.push(radius);
    }
    Ok(radii)
}

fn spawn_gradient<'js>(
    ctx: &Ctx<'js>,
    gradient: Gradient,
) -> rquickjs::Result<Class<'js, JsGradient>> {
    let host = host_from_ctx(ctx)?;
    let budget = session_budget(host.inner());
    budget
        .reserve(GRADIENT_NEW)
        .map_err(|error| raise(ctx, &error))?;
    match Class::instance(
        ctx.clone(),
        JsGradient {
            inner: Rc::downgrade(&host.inner),
            state: RefCell::new(GradientState {
                gradient,
                charged: GRADIENT_NEW,
            }),
        },
    ) {
        Ok(class) => Ok(class),
        Err(error) => {
            budget.refund(GRADIENT_NEW);
            Err(error)
        }
    }
}

fn cap_name(cap: LineCap) -> &'static str {
    match cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    }
}

fn parse_cap(text: &str) -> Option<LineCap> {
    match text {
        "butt" => Some(LineCap::Butt),
        "round" => Some(LineCap::Round),
        "square" => Some(LineCap::Square),
        _ => None,
    }
}

fn join_name(join: LineJoin) -> &'static str {
    match join {
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
        LineJoin::Bevel => "bevel",
    }
}

fn parse_join(text: &str) -> Option<LineJoin> {
    match text {
        "miter" => Some(LineJoin::Miter),
        "round" => Some(LineJoin::Round),
        "bevel" => Some(LineJoin::Bevel),
        _ => None,
    }
}

fn composite_name(composite: CanvasComposite) -> &'static str {
    match composite {
        CanvasComposite::SourceOver => "source-over",
        CanvasComposite::DestinationOver => "destination-over",
        CanvasComposite::DestinationOut => "destination-out",
        CanvasComposite::Multiply => "multiply",
        CanvasComposite::Screen => "screen",
        CanvasComposite::Lighter => "lighter",
        CanvasComposite::Xor => "xor",
    }
}

fn parse_composite(text: &str) -> Option<CanvasComposite> {
    match text {
        "source-over" => Some(CanvasComposite::SourceOver),
        "destination-over" => Some(CanvasComposite::DestinationOver),
        "destination-out" => Some(CanvasComposite::DestinationOut),
        "multiply" => Some(CanvasComposite::Multiply),
        "screen" => Some(CanvasComposite::Screen),
        "lighter" => Some(CanvasComposite::Lighter),
        "xor" => Some(CanvasComposite::Xor),
        _ => None,
    }
}

fn align_name(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "left",
        TextAlign::Right => "right",
        TextAlign::Center => "center",
        TextAlign::Start => "start",
        TextAlign::End => "end",
    }
}

fn parse_align(text: &str) -> Option<TextAlign> {
    match text {
        "left" => Some(TextAlign::Left),
        "right" => Some(TextAlign::Right),
        "center" => Some(TextAlign::Center),
        "start" => Some(TextAlign::Start),
        "end" => Some(TextAlign::End),
        _ => None,
    }
}

fn baseline_name(baseline: TextBaseline) -> &'static str {
    match baseline {
        TextBaseline::Alphabetic => "alphabetic",
        TextBaseline::Ideographic => "ideographic",
        TextBaseline::Top => "top",
        TextBaseline::Middle => "middle",
        TextBaseline::Bottom => "bottom",
    }
}

fn parse_baseline(text: &str) -> Option<TextBaseline> {
    match text {
        "alphabetic" => Some(TextBaseline::Alphabetic),
        "ideographic" => Some(TextBaseline::Ideographic),
        "top" => Some(TextBaseline::Top),
        "middle" => Some(TextBaseline::Middle),
        "bottom" => Some(TextBaseline::Bottom),
        _ => None,
    }
}

fn direction_name(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::Inherit => "inherit",
        TextDirection::Ltr => "ltr",
        TextDirection::Rtl => "rtl",
    }
}

fn parse_direction(text: &str) -> Option<TextDirection> {
    match text {
        "inherit" => Some(TextDirection::Inherit),
        "ltr" => Some(TextDirection::Ltr),
        "rtl" => Some(TextDirection::Rtl),
        _ => None,
    }
}

fn matrix_of(value: &Value<'_>) -> Option<[f64; 6]> {
    let object = value.as_object()?;
    let component = |key| {
        object
            .get::<_, Value<'_>>(key)
            .ok()
            .and_then(|item| item.as_number())
    };
    Some([
        component("a")?,
        component("b")?,
        component("c")?,
        component("d")?,
        component("e")?,
        component("f")?,
    ])
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "Path2D")]
struct JsPath {
    #[qjs(skip_trace)]
    inner: Weak<Inner>,
    #[qjs(skip_trace)]
    state: RefCell<PathState>,
}

impl Drop for JsPath {
    fn drop(&mut self) {
        let charged = self.state.get_mut().charged;
        if charged != 0
            && let Some(inner) = self.inner.upgrade()
        {
            session_budget(&inner).refund(charged);
        }
    }
}

impl JsPath {
    fn change(
        &self,
        ctx: &Ctx<'_>,
        body: impl FnOnce(&mut Path2D) -> Result<(), CanvasError>,
    ) -> rquickjs::Result<()> {
        let host = host_from_ctx(ctx)?;
        let budget = session_budget(host.inner());
        budget
            .reserve(PATH_STEP)
            .map_err(|error| raise(ctx, &error))?;
        let result = {
            let mut state = self.state.borrow_mut();
            let result = body(&mut state.path);
            if result.is_ok() {
                state.charged = state.charged.saturating_add(PATH_STEP);
            }
            result
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                budget.refund(PATH_STEP);
                Err(raise(ctx, &error))
            }
        }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl JsPath {
    #[qjs(constructor)]
    fn new(ctx: Ctx<'_>, source: Opt<Value<'_>>) -> rquickjs::Result<Self> {
        let host = host_from_ctx(&ctx)?;
        let budget = session_budget(host.inner());
        budget
            .reserve(PATH_NEW)
            .map_err(|error| raise(&ctx, &error))?;
        let path = match build_path(&source.0) {
            Ok(path) => path,
            Err(error) => {
                budget.refund(PATH_NEW);
                return Err(raise(&ctx, &error));
            }
        };
        Ok(Self {
            inner: Rc::downgrade(&host.inner),
            state: RefCell::new(PathState {
                path,
                charged: PATH_NEW,
            }),
        })
    }

    fn add_path<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let Some(path) = args.0.first().and_then(cloned_path) else {
            return Err(raise(
                &ctx,
                &CanvasError::argument("addPath", "expected a path"),
            ));
        };
        let matrix = match args.0.get(1) {
            None => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            Some(value) if value.is_undefined() || value.is_null() => {
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
            }
            Some(value) => matrix_of(value).ok_or_else(|| {
                raise(&ctx, &CanvasError::argument("addPath", "expected a matrix"))
            })?,
        };
        self.change(&ctx, |target| target.add_matrix(&path, matrix))
    }

    fn close_path(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.change(&ctx, Path2D::close)
    }

    fn move_to<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<2>(&args.0, "moveTo").map_err(|error| raise(&ctx, &error))?;
        self.change(&ctx, |path| path.move_to(nums[0], nums[1]))
    }

    fn line_to<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<2>(&args.0, "lineTo").map_err(|error| raise(&ctx, &error))?;
        self.change(&ctx, |path| path.line_to(nums[0], nums[1]))
    }

    fn quadratic_curve_to<'js>(
        &self,
        ctx: Ctx<'js>,
        args: Rest<Value<'js>>,
    ) -> rquickjs::Result<()> {
        let nums =
            numbers::<4>(&args.0, "quadraticCurveTo").map_err(|error| raise(&ctx, &error))?;
        self.change(&ctx, |path| {
            path.quad_to(nums[0], nums[1], nums[2], nums[3])
        })
    }

    fn bezier_curve_to<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<6>(&args.0, "bezierCurveTo").map_err(|error| raise(&ctx, &error))?;
        self.change(&ctx, |path| {
            path.cubic_to(nums[0], nums[1], nums[2], nums[3], nums[4], nums[5])
        })
    }

    fn rect<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<4>(&args.0, "rect").map_err(|error| raise(&ctx, &error))?;
        self.change(&ctx, |path| path.rect(nums[0], nums[1], nums[2], nums[3]))
    }

    fn round_rect<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<4>(&args.0, "roundRect").map_err(|error| raise(&ctx, &error))?;
        let radii = radii_of(&ctx, args.0.get(4))?;
        self.change(&ctx, |path| {
            path.round_rect(nums[0], nums[1], nums[2], nums[3], &radii)
        })
    }

    fn arc<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<5>(&args.0, "arc").map_err(|error| raise(&ctx, &error))?;
        let ccw = optional_bool(args.0.get(5), "arc").map_err(|error| raise(&ctx, &error))?;
        self.change(&ctx, |path| {
            path.arc(nums[0], nums[1], nums[2], nums[3], nums[4], ccw)
        })
    }

    fn arc_to<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<5>(&args.0, "arcTo").map_err(|error| raise(&ctx, &error))?;
        self.change(&ctx, |path| {
            path.arc_to(nums[0], nums[1], nums[2], nums[3], nums[4])
        })
    }

    fn ellipse<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let nums = numbers::<7>(&args.0, "ellipse").map_err(|error| raise(&ctx, &error))?;
        let ccw = optional_bool(args.0.get(7), "ellipse").map_err(|error| raise(&ctx, &error))?;
        self.change(&ctx, |path| {
            path.ellipse(
                nums[0], nums[1], nums[2], nums[3], nums[4], nums[5], nums[6], ccw,
            )
        })
    }
}

fn build_path(source: &Option<Value<'_>>) -> Result<Path2D, CanvasError> {
    match source {
        None => Ok(Path2D::new()),
        Some(value) if value.is_undefined() => Ok(Path2D::new()),
        Some(value) if value.is_string() => {
            Ok(Path2D::from_svg(&js_string(value).unwrap_or_default()))
        }
        Some(value) if is_path(value) => {
            cloned_path(value).ok_or_else(|| CanvasError::argument("Path2D", "expected a path"))
        }
        Some(_) => Err(CanvasError::argument(
            "Path2D",
            "expected a path or an SVG string",
        )),
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "CanvasGradient")]
struct JsGradient {
    #[qjs(skip_trace)]
    inner: Weak<Inner>,
    #[qjs(skip_trace)]
    state: RefCell<GradientState>,
}

impl Drop for JsGradient {
    fn drop(&mut self) {
        let charged = self.state.get_mut().charged;
        if charged != 0
            && let Some(inner) = self.inner.upgrade()
        {
            session_budget(&inner).refund(charged);
        }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl JsGradient {
    fn add_color_stop<'js>(&self, ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
        let offset = args.0.first().and_then(Value::as_number).ok_or_else(|| {
            raise(
                &ctx,
                &CanvasError::argument("addColorStop", "expected a number"),
            )
        })?;
        let color = args.0.get(1).and_then(js_string).ok_or_else(|| {
            raise(
                &ctx,
                &CanvasError::argument("addColorStop", "expected a color"),
            )
        })?;
        let host = host_from_ctx(&ctx)?;
        let budget = session_budget(host.inner());
        budget
            .reserve(STOP_STEP)
            .map_err(|error| raise(&ctx, &error))?;
        let result = {
            let mut state = self.state.borrow_mut();
            let result = state.gradient.add_color_stop(offset, &color);
            if result.is_ok() {
                state.charged = state.charged.saturating_add(STOP_STEP);
            }
            result
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                budget.refund(STOP_STEP);
                Err(raise(&ctx, &error))
            }
        }
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "CanvasController")]
struct Controller {
    index: u32,
    generation: u32,
    #[qjs(skip_trace)]
    released: Cell<bool>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl Controller {
    fn invalidate<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<()> {
        if self.released.get() {
            return Ok(());
        }
        let host = host_from_ctx(&ctx)?;
        if host.inner().graph.is_painting() {
            host.inner().canvas.borrow_mut().blocked = true;
            return Err(raise(&ctx, &side_effect_error()));
        }
        let request = {
            let mut session = host.inner().canvas.borrow_mut();
            if let Some(binding) = binding_at_mut(&mut session, self.index, self.generation)
                && binding.alive
            {
                binding.pending = true;
                !binding.snap.offscreen
            } else {
                false
            }
        };
        if request && !host.paused.get() && !host.inner().paused.get() {
            host.wants_frame.set(true);
        }
        Ok(())
    }

    fn to_local<'js>(&self, ctx: Ctx<'js>, point: Value<'js>) -> rquickjs::Result<Value<'js>> {
        if self.released.get() {
            return Ok(Value::new_null(ctx));
        }
        let host = host_from_ctx(&ctx)?;
        let alive = {
            let session = host.inner().canvas.borrow();
            binding_at(&session, self.index, self.generation).is_some_and(|binding| binding.alive)
        };
        if !alive {
            return Ok(Value::new_null(ctx));
        }
        let Some(object) = point.as_object() else {
            return Err(raise(
                &ctx,
                &CanvasError::argument("toLocal", "expected a point"),
            ));
        };
        let x = object
            .get::<_, Value<'_>>("x")
            .ok()
            .and_then(|value| value.as_number());
        let y = object
            .get::<_, Value<'_>>("y")
            .ok()
            .and_then(|value| value.as_number());
        let (Some(x), Some(y)) = (x, y) else {
            return Err(raise(
                &ctx,
                &CanvasError::argument("toLocal", "expected a finite point"),
            ));
        };
        if !x.is_finite() || !y.is_finite() {
            return Err(raise(
                &ctx,
                &CanvasError::argument("toLocal", "expected a finite point"),
            ));
        }
        let node = {
            let session = host.inner().canvas.borrow();
            binding_at(&session, self.index, self.generation).map(|binding| binding.node)
        };
        let Some(node) = node else {
            return Ok(Value::new_null(ctx.clone()));
        };
        let local = host.inner().tree.borrow().canvas_local(
            NodeId::from_raw(node),
            narrow_f32(x),
            narrow_f32(y),
        );
        let Some((x, y)) = local else {
            return Ok(Value::new_null(ctx));
        };
        let object = Object::new(ctx.clone())?;
        object.set("x", f64::from(x))?;
        object.set("y", f64::from(y))?;
        Ok(object.into_value())
    }

    fn release<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<()> {
        if self.released.get() {
            return Ok(());
        }
        let host = host_from_ctx(&ctx)?;
        if host.inner().graph.is_painting() {
            host.inner().canvas.borrow_mut().blocked = true;
            return Err(raise(&ctx, &side_effect_error()));
        }
        self.released.set(true);
        release_binding(&host, self.index, self.generation);
        Ok(())
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "scene points are f32 after a finite check"
)]
fn narrow_f32(value: f64) -> f32 {
    value as f32
}
