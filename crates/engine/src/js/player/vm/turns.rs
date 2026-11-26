//! UI callback turns and animation frames.
use super::*;

fn number<'js>(ctx: &Ctx<'js>, number: impl Into<f64>) -> Value<'js> {
    Value::new_float(ctx.clone(), number.into())
}

const KINDS: [&str; 23] = [
    "click",
    "press",
    "release",
    "dragBegin",
    "dragBy",
    "dragEnd",
    "focus",
    "blur",
    "key",
    "text",
    "composing",
    "hover",
    "leave",
    "wheel",
    "secondaryClick",
    "scroll",
    "textReplaced",
    "link",
    "longPress",
    "doubleTap",
    "zoom",
    "scrollPhase",
    "scrollOffset",
];
fn dispatch_kind(kind: i32) -> &'static str {
    usize::try_from(kind)
        .ok()
        .and_then(|k| KINDS.get(k))
        .copied()
        .unwrap_or("tap")
}

impl Host {
    pub(in crate::js::player) fn observes_frame(&self) -> bool {
        self.refs.borrow().hooks.contains_key("frame")
    }

    pub(in crate::js::player) fn observes_scroll(&self, id: u64) -> bool {
        let refs = self.refs.borrow();
        refs.callbacks.contains_key(&(id, 15)) || refs.callbacks.contains_key(&(id, 22))
    }

    pub(in crate::js::player) fn dispatch(
        &self,
        id: u64,
        kind: i32,
        a: f64,
        b: f64,
    ) -> Result<(), String> {
        if kind == 15 {
            self.dispatch(id, 22, a, b)?;
        }
        let _budget = self.budget(dispatch_kind(kind), id);
        let callback = self.refs.borrow().callbacks.get(&(id, kind)).cloned();
        let Some(callback) = callback else {
            return Ok(());
        };
        self.with(|ctx| {
            let args = match kind {
                4 | 5 | 13 | 15 | 22 => vec![number(&ctx, a), number(&ctx, b)],
                17 | 20 => vec![number(&ctx, a)],
                _ => vec![],
            };
            self.listener_result(&ctx, self.call(&ctx, &callback, args), "listener")
        })
    }

    pub(in crate::js::player) fn dispatch_input(
        &self,
        id: u64,
        kind: i32,
        text: Option<&str>,
        span: Option<(i32, i32, i32, i32)>,
    ) -> Result<(), String> {
        let _budget = self.budget(dispatch_kind(kind), id);
        let callback = self.refs.borrow().callbacks.get(&(id, kind)).cloned();
        let Some(callback) = callback else {
            return Ok(());
        };
        self.with(|ctx| {
            let value = text.map_or_else(|| Ok(Value::new_null(ctx.clone())), |s| s.into_js(&ctx));
            let result = value.and_then(|value| {
                let mut args = vec![value];
                if let Some((start, end, compose_start, compose_end)) = span {
                    args.extend([start, end, compose_start, compose_end].map(|v| number(&ctx, v)));
                }
                self.call(&ctx, &callback, args)
            });
            self.listener_result(&ctx, result, "input")
        })
    }

    fn hook_turn(
        &self,
        name: &'static str,
        values: impl for<'js> FnOnce(&Ctx<'js>) -> Vec<Value<'js>>,
    ) -> Result<(), String> {
        let _budget = self.budget(name, 0);
        let hook = self.refs.borrow().hooks.get(name).cloned();
        let Some(hook) = hook else {
            return Ok(());
        };
        self.with(|ctx| {
            self.finish(&ctx, name, self.call(&ctx, &hook, values(&ctx)))
                .map(|_| ())
        })
    }

    pub(in crate::js::player) fn frame(&self, target: f64, interval: f64) -> Result<(), String> {
        self.hook_turn("frame", |ctx| {
            vec![number(ctx, target), number(ctx, interval)]
        })
    }

    pub(in crate::js::player) fn layout(&self) -> Result<(), String> {
        self.hook_turn("layout", |_| vec![])
    }

    pub(in crate::js::player) fn scroll_threshold(
        &self,
        id: u32,
        active: bool,
    ) -> Result<(), String> {
        self.hook_turn("scrollThreshold", |ctx| {
            vec![number(ctx, id), Value::new_bool(ctx.clone(), active)]
        })
    }

    pub(in crate::js::player) fn scroll_state(
        &self,
        id: u64,
        phase: crate::motion::ScrollPhase,
        velocity: (f32, f32),
        offset: (f32, f32),
    ) -> Result<(), String> {
        self.hook_turn("scroll", |ctx| {
            vec![
                number(ctx, id as f64),
                number(ctx, phase.code()),
                number(ctx, velocity.0),
                number(ctx, velocity.1),
                number(ctx, offset.0),
                number(ctx, offset.1),
            ]
        })
    }

    pub(in crate::js::player) fn driver(
        &self,
        event: &crate::tree::list::DriverEvent,
    ) -> Result<(), String> {
        use crate::tree::list::{DriverEvent, ListOp};

        let _budget = self.budget("driver", 0);
        let Some(hook) = self.refs.borrow().hooks.get("driver").cloned() else {
            return Ok(());
        };
        let (kind, id, tag) = match event {
            DriverEvent::ListOp { id, .. } => (1, id, 1),
            DriverEvent::ListChrome { id, .. } => (1, id, 2),
            DriverEvent::EndReached { id } => (1, id, 3),
            DriverEvent::Refresh { id } => (2, id, 5),
        };
        let mut scalars = [0.0; 8];
        let (key, extra) = match event {
            DriverEvent::ListOp { op, .. } => match op {
                ListOp::Detach { key } => {
                    scalars[0] = 1.0;
                    (key.as_str(), "")
                }
                ListOp::Adopt { key, ty, index } => {
                    scalars[0] = 2.0;
                    scalars[1] = f64::from(i32::try_from(*index).unwrap_or(0));
                    (key.as_str(), ty.as_str())
                }
                ListOp::Set { key, index } => {
                    scalars[0] = 3.0;
                    scalars[1] = f64::from(i32::try_from(*index).unwrap_or(0));
                    (key.as_str(), "")
                }
                ListOp::Place { key, before } => {
                    scalars[0] = 4.0;
                    (key.as_str(), before.as_str())
                }
            },
            DriverEvent::ListChrome { chrome, .. } => {
                let flags = u8::from(chrome.thumb_on) | (u8::from(chrome.h_on) << 1);
                scalars = [
                    f64::from(flags),
                    f64::from(chrome.pin_index),
                    f64::from(chrome.thumb_len),
                    f64::from(chrome.thumb_off),
                    f64::from(chrome.h_len),
                    f64::from(chrome.h_off),
                    f64::from(chrome.pin_y),
                    f64::from(chrome.extent),
                ];
                (chrome.pin_key.as_str(), "")
            }
            DriverEvent::EndReached { .. } | DriverEvent::Refresh { .. } => ("", ""),
        };
        self.with(|ctx| {
            let result = (|| {
                let mut args = vec![
                    number(&ctx, kind),
                    number(&ctx, id.index),
                    number(&ctx, id.generation),
                    number(&ctx, tag),
                ];
                args.extend(scalars.map(|value| number(&ctx, value)));
                args.push(key.into_js(&ctx)?);
                args.push(extra.into_js(&ctx)?);
                self.call(&ctx, &hook, args)
            })();
            self.finish(&ctx, "driver", result).map(|_| ())
        })
    }

    pub(in crate::js::player) fn run_updates(&self) -> Result<(), String> {
        let _budget = self.budget("reactive", 0);
        self.reactive_failed.set(false);
        self.with(|_| self.inner().graph.run_updates(reactive::JsEval));
        if self.reactive_failed.get() {
            Err(self.record_fault("reactive", None))
        } else {
            Ok(())
        }
    }

    pub(in crate::js::player) fn drain_jobs(&self) -> Result<(), String> {
        self.drain().map_err(|_| self.record_fault("jobs", None))
    }

    pub(in crate::js::player) fn jobs_pending(&self) -> bool {
        self.runtime.jobs_pending()
    }

    pub(in crate::js::player) fn tap_count(&self) -> usize {
        self.refs
            .borrow()
            .callbacks
            .keys()
            .map(|(id, _)| *id)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    pub(in crate::js::player) fn forget_ids(&self, ids: &[u64]) {
        {
            let mut refs = self.refs.borrow_mut();
            for &id in ids {
                for &(_, kind) in bindings::GESTURES {
                    refs.callbacks.remove(&(id, kind));
                }
            }
            refs.motions.retain(|(_, id, _, _)| !ids.contains(id));
        }
        canvas::forget_nodes(self, ids);
    }

    pub(in crate::js::player) fn take_wants_frame(&self) -> bool {
        self.wants_frame.replace(false)
    }

    pub(in crate::js::player) fn take_back_enabled(&self) -> Option<bool> {
        self.back_enabled.take()
    }

    pub(in crate::js::player) fn back_gesture(
        &self,
        event: crate::session::BackGesture,
    ) -> Result<bool, String> {
        use crate::session::BackGesture;
        let hook = self.refs.borrow().hooks.get("backGesture").cloned();
        let Some(hook) = hook else {
            return if event == BackGesture::Committed {
                self.dispatch_back("backGesture")
            } else {
                Ok(true)
            };
        };
        let _budget = self.budget("backGesture", 0);
        self.with(|ctx| {
            let (phase, value) = match event {
                BackGesture::Started { from_right } => ("start", f64::from(u8::from(from_right))),
                BackGesture::Progress(value) => ("progress", f64::from(value)),
                BackGesture::Cancelled => ("cancel", 0.0),
                BackGesture::Committed => ("commit", 1.0),
            };
            let result = phase
                .into_js(&ctx)
                .and_then(|phase| self.call(&ctx, &hook, vec![phase, number(&ctx, value)]));
            self.finish(&ctx, "backGesture", result)
                .map(|value| event != BackGesture::Committed || value.as_bool() != Some(false))
        })
    }

    pub(in crate::js::player) fn dispatch_back(
        &self,
        source: &'static str,
    ) -> Result<bool, String> {
        let _budget = self.budget("back", 0);
        let callbacks = self.refs.borrow().backs.clone();
        self.with(|ctx| {
            for (_, callback) in callbacks.iter().rev() {
                let result = self
                    .call(&ctx, callback, vec![])
                    .and_then(|v| rquickjs::Coerced::<bool>::from_js(&ctx, v));
                if self.finish(&ctx, source, result)?.0 {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    pub(in crate::js::player) fn try_shortcut(&self, spec: &str) -> Result<bool, String> {
        let _budget = self.budget("shortcut", 0);
        let callback = self
            .refs
            .borrow()
            .shortcuts
            .iter()
            .find(|(s, _)| *s == spec)
            .map(|(_, f)| f.clone());
        let Some(callback) = callback else {
            return Ok(false);
        };
        self.with(|ctx| {
            self.listener_result(&ctx, self.call(&ctx, &callback, vec![]), "listener")?;
            Ok(true)
        })
    }

    pub(in crate::js::player) fn apply_settings(&self) -> Result<(), String> {
        let _budget = self.budget("settings", 0);
        let callbacks = self.refs.borrow().settings.clone();
        self.with(|ctx| {
            let result = bindings::settings(self, &ctx).and_then(|settings| {
                for (_, callback) in callbacks {
                    self.call(&ctx, &callback, vec![settings.clone()])?;
                }
                Ok(())
            });
            self.finish(&ctx, "input", result)
        })
    }

    pub(in crate::js::player) fn apply_window_metrics(
        &self,
        source: &'static str,
    ) -> Result<(), String> {
        let callbacks = self.refs.borrow().window_metrics.clone();
        if callbacks.is_empty() {
            return Ok(());
        }
        let _budget = self.budget("window metrics", 0);
        self.with(|ctx| {
            let result = bindings::window_metrics(self, &ctx).and_then(|metrics| {
                for (_, callback) in callbacks {
                    self.call(&ctx, &callback, vec![metrics.clone()])?;
                }
                Ok(())
            });
            self.finish(&ctx, source, result)
        })
    }

    pub(in crate::js::player) fn motion_end(&self, node: u64, prop: &str) -> Result<(), String> {
        let _budget = self.budget("motion", node);
        let callbacks: Vec<_> = self
            .refs
            .borrow()
            .motions
            .iter()
            .filter(|(_, n, p, _)| *n == node && *p == prop)
            .map(|(_, _, _, f)| f.clone())
            .collect();
        self.with(|ctx| {
            let mut result = Ok(());
            for callback in callbacks {
                if let Err(error) = self.call(&ctx, &callback, vec![]) {
                    self.keep_first_error(&ctx, "motion", error, &mut result);
                }
            }
            result
        })
    }

    pub(in crate::js::player) fn set_clock(&self, mono: i64, paused: bool) {
        self.runtime.set_clock(mono);
        self.runtime.begin_timer_turn();
        self.paused.set(paused);
    }

    fn keep_first_error(
        &self,
        ctx: &Ctx<'_>,
        source: &'static str,
        error: rquickjs::Error,
        result: &mut Result<(), String>,
    ) {
        if result.is_ok() {
            *result = Err(self.stash_fault(ctx, source, error));
        } else if ctx.has_exception() {
            drop(ctx.catch());
        }
    }

    pub(in crate::js::player) fn fire_timers(&self) -> Result<(), String> {
        if self.paused.get() {
            return Ok(());
        }
        let mut result = Ok(());
        let due = self.runtime.run_due_timers(|ctx, error| {
            self.keep_first_error(ctx, "timer", error, &mut result);
            true
        });
        if let Err(error) = due
            && result.is_ok()
        {
            self.runtime
                .extension()
                .record_error(error.message, String::new());
            result = Err(self.record_fault("timer", None));
        }
        if self.runtime.due_timer_next_turn() {
            self.wants_frame.set(true);
        }
        result
    }

    pub(in crate::js::player) fn fire_raf(&self) -> Result<(), String> {
        if self.paused.get() {
            return Ok(());
        }
        let _budget = self.budget("raf", 0);
        let mut output = Ok(());
        let turn = self.runtime.timer_turn();
        loop {
            let callback = javascript::runtime::extension::poll_timer(
                &mut self.raf.borrow_mut(),
                true,
                turn,
                0,
            );
            let Some((function, arguments)) = callback else {
                break;
            };
            self.with(|ctx| {
                let result = arguments
                    .into_iter()
                    .map(|value| value.restore(&ctx))
                    .collect::<rquickjs::Result<Vec<_>>>()
                    .and_then(|values| self.call(&ctx, &function, values));
                if let Err(error) = result {
                    self.keep_first_error(&ctx, "timer", error, &mut output);
                }
            });
        }
        self.raf.borrow_mut().retain(|timer| timer.active);
        output
    }

    pub(in crate::js::player) fn needs_frame(&self) -> bool {
        !self.paused.get()
            && (self.raf.borrow().iter().any(|timer| timer.active)
                || self.runtime.due_timer_next_turn())
    }

    pub(in crate::js::player) fn next_deadline(&self) -> Option<i64> {
        if self.paused.get() {
            return None;
        }
        self.runtime
            .next_deadline_ms()
            .filter(|deadline| *deadline > self.runtime.clock_ms())
    }
}
