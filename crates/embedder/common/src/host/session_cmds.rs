use engine::session::{Metrics, PlatformSettings};

use super::{UiHost, arm, flush_gpu, note_wake, session_call};
use crate::command::Command;
use crate::error::BootError;

pub(super) fn handle(runtime: &mut UiHost, command: Command) -> Result<bool, BootError> {
    match command {
        command @ (Command::Pointer(_)
        | Command::Input(_)
        | Command::ImeDismissed
        | Command::Metrics(_)) => input(runtime, command),
        command @ (Command::Lifecycle(_)
        | Command::BackGesture(_)
        | Command::WindowDisplay(_)
        | Command::WindowStateChanged(_)
        | Command::Settings(_)
        | Command::WindowFocus(_)
        | Command::SemanticsEnabled(_)
        | Command::AutofillEnabled(_)
        | Command::SemanticsAction(_)
        | Command::LaunchUrl(_)) => session(runtime, command),
        Command::JsIo(delivery) => {
            io_done(runtime, |session| session.io_delivery(delivery));
            Ok(false)
        }
        Command::Io(event) => {
            io_done(runtime, |session| session.io_event(event));
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn input(runtime: &mut UiHost, command: Command) -> Result<bool, BootError> {
    match command {
        Command::Pointer(event) => {
            if runtime.input.push_pointer(event) {
                arm(runtime);
            }
        }
        Command::Input(event) => {
            if runtime.input.push_input(event) {
                arm(runtime);
            }
        }
        Command::ImeDismissed => {
            if runtime.input.dismiss_ime() {
                arm(runtime);
            }
        }
        Command::Metrics(metrics) => metrics_cmd(runtime, metrics),
        _ => {}
    }
    Ok(false)
}

fn session(runtime: &mut UiHost, command: Command) -> Result<bool, BootError> {
    match command {
        Command::Lifecycle(state) => lifecycle(runtime, state),
        Command::BackGesture(event) => {
            if let Some(session) = runtime.session.as_mut() {
                let mut handled = true;
                session_call(|| handled = session.back_gesture(event));
                if !handled {
                    runtime.services.borrow().unhandled_back();
                }
                arm(runtime);
            }
        }
        Command::WindowDisplay(display) => {
            if let Some(session) = runtime.session.as_mut() {
                session_call(|| session.window_display(display));
                arm(runtime);
            }
        }
        Command::Settings(settings) => settings_cmd(runtime, &settings),
        Command::WindowStateChanged(key) => {
            io_done(runtime, |session| session.window_state_changed(key));
        }
        Command::WindowFocus(focused) => {
            if let Some(session) = runtime.session.as_mut() {
                session_call(|| session.window_focus(focused));
            }
        }
        Command::SemanticsEnabled(enabled) => {
            if std::mem::replace(&mut runtime.semantics_enabled, enabled) != enabled {
                if let Some(session) = runtime.session.as_mut() {
                    session_call(|| session.set_semantics_enabled(enabled));
                }
                renew_semantics(runtime);
            }
        }
        Command::AutofillEnabled(enabled) => {
            if std::mem::replace(&mut runtime.autofill_enabled, enabled) != enabled {
                if let Some(session) = runtime.session.as_mut() {
                    session_call(|| session.set_autofill_enabled(enabled));
                }
                renew_semantics(runtime);
            }
        }
        Command::SemanticsAction(action) => {
            if let Some(session) = runtime.session.as_mut() {
                session_call(|| session.semantics_action(action));
                arm(runtime);
            }
        }
        Command::LaunchUrl(url) => launch_url(runtime, url),
        _ => {}
    }
    Ok(false)
}

fn renew_semantics(runtime: &mut UiHost) {
    if runtime.semantics.take().is_some() {
        runtime.services.borrow().semantics(&[]);
    }
    if runtime.semantics_enabled || runtime.autofill_enabled {
        arm(runtime);
    }
}

fn metrics_cmd(runtime: &mut UiHost, metrics: Metrics) {
    super::note_scale(runtime, metrics.device_pixel_ratio);
    if let Some(hub) = runtime.hub.as_ref() {
        hub.note_metrics(metrics);
    }
    if runtime.input.set_metrics(metrics) {
        arm(runtime);
    }
}

fn lifecycle(runtime: &mut UiHost, state: engine::session::Lifecycle) {
    let deadline = runtime.session.as_mut().map(|session| {
        session_call(|| session.lifecycle(state));
        session.next_wake_ms().map(crate::net::ms_i64)
    });
    if let Some(deadline) = deadline {
        note_wake(runtime, deadline);
    }
    let arm_frame = match (state, runtime.input.is_paused()) {
        (engine::session::Lifecycle::Hidden | engine::session::Lifecycle::Paused, _) => {
            flush_gpu(runtime, "trim");
            runtime.input.pause()
        }
        (engine::session::Lifecycle::Resumed, _) | (engine::session::Lifecycle::Inactive, true) => {
            runtime.revision = runtime.revision.wrapping_add(1);
            runtime.update_raster(
                |graphics| {
                    graphics.needs_present = true;
                    graphics.present_failures = 0;
                    graphics.frame_retry_at = None;
                    graphics.surfaces.rearm_pending()
                },
                |ui, rearmed| {
                    if rearmed {
                        ui.try_surface = true;
                        if !ui.input.is_paused() {
                            arm(ui);
                        }
                    }
                },
            );
            runtime.present_retry_at = None;
            // A stuck armed token cannot be replaced by `arm`. Drop it so resume
            // can post a fresh vsync even if the surface never went away.
            runtime.scheduler.cancel();
            runtime.vsync.cancel();
            runtime.input.resume()
        }
        (engine::session::Lifecycle::Inactive, false)
        | (engine::session::Lifecycle::Detached, _) => false,
    };
    if arm_frame {
        arm(runtime);
    }
}

fn settings_cmd(runtime: &mut UiHost, settings: &PlatformSettings) {
    if let Some(session) = runtime.session.as_mut() {
        session_call(|| session.settings(settings));
        if session.wants_frame() {
            arm(runtime);
        }
    }
}

fn launch_url(runtime: &mut UiHost, url: String) {
    if url.is_empty() {
        return;
    }
    if let Some(session) = runtime.session.as_mut() {
        session_call(|| session.set_launch_url(&url));
        // A navigation listener can queue reactive work that only flushes
        // on the next frame, before the session reports any frame demand.
        arm(runtime);
    }
}

/// Session-side IO completion: deliver the event, arm a frame if the session asks.
fn io_done(runtime: &mut UiHost, dispatch: impl FnOnce(&mut dyn engine::session::Session)) {
    if let Some(session) = runtime.session.as_mut() {
        session_call(|| dispatch(session.as_mut()));
        let wants_frame = session.wants_frame();
        let deadline = session.next_wake_ms().map(crate::net::ms_i64);
        note_wake(
            runtime,
            deadline.into_iter().chain(runtime.present_retry_at).min(),
        );
        if wants_frame {
            arm(runtime);
        }
    }
}
