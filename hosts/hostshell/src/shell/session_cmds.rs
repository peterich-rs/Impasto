use session::{IoEvent, Metrics, PlatformSettings};

use super::{Runtime, arm, note_wake};
use crate::command::Command;
use crate::error::BootError;

pub(super) fn handle(runtime: &mut Runtime, command: Command) -> Result<bool, BootError> {
    match command {
        command @ (Command::Pointer(_)
        | Command::Input(_)
        | Command::ImeDismissed
        | Command::Metrics(_)) => input(runtime, command),
        command @ (Command::Lifecycle(_)
        | Command::Settings(_)
        | Command::WindowFocus(_)
        | Command::SemanticsAction(_)
        | Command::LaunchUrl(_)) => session(runtime, command),
        command @ (Command::HttpDone { .. }
        | Command::HttpHead { .. }
        | Command::HttpChunk { .. }
        | Command::PluginDone { .. }
        | Command::Socket { .. }) => io(runtime, command),
        _ => Ok(false),
    }
}

fn input(runtime: &mut Runtime, command: Command) -> Result<bool, BootError> {
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

fn session(runtime: &mut Runtime, command: Command) -> Result<bool, BootError> {
    match command {
        Command::Lifecycle(state) => lifecycle(runtime, state),
        Command::Settings(settings) => settings_cmd(runtime, &settings),
        Command::WindowFocus(focused) => {
            if let Some(session) = runtime.session.as_mut() {
                session.window_focus(focused);
            }
        }
        Command::SemanticsAction(action) => {
            if let Some(session) = runtime.session.as_mut() {
                session.semantics_action(action);
                arm(runtime);
            }
        }
        Command::LaunchUrl(url) => launch_url(runtime, url),
        _ => {}
    }
    Ok(false)
}

fn io(runtime: &mut Runtime, command: Command) -> Result<bool, BootError> {
    let event = match command {
        Command::HttpDone { token, result } => IoEvent::Http { token, result },
        Command::HttpHead {
            token,
            status,
            headers,
        } => IoEvent::HttpHead {
            token,
            status,
            headers,
        },
        Command::HttpChunk {
            token,
            bytes,
            done,
            error,
        } => IoEvent::HttpChunk {
            token,
            bytes,
            done,
            error,
        },
        Command::PluginDone { token, result } => IoEvent::Plugin { token, result },
        Command::Socket {
            id,
            event,
            text,
            data,
        } => IoEvent::Socket {
            id,
            event,
            text,
            data,
        },
        _ => return Ok(false),
    };
    io_done(runtime, event);
    Ok(false)
}

fn metrics_cmd(runtime: &mut Runtime, metrics: Metrics) {
    super::note_scale(runtime, metrics.device_pixel_ratio);
    if runtime.input.set_metrics(metrics) {
        arm(runtime);
    }
}

fn lifecycle(runtime: &mut Runtime, state: session::Lifecycle) {
    let deadline = runtime.session.as_mut().map(|session| {
        session.lifecycle(state);
        session.next_wake_ms().map(|ms| ms as i64)
    });
    if let Some(deadline) = deadline {
        note_wake(runtime, deadline);
    }
    let arm_frame = match state {
        session::Lifecycle::Hidden | session::Lifecycle::Paused => runtime.input.pause(),
        session::Lifecycle::Resumed => {
            runtime.needs_present = true;
            runtime.present_failures = 0;
            runtime.present_logged = false;
            runtime.present_retry_at = None;
            runtime.surfaces.rearm_pending();
            runtime.input.resume()
        }
        session::Lifecycle::Inactive | session::Lifecycle::Detached => false,
    };
    if arm_frame {
        arm(runtime);
    }
}

fn settings_cmd(runtime: &mut Runtime, settings: &PlatformSettings) {
    if let Some(session) = runtime.session.as_mut() {
        session.settings(settings);
        if session.wants_frame() {
            arm(runtime);
        }
    }
}

fn launch_url(runtime: &mut Runtime, url: String) {
    if url.is_empty() {
        return;
    }
    if let Some(session) = runtime.session.as_mut() {
        session.set_launch_url(&url);
        if session.wants_frame() {
            arm(runtime);
        }
    }
}

/// Session-side IO completion: deliver the event, arm a frame if the session asks.
fn io_done(runtime: &mut Runtime, event: IoEvent) {
    if let Some(session) = runtime.session.as_mut() {
        session.io_event(event);
        if session.wants_frame() {
            arm(runtime);
        }
    }
}
