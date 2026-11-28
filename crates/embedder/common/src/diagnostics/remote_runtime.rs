use engine::session::{JsGcResult, JsMemoryDetail, JsResourcesSnapshot, TimerKind};
use serde_json::{Value, json};

pub(crate) fn memory(detail: JsMemoryDetail) -> Value {
    json!({
        "malloc_size": detail.malloc_size,
        "malloc_limit": detail.malloc_limit,
        "memory_used_size": detail.memory_used_size,
        "malloc_count": detail.malloc_count,
        "memory_used_count": detail.memory_used_count,
        "atom_count": detail.atom_count,
        "atom_size": detail.atom_size,
        "str_count": detail.str_count,
        "str_size": detail.str_size,
        "obj_count": detail.obj_count,
        "obj_size": detail.obj_size,
        "prop_count": detail.prop_count,
        "prop_size": detail.prop_size,
        "shape_count": detail.shape_count,
        "shape_size": detail.shape_size,
        "js_func_count": detail.js_func_count,
        "js_func_size": detail.js_func_size,
        "js_func_code_size": detail.js_func_code_size,
        "js_func_pc2line_count": detail.js_func_pc2line_count,
        "js_func_pc2line_size": detail.js_func_pc2line_size,
        "c_func_count": detail.c_func_count,
        "array_count": detail.array_count,
        "fast_array_count": detail.fast_array_count,
        "fast_array_elements": detail.fast_array_elements,
        "binary_object_count": detail.binary_object_count,
        "binary_object_size": detail.binary_object_size,
    })
}

pub(crate) fn gc(result: JsGcResult) -> Value {
    json!({
        "before": memory(result.before), "after": memory(result.after),
        "freed_bytes": result.freed_bytes, "duration_ms": result.duration_ms,
    })
}

pub(crate) fn resources(snapshot: JsResourcesSnapshot) -> Value {
    let counts = snapshot.counts;
    let timers: Vec<_> = snapshot
        .active_timers
        .iter()
        .map(|timer| {
            json!({
                "id": timer.id,
                "kind": match timer.kind {
                    TimerKind::Timeout => "timeout",
                    TimerKind::Interval => "interval",
                    TimerKind::AnimationFrame => "animation_frame",
                },
                "due_in_ms": timer.due_in_ms, "interval_ms": timer.interval_ms,
            })
        })
        .collect();
    json!({
        "counts": {
            "promises": counts.promises,
            "timers": counts.timers,
            "tickets": counts.tickets,
            "rejections": counts.rejections,
            "listeners": counts.listeners,
            "requests": counts.requests,
            "queued_events": counts.queued_events,
            "queued_bytes": counts.queued_bytes,
            "peak_promises": counts.peak_promises,
            "peak_timers": counts.peak_timers,
            "peak_listeners": counts.peak_listeners,
            "peak_tickets": counts.peak_tickets,
            "peak_rejections": counts.peak_rejections,
            "peak_requests": counts.peak_requests,
            "peak_queued_events": counts.peak_queued_events,
            "peak_queued_bytes": counts.peak_queued_bytes,
            "discarded_events": counts.discarded_events,
            "rejection_overflows": counts.rejection_overflows,
        },
        "active_timers": timers, "timers_truncated": snapshot.timers_truncated,
        "pending_promises": snapshot.pending_promises, "promises_truncated": snapshot.promises_truncated,
        "pending_modules": snapshot.pending_modules, "job_limit_hits": snapshot.job_limit_hits,
        "budget_overruns": snapshot.budget_overruns,
    })
}
