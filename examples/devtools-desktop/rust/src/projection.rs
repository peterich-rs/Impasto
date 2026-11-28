use std::collections::BTreeMap;

use devtools_protocol::FrameRecord;
use impasto::{Bytes, ImpastoType};
use serde_json::{Value, json};

use crate::analyzer::{
    flamegraph, gantt,
    treemap::{self, Rect},
};
use crate::store::{Recording, frames, network};
use crate::types::{DetailField, fields, matches, number};

#[derive(ImpastoType)]
pub struct RectBucket {
    pub color: u16,
    pub rects: Bytes,
}

#[derive(ImpastoType)]
pub struct LineBucket {
    pub color: u16,
    pub width: f64,
    pub dashed: bool,
    pub points: Bytes,
}

#[derive(ImpastoType)]
pub struct CanvasLabel {
    pub x: f64,
    pub y: f64,
    pub max_width: f64,
    pub color: u16,
    pub align: u16,
    pub text: String,
}

#[derive(Default, ImpastoType)]
pub struct ProjectedCanvas {
    pub rect_buckets: Vec<RectBucket>,
    pub polylines: Vec<LineBucket>,
    pub segments: Vec<LineBucket>,
    pub labels: Vec<CanvasLabel>,
    pub min_x: f64,
    pub max_x: f64,
    pub max_y: f64,
}

#[derive(ImpastoType)]
pub struct Hit {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub path: Vec<String>,
    pub fields: Vec<DetailField>,
}

struct Plot {
    width: f64,
    height: f64,
    rects: BTreeMap<u16, Vec<u8>>,
    canvas: ProjectedCanvas,
}

impl Plot {
    fn new(width: f64, height: f64) -> Self {
        Self {
            width: dimension(width),
            height: dimension(height),
            rects: BTreeMap::new(),
            canvas: ProjectedCanvas::default(),
        }
    }

    fn rect(&mut self, color: u16, rect: Rect) {
        let x = rect.x.clamp(0., self.width);
        let y = rect.y.clamp(0., self.height);
        let w = (rect.x + rect.w).clamp(0., self.width) - x;
        let h = (rect.y + rect.h).clamp(0., self.height) - y;
        if w > 0. && h > 0. {
            self.rects
                .entry(color)
                .or_default()
                .extend([x, y, w, h].into_iter().flat_map(f64::to_le_bytes));
        }
    }

    fn line(
        &mut self,
        color: u16,
        points: impl IntoIterator<Item = (f64, f64)>,
        dashed: bool,
        segments: bool,
    ) {
        let points = points
            .into_iter()
            .flat_map(|(x, y)| {
                [x.clamp(0., self.width), y.clamp(0., self.height)]
                    .into_iter()
                    .flat_map(f64::to_le_bytes)
            })
            .collect();
        let bucket = LineBucket {
            color,
            width: 1.,
            dashed,
            points: Bytes(points),
        };
        if segments {
            self.canvas.segments.push(bucket);
        } else {
            self.canvas.polylines.push(bucket);
        }
    }

    fn label(&mut self, text: impl Into<String>, x: f64, y: f64, max_width: f64) {
        if self.canvas.labels.len() < 256
            && max_width >= 10.
            && y >= 0.
            && y <= self.height
            && x <= self.width
        {
            self.canvas.labels.push(CanvasLabel {
                x,
                y,
                max_width: max_width.min(self.width - x),
                color: 85,
                align: 0,
                text: text.into(),
            });
        }
    }

    fn finish(mut self) -> ProjectedCanvas {
        self.canvas.rect_buckets = self
            .rects
            .into_iter()
            .map(|(color, rects)| RectBucket {
                color,
                rects: Bytes(rects),
            })
            .collect();
        self.canvas
    }
}

pub(crate) fn dimension(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0., 16_384.)
    } else {
        0.
    }
}

fn span(start: f64, end: f64, available: f64) -> (f64, f64) {
    let start = if start.is_finite() { start.max(0.) } else { 0. };
    let end = if end.is_finite() && end > start {
        end
    } else {
        available.max(start + 0.01)
    };
    (start, end)
}

fn frame_window(data: &Recording, start: u32, visible: u32) -> impl Iterator<Item = &FrameRecord> {
    data.frames
        .entries()
        .iter()
        .skip(start as usize)
        .take(visible.clamp(1, 2400) as usize)
}

pub(crate) fn frame_strip(
    data: &Recording,
    width: f64,
    height: f64,
    start: u32,
    visible: u32,
    selected: Option<u64>,
    range: Option<(u64, u64)>,
) -> ProjectedCanvas {
    let mut plot = Plot::new(width, height);
    let pitch = plot.width / f64::from(visible.clamp(1, 2400));
    let maximum = frame_window(data, start, visible)
        .map(|entry| frames::duration(entry).max(entry.budget_ms))
        .fold(1_f64, f64::max)
        * 1.12;
    let height = (plot.height - 14.).max(0.);
    let scale = height / maximum;
    let mut outlines = Vec::new();
    for (column, frame) in frame_window(data, start, visible).enumerate() {
        let x = column as f64 * pitch;
        let w = (pitch - 1.).max(pitch * 0.6);
        let bottom = plot.height - 14.;
        let mut y = bottom;
        let series = if frame.compositor_only {
            [0., 0., 0., 0., 0., 0., frames::duration(frame).max(0.1)]
        } else {
            [
                frame.script_ms,
                frame.layout_ms,
                frame.record_ms,
                frame.queue_ms,
                frame.raster_ms,
                frame.gpu.frame_span_us.unwrap_or_default() / 1_000.,
                0.,
            ]
        };
        for (color, duration) in series.into_iter().enumerate() {
            let h = (duration * scale).max(0.);
            y -= h;
            plot.rect(color as u16, Rect { x, y, w, h });
        }
        if frame.skip_reason.is_some() {
            plot.rect(
                7,
                Rect {
                    x,
                    y: bottom - 4.,
                    w,
                    h: 4.,
                },
            );
        } else if frame.over_budget || frame.missed_vsync {
            plot.rect(
                7,
                Rect {
                    x,
                    y: y - 3.,
                    w,
                    h: 3.,
                },
            );
        }
        if range.is_some_and(|(a, b)| (a.min(b)..=a.max(b)).contains(&frame.id)) {
            plot.rect(
                83,
                Rect {
                    x,
                    y: 0.,
                    w: pitch,
                    h: bottom,
                },
            );
        }
        if selected == Some(frame.id) {
            outlines.extend([(x, 0.), (x, bottom), (x + w, 0.), (x + w, bottom)]);
        }
        if pitch >= 5. && column % ((80. / pitch) as usize).max(1) == 0 {
            plot.label(format!("#{}", frame.id), x + 2., bottom + 7., 72.);
        }
    }
    if let Some(last) = frame_window(data, start, visible)
        .last()
        .filter(|frame| frame.budget_ms > 0.)
    {
        let y = height - last.budget_ms * scale;
        plot.line(81, [(0., y), (plot.width, y)], true, true);
    }
    plot.line(84, outlines, false, true);
    plot.canvas.max_y = maximum;
    plot.finish()
}

pub(crate) fn hit_frame_strip(
    data: &Recording,
    x: f64,
    width: f64,
    start: u32,
    visible: u32,
) -> Option<Hit> {
    let width = dimension(width);
    if !(0.0..width).contains(&x) {
        return None;
    }
    let column = (x / width * f64::from(visible.clamp(1, 2400))) as usize;
    let frame = frame_window(data, start, visible).nth(column)?;
    Some(Hit {
        id: frame.id.to_string(),
        name: format!("Frame #{}", frame.id),
        detail: format!(
            "{:.2} ms · {:.2} ms budget",
            frames::duration(frame),
            frame.budget_ms
        ),
        path: Vec::new(),
        fields: Vec::new(),
    })
}

#[derive(Clone, Copy)]
pub(crate) struct Viewport {
    pub width: f64,
    pub height: f64,
    pub start: f64,
    pub end: f64,
}

const GANTT_HEAD: f64 = 92.;

fn gantt_rect(bar: &gantt::Bar, lane_height: f64, viewport: Viewport) -> Option<Rect> {
    let x_scale = (viewport.width - GANTT_HEAD - 4.).max(0.) / (viewport.end - viewport.start);
    let x = (GANTT_HEAD + (bar.start - viewport.start) * x_scale).max(GANTT_HEAD);
    let end = (GANTT_HEAD + (bar.start + bar.duration - viewport.start) * x_scale)
        .min(viewport.width - 4.);
    let depth = bar.depth as f64;
    let row = ((lane_height - 10.) / 9.).max(1.);
    let y = bar.lane as f64 * lane_height + 7. + depth * row + 16.;
    (end > x).then_some(Rect {
        x,
        y,
        w: (end - x).max(1.).min(viewport.width - x),
        h: (lane_height - depth * row - 12.).clamp(1., 18.),
    })
}

pub(crate) fn gantt(
    data: &Recording,
    id: u64,
    mut viewport: Viewport,
    hit: Option<(f64, f64)>,
) -> (ProjectedCanvas, Option<Hit>) {
    let mut plot = Plot::new(viewport.width, viewport.height);
    let Some(frame) = data.frame(id) else {
        return (plot.finish(), None);
    };
    let bars = gantt::bars(&frame, data.spans.get(&id));
    viewport.width = plot.width;
    viewport.height = plot.height;
    (viewport.start, viewport.end) = span(
        viewport.start,
        viewport.end,
        bars.iter()
            .map(|bar| bar.start + bar.duration)
            .fold(frame.budget_ms, f64::max),
    );
    let lane_height = ((viewport.height - 16.) / 5.).max(1.);
    let mut picked = None;
    let mut picked_depth = 0;
    let mut minor = BTreeMap::<(usize, i64), Rect>::new();
    for (index, label) in gantt::LANES.into_iter().enumerate() {
        let y = 16. + lane_height * index as f64;
        plot.line(80, [(0., y), (plot.width, y)], false, true);
        plot.label(label, 6., y + 12., GANTT_HEAD - 12.);
    }
    for tick in 0..=5 {
        let ratio = f64::from(tick) / 5.;
        let x = GANTT_HEAD + ratio * (plot.width - GANTT_HEAD - 4.).max(0.);
        plot.line(80, [(x, 16.), (x, plot.height)], false, true);
        plot.label(
            format!(
                "{:.2} ms",
                viewport.start + ratio * (viewport.end - viewport.start)
            ),
            x + 3.,
            8.,
            68.,
        );
    }
    for bar in bars {
        let Some(rect) = gantt_rect(&bar, lane_height, viewport) else {
            continue;
        };
        if hit.is_some_and(|(x, y)| rect.contains(x, y))
            && (picked.is_none() || bar.depth >= picked_depth)
        {
            picked_depth = bar.depth;
            picked = Some(Hit {
                id: bar.id,
                name: bar.label.clone(),
                detail: format!("{:.3} ms", bar.duration),
                fields: fields(&bar.fields),
                path: Vec::new(),
            });
        }
        if rect.w < 1.5 {
            minor.entry((bar.lane, rect.x as i64)).or_insert(rect);
        } else {
            plot.rect(bar.color, rect);
        }
        if rect.w >= 48. {
            plot.label(bar.label, rect.x + 4., rect.y + rect.h / 2., rect.w - 8.);
        }
    }
    for rect in minor.into_values() {
        plot.rect(85, rect);
    }
    plot.canvas.min_x = viewport.start;
    plot.canvas.max_x = viewport.end;
    (plot.finish(), picked)
}

pub(crate) fn flamegraph(
    tree: &flamegraph::Tree,
    width: f64,
    height: f64,
    zoom: u32,
    search: &str,
    hit: Option<(f64, f64)>,
) -> (ProjectedCanvas, Option<Hit>) {
    let mut plot = Plot::new(width, height);
    let query = search.to_lowercase();
    let mut selected = None;
    for block in tree.blocks(zoom, 0.75 / plot.width.max(1.)) {
        let level = if zoom == 0 {
            block.depth.saturating_sub(1)
        } else {
            block.depth
        };
        let rect = Rect {
            x: block.start * plot.width,
            y: f64::from(level) * 23. + 1.,
            w: ((block.end - block.start) * plot.width - 1.).max(0.),
            h: 21.,
        };
        if rect.y >= plot.height {
            continue;
        }
        if hit.is_some_and(|(x, y)| rect.contains(x, y)) {
            selected = Some(Hit {
                id: block.id.to_string(),
                name: block.symbol.to_owned(),
                detail: format!("{} self · {} total samples", block.own, block.total),
                path: Vec::new(),
                fields: fields(
                    &json!({"symbol": block.symbol, "self_samples": block.own, "total_samples": block.total,
                    "self_percent_of_total": f64::from(block.own) / f64::from(block.total) * 100.}),
                ),
            });
        }
        let hue = block.symbol.bytes().fold(0u16, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u16::from(byte))
        });
        plot.rect(
            if matches(block.symbol, &query) {
                48 + hue % 6
            } else {
                80
            },
            rect,
        );
        if rect.w >= 42. {
            plot.label(block.symbol, rect.x + 4., rect.y + 10., rect.w - 8.);
        }
    }
    (plot.finish(), selected)
}

pub(crate) fn bundle(
    bundle: &treemap::Bundle,
    width: f64,
    height: f64,
    path: &[String],
    metric: &str,
    hit: Option<(f64, f64)>,
) -> (ProjectedCanvas, Option<Hit>) {
    let mut plot = Plot::new(width, height);
    let items = bundle.children(path, metric);
    let weights: Vec<_> = items.iter().map(|item| item.bytes).collect();
    let mut selected = None;
    for (index, rect) in treemap::squarify(&weights, plot.width, plot.height) {
        let item = &items[index];
        if hit.is_some_and(|(x, y)| rect.contains(x, y)) {
            let detail = item
                .module
                .as_ref()
                .and_then(|module| serde_json::to_value(module).ok())
                .unwrap_or_else(|| json!({"bytes": item.bytes, "path": item.path.join("/")}));
            selected = Some(Hit {
                id: item.path.join("/"),
                name: item.label.clone(),
                detail: format!("{} bytes", item.bytes),
                fields: fields(&detail),
                path: if item.module.is_none() {
                    item.path.clone()
                } else {
                    Vec::new()
                },
            });
        }
        plot.rect(
            item.color,
            Rect {
                x: rect.x + 0.5,
                y: rect.y + 0.5,
                w: (rect.w - 1.).max(0.),
                h: (rect.h - 1.).max(0.),
            },
        );
        if rect.w >= 32. && rect.h >= 16. {
            plot.label(&item.label, rect.x + 6., rect.y + 11., rect.w - 12.);
        }
    }
    (plot.finish(), selected)
}

pub(crate) fn waterfall(value: &Value, width: f64, height: f64) -> ProjectedCanvas {
    let mut plot = Plot::new(width, height);
    let row = plot.height / 9.;
    let bar_width = (plot.width - 163.).max(1.);
    let phases = network::TIMINGS.map(|(key, name)| (name, value[key].as_f64()));
    let sum: f64 = phases.iter().filter_map(|(_, duration)| *duration).sum();
    let total = number(value, "total_ms").max(sum);
    let mut start = 0.;
    for (index, (name, duration)) in phases
        .into_iter()
        .chain([("Total", Some(total))])
        .enumerate()
    {
        let y = index as f64 * row;
        plot.label(name, 4., y + row / 2., 90.);
        if let Some(duration) = duration.filter(|_| total > 0.) {
            let ratio = duration / total;
            plot.rect(
                if index < 8 { 16 + index as u16 } else { 84 },
                Rect {
                    x: 96. + if index < 8 { start } else { 0. },
                    y: y + 4.,
                    w: bar_width * ratio,
                    h: row - 8.,
                },
            );
            start += bar_width * ratio;
            plot.label(
                format!("{duration:.2}"),
                100. + bar_width,
                y + row / 2.,
                60.,
            );
        }
    }
    plot.finish()
}

struct Series {
    color: u16,
    points: Vec<(f64, f64)>,
}

fn series(color: u16, points: impl IntoIterator<Item = (f64, f64)>) -> Series {
    Series {
        color,
        points: points.into_iter().collect(),
    }
}

fn envelope(points: impl Iterator<Item = (f64, f64)>, columns: usize) -> Vec<(f64, f64)> {
    let mut output = Vec::new();
    let mut bin = None::<(usize, (f64, f64), (f64, f64))>;
    let append = |output: &mut Vec<_>, min: (f64, f64), max: (f64, f64)| {
        if min.0 <= max.0 {
            output.extend([min, max]);
        } else {
            output.extend([max, min]);
        }
    };
    for point in points {
        let key = (point.0 * columns as f64).floor() as usize;
        match bin {
            Some((current, min, max)) if current == key => {
                bin = Some((
                    key,
                    if point.1 < min.1 { point } else { min },
                    if point.1 > max.1 { point } else { max },
                ));
            }
            Some((_, min, max)) => {
                append(&mut output, min, max);
                bin = Some((key, point, point));
            }
            None => bin = Some((key, point, point)),
        }
    }
    if let Some((_, min, max)) = bin {
        append(&mut output, min, max);
    }
    output
}

fn tracks(
    width: f64,
    height: f64,
    start: f64,
    end: f64,
    series: &[Series],
    budget: Option<f64>,
) -> ProjectedCanvas {
    let mut plot = Plot::new(width, height);
    let maximum = series
        .iter()
        .flat_map(|series| &series.points)
        .filter(|(time, _)| (start..=end).contains(time))
        .map(|(_, value)| *value)
        .fold(budget.unwrap_or(1.).max(1.), f64::max)
        * 1.08;
    for i in 0..=4 {
        let x = plot.width * f64::from(i) / 4.;
        plot.line(80, [(x, 0.), (x, plot.height)], false, true);
    }
    if let Some(budget) = budget {
        let y = plot.height * (1. - budget / maximum);
        plot.line(81, [(0., y), (plot.width, y)], true, true);
    }
    for series in series {
        let points = series
            .points
            .iter()
            .copied()
            .filter(|(time, _)| (start..=end).contains(time))
            .map(|(time, value)| ((time - start) / (end - start), value));
        let points = envelope(points, plot.width.ceil().max(1.) as usize)
            .into_iter()
            .map(|(x, y)| (x * plot.width, plot.height * (1. - y / maximum)))
            .collect::<Vec<_>>();
        plot.line(series.color, points, false, false);
    }
    plot.canvas.min_x = start;
    plot.canvas.max_x = end;
    plot.canvas.max_y = maximum;
    plot.finish()
}

pub(crate) fn overview(
    data: &Recording,
    width: f64,
    height: f64,
    seconds: u32,
) -> Vec<ProjectedCanvas> {
    let frames = data.frames.entries();
    let end = frames
        .back()
        .map_or(0., |frame| frame.at_ms)
        .max(
            data.runtime
                .back()
                .map_or(0., |entry| number(entry, "at_ms")),
        )
        .max(
            data.network
                .values()
                .map(|record| number(record, "at_ms").max(number(record, "done_ms")))
                .fold(0., f64::max),
        );
    let start = end - f64::from(seconds.clamp(1, 120)) * 1000.;
    let budget = frames
        .back()
        .map(|frame| frame.budget_ms)
        .filter(|&budget| budget > 0.);
    let mut requests = Vec::new();
    let mut durations = Vec::new();
    for entry in data.network.values() {
        let begin = number(entry, "at_ms");
        requests.push((begin, 1));
        if let Some(finish) = entry["done_ms"].as_f64() {
            requests.push((finish.max(begin), -1));
            durations.push((finish, network::elapsed(entry)));
        }
    }
    requests.sort_by(|a, b| a.0.total_cmp(&b.0));
    durations.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut active = 0_i32;
    vec![
        tracks(
            width,
            height,
            start,
            end,
            &[series(
                84,
                frames
                    .iter()
                    .map(|frame| (frame.at_ms, frames::duration(frame))),
            )],
            budget,
        ),
        tracks(
            width,
            height,
            start,
            end,
            &[
                series(0, frames.iter().map(|frame| (frame.at_ms, frame.ui_ms))),
                series(4, frames.iter().map(|frame| (frame.at_ms, frame.raster_ms))),
                series(
                    5,
                    frames
                        .iter()
                        .filter_map(|frame| Some((frame.at_ms, frame.gpu.frame_span_us? / 1000.))),
                ),
            ],
            None,
        ),
        tracks(
            width,
            height,
            start,
            end,
            &[series(
                48,
                data.runtime.iter().map(|entry| {
                    (
                        number(entry, "at_ms"),
                        number(&entry["value"]["detail"], "memory_used_size") / 1048576.,
                    )
                }),
            )],
            None,
        ),
        tracks(
            width,
            height,
            start,
            end,
            &[
                series(
                    22,
                    requests.into_iter().map(|(time, delta)| {
                        active += delta;
                        (time, f64::from(active.max(0)))
                    }),
                ),
                series(
                    23,
                    durations.into_iter().map(|(time, ms)| (time, ms / 1000.)),
                ),
            ],
            None,
        ),
    ]
}

pub(crate) fn heap(data: &Recording, width: f64, height: f64) -> ProjectedCanvas {
    let points: Vec<_> = data
        .runtime
        .iter()
        .filter_map(|row| {
            let detail = row["value"].get("detail")?;
            let ts = row["at_ms"].as_f64()?;
            Some((ts, heap_values(detail)))
        })
        .collect();
    let mut plot = Plot::new(width, height);
    let Some((start, _)) = points.first() else {
        return plot.finish();
    };
    let end = points
        .last()
        .map_or(*start, |entry| entry.0)
        .max(start + 1.);
    let max = points
        .iter()
        .map(|(_, values)| values.iter().sum::<f64>())
        .fold(1., f64::max)
        * 1.05;
    for (index, (time, amounts)) in points.iter().enumerate() {
        let x = (time - start) / (end - start) * (plot.width - 1.).max(0.);
        let right = points.get(index + 1).map_or(plot.width, |(time, _)| {
            (time - start) / (end - start) * plot.width
        });
        let mut y = plot.height;
        for (class, amount) in amounts.iter().enumerate() {
            let h = amount / max * plot.height;
            y -= h;
            plot.rect(
                48 + class as u16,
                Rect {
                    x,
                    y,
                    w: (right - x).max(1.),
                    h,
                },
            );
        }
    }
    plot.canvas.min_x = *start;
    plot.canvas.max_x = end;
    plot.canvas.max_y = max;
    plot.finish()
}

pub(crate) fn heap_values(value: &Value) -> [f64; 7] {
    let mut amounts = [
        number(value, "obj_size") + number(value, "prop_size"),
        number(value, "str_size"),
        number(value, "atom_size"),
        number(value, "shape_size"),
        number(value, "js_func_size")
            + number(value, "js_func_code_size")
            + number(value, "js_func_pc2line_size"),
        number(value, "binary_object_size"),
        0.,
    ];
    amounts[6] = (number(value, "memory_used_size") - amounts.iter().sum::<f64>()).max(0.);
    amounts
}

pub(crate) fn resources(data: &Recording, width: f64, height: f64) -> Vec<ProjectedCanvas> {
    let samples = &data.resource_samples;
    let start = samples.front().map_or(0., |sample| sample.at_ms);
    let end = samples
        .back()
        .map_or(start, |sample| sample.at_ms)
        .max(start + 1.);
    vec![
        tracks(
            width,
            height,
            start,
            end,
            &[
                series(
                    48,
                    samples
                        .iter()
                        .map(|sample| (sample.at_ms, sample.atlas_bytes as f64 / 1048576.)),
                ),
                series(
                    51,
                    samples.iter().map(|sample| {
                        (sample.at_ms, sample.offscreen_peak_bytes as f64 / 1048576.)
                    }),
                ),
                series(
                    49,
                    samples.iter().map(|sample| {
                        (sample.at_ms, sample.image_resident_bytes as f64 / 1048576.)
                    }),
                ),
                series(
                    52,
                    data.runtime.iter().map(|entry| {
                        (
                            number(entry, "at_ms"),
                            number(&entry["value"]["detail"], "memory_used_size") / 1048576.,
                        )
                    }),
                ),
            ],
            None,
        ),
        tracks(
            width,
            height,
            start,
            end,
            &[
                series(
                    48,
                    samples
                        .iter()
                        .map(|sample| (sample.at_ms, f64::from(sample.atlas_entries))),
                ),
                series(
                    51,
                    samples
                        .iter()
                        .map(|sample| (sample.at_ms, f64::from(sample.textures))),
                ),
                series(
                    49,
                    samples
                        .iter()
                        .map(|sample| (sample.at_ms, f64::from(sample.pictures))),
                ),
                series(
                    52,
                    samples
                        .iter()
                        .map(|sample| (sample.at_ms, f64::from(sample.layers))),
                ),
            ],
            None,
        ),
        tracks(
            width,
            height,
            start,
            end,
            &[
                series(
                    49,
                    samples.iter().map(|sample| {
                        let hits = f64::from(sample.atlas_hits);
                        let rate = hits / (hits + f64::from(sample.atlas_misses)).max(1.) * 100.;
                        (sample.at_ms, rate)
                    }),
                ),
                series(
                    52,
                    samples
                        .iter()
                        .map(|sample| (sample.at_ms, f64::from(sample.atlas_evictions))),
                ),
            ],
            None,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_clips_rects_and_agrees_with_frame_hit() {
        let mut data = Recording::default();
        data.frames.update(
            false,
            vec![
                FrameRecord {
                    id: 7,
                    at_ms: 1.,
                    budget_ms: 16.,
                    script_ms: 12.,
                    raster_ms: 42.,
                    ..FrameRecord::default()
                },
                FrameRecord {
                    id: 9,
                    at_ms: 4.,
                    budget_ms: 16.,
                    script_ms: 24.,
                    ..FrameRecord::default()
                },
            ],
        );
        let plot = frame_strip(&data, 100., 100., 0, 2, Some(9), None);
        for bucket in &plot.rect_buckets {
            assert_eq!(bucket.rects.0.len() % 32, 0);
            for rect in bucket.rects.0.as_chunks::<32>().0 {
                let coords: Vec<_> = rect
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .copied()
                    .map(f64::from_le_bytes)
                    .collect();
                assert!(coords[0] >= 0. && coords[1] >= 0.);
                assert!(coords[0] + coords[2] <= 100. && coords[1] + coords[3] <= 100.);
            }
        }
        assert_eq!(
            hit_frame_strip(&data, 50., 100., 0, 2).map(|hit| hit.id),
            Some("9".to_owned())
        );
        assert!(hit_frame_strip(&data, 100., 100., 0, 2).is_none());
    }
}
