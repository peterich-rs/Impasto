use super::{
    JsGcResult, JsMemoryDetail, JsResourcesSnapshot, Runtime, SamplerConfig, TimerKind,
    TimerSnapshot, TrackedModule,
};

const RESOURCE_LIMIT: usize = 256;

impl Runtime {
    /// Reads allocator statistics on the owner thread between script turns.
    pub fn memory_detail(&self) -> Option<JsMemoryDetail> {
        self.state.vm.memory_detail()
    }

    /// Explicitly performs a collection between script turns.
    pub fn run_gc(&self) -> Option<JsGcResult> {
        self.state.vm.run_gc()
    }

    /// Applies limits to the configured stack sampler.
    pub fn set_sampler_config(&self, config: SamplerConfig) -> bool {
        self.state.vm.set_sampler(config)
    }

    /// Reads async state without invoking callbacks. Host-owned animation frames use the same clock.
    pub fn resources_snapshot(
        &self,
        animation_frames: impl IntoIterator<Item = TimerSnapshot>,
    ) -> JsResourcesSnapshot {
        let state = &self.state;
        let now = self.clock_ms();
        let mut active_timers: Vec<_> = state
            .scheduler
            .timers
            .borrow()
            .iter()
            .filter(|timer| timer.active)
            .map(|timer| TimerSnapshot {
                id: timer.id,
                kind: match timer.kind {
                    1 => TimerKind::Interval,
                    2 => TimerKind::AnimationFrame,
                    _ => TimerKind::Timeout,
                },
                due_in_ms: timer.due.saturating_sub(now),
                interval_ms: timer.interval,
            })
            .chain(animation_frames)
            .collect();
        active_timers.sort_unstable_by_key(|timer| (timer.due_in_ms, timer.id));
        let timers_truncated = active_timers.len().saturating_sub(RESOURCE_LIMIT);
        active_timers.truncate(RESOURCE_LIMIT);
        let mut pending_promises: Vec<_> = state.vm.promises.borrow().keys().copied().collect();
        pending_promises.sort_unstable();
        let promises_truncated = pending_promises.len().saturating_sub(RESOURCE_LIMIT);
        pending_promises.truncate(RESOURCE_LIMIT);
        let mut pending_modules: Vec<_> = state
            .tickets
            .borrow()
            .values()
            .filter_map(|ticket| match ticket {
                TrackedModule::Pending { module, .. } => Some(module.clone()),
                TrackedModule::Finished(_) => None,
            })
            .collect();
        pending_modules.sort_unstable();
        let mut counts = self.counts();
        counts.timers = active_timers.len() + timers_truncated;
        counts.peak_timers = counts.peak_timers.max(counts.timers);
        JsResourcesSnapshot {
            counts,
            active_timers,
            timers_truncated,
            pending_promises,
            promises_truncated,
            pending_modules,
            job_limit_hits: state.vm.job_limit_hits.get(),
            budget_overruns: state.vm.budget_overruns.get(),
        }
    }
}
