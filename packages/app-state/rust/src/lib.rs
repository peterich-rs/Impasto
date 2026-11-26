//! Application lifecycle and state shared by platform windows.

use impasto::{
    ImpastoEvents, ImpastoType, Lifecycle, PluginContext, PluginInitError, PluginRegistrar,
    Subscription,
};

#[derive(ImpastoType)]
enum Focus {
    True,
    False,
}

#[derive(ImpastoEvents)]
enum AppEvents {
    Change(String),
    Focus(Focus),
    WindowState(String),
}

#[impasto::module(
    name = "AppState",
    specifier = "@impasto/app-state/native",
    init = init,
    events = AppEvents
)]
struct AppStatePlugin {
    context: PluginContext,
    _subscriptions: [Subscription; 3],
}

fn state_name(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Resumed => "active",
        Lifecycle::Inactive => "inactive",
        Lifecycle::Hidden | Lifecycle::Paused | Lifecycle::Detached => "background",
    }
}

impl AppStatePlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        let lifecycle = ctx.on_lifecycle({
            let events = ctx.emitter::<AppEvents>();
            move |lifecycle| {
                let _ = events.emit(AppEvents::Change(state_name(lifecycle).into()));
            }
        });
        let focus = ctx.on_window_focus({
            let events = ctx.emitter::<AppEvents>();
            move |focused| {
                let focus = if focused { Focus::True } else { Focus::False };
                let _ = events.emit(AppEvents::Focus(focus));
            }
        });
        let window = ctx.on_window_state({
            let events = ctx.emitter::<AppEvents>();
            move |key| {
                let _ = events.emit(AppEvents::WindowState(key.into()));
            }
        });
        Ok(Self {
            context: ctx.clone(),
            _subscriptions: [lifecycle, focus, window],
        })
    }
}

#[derive(Debug)]
struct StateError(String);

impl impasto::ImpastoError for StateError {
    fn scope(&self) -> &'static str {
        "app-state"
    }
    fn code(&self) -> &'static str {
        "state"
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[impasto::export]
impl AppStatePlugin {
    pub fn get_current_state(&self) -> String {
        state_name(self.context.lifecycle()).into()
    }

    pub fn get_restoration_state(&self, key: &str) -> Option<String> {
        self.context.restoration()?.get(key)
    }

    pub fn set_restoration_state(
        &self,
        key: &str,
        value: Option<String>,
    ) -> Result<(), StateError> {
        self.context.restoration().map_or(Ok(()), |state| {
            state.set(key, value.as_deref()).map_err(StateError)
        })
    }

    pub fn get_window_state(&self, key: &str) -> Option<String> {
        self.context.window_session()?.get(key)
    }

    pub fn set_window_state(&self, key: &str, value: Option<String>) -> Result<(), StateError> {
        self.context.window_session().map_or(Ok(()), |state| {
            state.set(key, value.as_deref()).map_err(StateError)
        })
    }
}

/// Registers application-state bindings in the current application.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(AppStatePlugin::plugin());
}
