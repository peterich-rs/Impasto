use engine::session::{Metrics, PlatformSettings};

use super::{UiHost, arm, flush_gpu, refresh_wake, session_call};
use crate::command::Command;
use crate::error::BootError;

pub(super) fn handle(runtime: &mut UiHost, command: Command) -> Result<bool, BootError> {
    match command {
        #[cfg(any(debug_assertions, feature = "devtools-server"))]
        Command::Devtools { query, reply } => {
            if !reply.is_closed()
                || matches!(query, crate::command::DevtoolsQuery::ClearOverlay { .. })
            {
                let _ = reply.send(devtools(runtime, query));
            }
            Ok(false)
        }
        command @ (Command::Pointer(_)
        | Command::PointerMoves { .. }
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
        Command::PointerMoves { event, history } => {
            if runtime.input.push_pointer_moves(event, history) {
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
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            if let Some(hub) = &runtime.telemetry {
                hub.note_refresh_rate(display.refresh_rate);
            }
            if let Some(session) = runtime.session.as_mut() {
                session_call(|| session.window_display(display));
                arm(runtime);
            }
        }
        Command::Settings(settings) => settings_cmd(runtime, &settings),
        Command::WindowStateChanged(key) => {
            if let Some(plugins) = &runtime.plugins {
                plugins.window_state_changed(&key);
            }
            io_done(runtime, |session| session.window_state_changed(key));
        }
        Command::WindowFocus(focused) => {
            if let Some(plugins) = &runtime.plugins {
                plugins.window_focus(focused);
            }
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
    if let Some(plugins) = &runtime.plugins {
        plugins.lifecycle(state);
    }
    if let Some(session) = runtime.session.as_mut() {
        session_call(|| session.lifecycle(state));
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
            runtime.cancel_vsync();
            runtime.input.resume()
        }
        (engine::session::Lifecycle::Inactive, false)
        | (engine::session::Lifecycle::Detached, _) => false,
    };
    if arm_frame {
        arm(runtime);
    }
    refresh_wake(runtime);
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
    if let Some(plugins) = &runtime.plugins {
        plugins.open_url(&url);
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
        refresh_wake(runtime);
        if wants_frame {
            arm(runtime);
        }
    }
}

#[cfg(any(debug_assertions, feature = "devtools-server"))]
fn devtools(runtime: &mut UiHost, query: crate::command::DevtoolsQuery) -> Result<Vec<u8>, String> {
    use crate::command::DevtoolsQuery;

    let session = runtime.session.as_mut().ok_or("session closed")?;
    let mut value = None;
    match query {
        DevtoolsQuery::ViewSnapshot { max_nodes } => {
            session_call(|| value = session.inspector_snapshot_limited(max_nodes))
        }
        DevtoolsQuery::ViewProbe { x, y } => session_call(|| value = session.inspector_probe(x, y)),
        DevtoolsQuery::ViewSetOverlay { owner, config } => {
            let highlight = config
                .highlight_node
                .as_deref()
                .map(str::parse::<u64>)
                .transpose()
                .map_err(|_| "invalid highlight node")?
                .map(engine::tree::NodeId::from_raw);
            session_call(|| {
                session.set_remote_overlay(engine::tree::RemoteOverlayConfig {
                    highlight,
                    pick_mode: config.pick_mode,
                    repaint_rainbow: config.repaint_rainbow,
                    layout_scan: config.layout_scan,
                })
            });
            runtime.overlay_owner = Some(owner);
            arm(runtime);
            return Ok(b"true".to_vec());
        }
        DevtoolsQuery::ClearOverlay { owner } => {
            if runtime.overlay_owner == Some(owner) {
                runtime.overlay_owner = None;
                session_call(|| {
                    session.set_remote_overlay(engine::tree::RemoteOverlayConfig::default())
                });
                arm(runtime);
            }
            return Ok(b"true".to_vec());
        }
        DevtoolsQuery::ViewRepaintLeaderboard { limit } => {
            session_call(|| {
                let records = session.repaint_leaderboard(limit);
                value = Some(crate::diagnostics::remote_views::leaderboard(&records).to_string());
            });
        }
    };
    value
        .map(String::into_bytes)
        .ok_or_else(|| "query unsupported by this session".to_owned())
}
