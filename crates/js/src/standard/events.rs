use std::cell::Cell;
use std::rc::Rc;

use rquickjs::class::{ClassKind, JsCell, JsClass, Trace, Writable};
use rquickjs::function::{IntoJsFunc, Opt, ParamRequirement, Params, This};
use rquickjs::object::{Accessor, Property};
use rquickjs::{
    Atom, Class, Coerced, Constructor, Ctx, Exception, Function, JsLifetime, Object, Result, Value,
};

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "Event")]
struct Event<'js> {
    kind: Atom<'js>,
    bubbles: bool,
    cancelable: bool,
    target: Option<Object<'js>>,
    dispatch: Dispatch,
    prevented: bool,
}

#[derive(Default, Trace, JsLifetime)]
struct Dispatch {
    active: bool,
    stopped: bool,
    immediate: bool,
    passive: bool,
}

impl<'js> Event<'js> {
    fn new(kind: Atom<'js>, bubbles: bool, cancelable: bool) -> Self {
        Self {
            kind,
            bubbles,
            cancelable,
            target: None,
            dispatch: Dispatch::default(),
            prevented: false,
        }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Event<'js> {
    #[qjs(get, rename = "type", configurable)]
    fn kind(&self) -> Result<Value<'js>> {
        self.kind.to_value()
    }

    #[qjs(get, configurable)]
    fn bubbles(&self) -> bool {
        self.bubbles
    }

    #[qjs(get, configurable)]
    fn cancelable(&self) -> bool {
        self.cancelable
    }

    #[qjs(get, configurable)]
    fn target(&self, ctx: Ctx<'js>) -> Value<'js> {
        self.target
            .clone()
            .map_or_else(|| Value::new_null(ctx), Object::into_value)
    }

    #[qjs(get, configurable)]
    fn current_target(&self, ctx: Ctx<'js>) -> Value<'js> {
        self.dispatch
            .active
            .then(|| self.target.clone())
            .flatten()
            .map_or_else(|| Value::new_null(ctx), Object::into_value)
    }

    #[qjs(get, configurable)]
    fn event_phase(&self) -> i32 {
        if self.dispatch.active { 2 } else { 0 }
    }

    #[qjs(get, configurable)]
    fn default_prevented(&self) -> bool {
        self.prevented
    }

    #[qjs(get, configurable)]
    fn cancel_bubble(&self) -> bool {
        self.dispatch.stopped
    }

    #[qjs(set, rename = "cancelBubble", configurable)]
    fn set_cancel_bubble(&mut self, Coerced(value): Coerced<bool>) {
        if value {
            self.stop_propagation();
        }
    }

    fn prevent_default(&mut self) {
        if self.cancelable && !self.dispatch.passive {
            self.prevented = true;
        }
    }

    fn stop_propagation(&mut self) {
        self.dispatch.stopped = true;
    }

    fn stop_immediate_propagation(&mut self) {
        self.dispatch.stopped = true;
        self.dispatch.immediate = true;
    }
}

#[derive(Default, Trace, JsLifetime)]
#[rquickjs::class(rename = "EventTarget")]
pub(super) struct Target<'js> {
    next_listener: u64,
    listeners: Vec<Listener<'js>>,
    signal: Option<AbortState<'js>>,
    attachment: Option<Object<'js>>,
}

#[derive(Clone, Trace, JsLifetime)]
struct Listener<'js> {
    id: u64,
    kind: Atom<'js>,
    callback: Value<'js>,
    capture: bool,
    once: bool,
    passive: bool,
    handler: bool,
    #[qjs(skip_trace)]
    removed: Rc<Cell<bool>>,
    abort: Option<Registration<'js>>,
}

#[derive(Default, Trace, JsLifetime)]
struct AbortState<'js> {
    reason: Option<Value<'js>>,
    next_action: u64,
    actions: Vec<Action<'js>>,
}

#[derive(Trace, JsLifetime)]
struct Action<'js> {
    id: u64,
    operation: AbortAction<'js>,
}

#[derive(Trace, JsLifetime)]
enum AbortAction<'js> {
    Callback(Function<'js>),
    RemoveListener { target: Object<'js>, listener: u64 },
}

#[derive(Clone, Trace, JsLifetime)]
struct Registration<'js> {
    signal: Object<'js>,
    id: u64,
}

impl Registration<'_> {
    fn remove(&self) -> Result<()> {
        if let Some(signal) = Class::<Target>::from_object(&self.signal)
            && let Some(state) = signal.try_borrow_mut()?.signal.as_mut()
        {
            state.actions.retain(|item| item.id != self.id);
        }
        Ok(())
    }
}

#[derive(Trace, JsLifetime)]
struct DetachAbort<'js> {
    registration: Option<Registration<'js>>,
}

impl<'js> JsClass<'js> for DetachAbort<'js> {
    const NAME: &'static str = "DetachAbort";
    const KIND: ClassKind = ClassKind::Callable;
    type Mutable = Writable;

    fn prototype(ctx: &Ctx<'js>) -> Result<Option<Object<'js>>> {
        Ok(Some(Function::prototype(ctx.clone())))
    }

    fn constructor(_ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        Ok(None)
    }

    fn call(this: &JsCell<'js, Self>, params: Params<'_, 'js>) -> Result<Value<'js>> {
        let registration = this
            .try_borrow_mut()
            .map_err(rquickjs::Error::ClassBorrow)?
            .registration
            .take();
        if let Some(registration) = registration {
            registration.remove()?;
        }
        Ok(Value::new_undefined(params.ctx().clone()))
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "AbortSignal")]
struct SignalPrototype {}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "AbortController")]
struct Controller<'js> {
    signal: Object<'js>,
}

#[rquickjs::methods]
impl<'js> Controller<'js> {
    #[qjs(get, configurable)]
    fn signal(&self) -> Object<'js> {
        self.signal.clone()
    }

    fn abort(This(this): This<Class<'js, Self>>, reason: Opt<Value<'js>>) -> Result<()> {
        let signal = this.try_borrow()?.signal.clone();
        abort_signal(&signal, reason.0)
    }
}

fn capture_option(options: Option<&Value<'_>>) -> Result<bool> {
    match options {
        Some(value) if value.is_bool() => Ok(value.as_bool().unwrap_or_default()),
        Some(value) => flag(value.as_object(), "capture"),
        None => Ok(false),
    }
}

fn flag(options: Option<&Object<'_>>, name: &str) -> Result<bool> {
    options.map_or(Ok(false), |object| {
        object.get::<_, Coerced<bool>>(name).map(|value| value.0)
    })
}

impl<'js> Target<'js> {
    fn insert(&mut self, mut listener: Listener<'js>) -> Result<u64> {
        self.next_listener = self
            .next_listener
            .checked_add(1)
            .ok_or(rquickjs::Error::Allocation)?;
        listener.id = self.next_listener;
        self.listeners.push(listener);
        Ok(self.next_listener)
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl<'js> Target<'js> {
    fn add_event_listener(
        This(this): This<Class<'js, Self>>,
        kind: Value<'js>,
        callback: Value<'js>,
        options: Opt<Value<'js>>,
    ) -> Result<()> {
        if !callback.is_object() {
            return Ok(());
        }
        let ctx = this.ctx();
        let kind = event_kind(ctx, kind)?;
        let capture = capture_option(options.0.as_ref())?;
        let options = options.0.as_ref().and_then(Value::as_object);
        let signal = options
            .map(|options| options.get("signal"))
            .transpose()?
            .map(optional_signal)
            .transpose()?
            .flatten();
        if let Some(signal) = &signal
            && abort_reason(signal)?.is_some()
        {
            return Ok(());
        }
        if this.try_borrow()?.listeners.iter().any(|entry| {
            !entry.handler
                && entry.kind == kind
                && entry.callback == callback
                && entry.capture == capture
        }) {
            return Ok(());
        }
        let listener = Listener {
            id: 0,
            kind,
            callback,
            capture,
            once: flag(options, "once")?,
            passive: flag(options, "passive")?,
            handler: false,
            removed: Rc::default(),
            abort: None,
        };
        let id = this.try_borrow_mut()?.insert(listener)?;
        if let Some(signal) = signal {
            let Some(registration) = register(
                &signal,
                AbortAction::RemoveListener {
                    target: this.as_inner().clone(),
                    listener: id,
                },
            )?
            else {
                return remove_listener(&this, id);
            };
            if let Some(listener) = this
                .try_borrow_mut()?
                .listeners
                .iter_mut()
                .find(|listener| listener.id == id)
            {
                listener.abort = Some(registration);
            }
        }
        Ok(())
    }

    fn remove_event_listener(
        This(this): This<Class<'js, Self>>,
        kind: Value<'js>,
        callback: Value<'js>,
        options: Opt<Value<'js>>,
    ) -> Result<()> {
        let kind = event_kind(this.ctx(), kind)?;
        let capture = capture_option(options.0.as_ref())?;
        let id = this
            .try_borrow()?
            .listeners
            .iter()
            .find(|entry| {
                !entry.handler
                    && entry.kind == kind
                    && entry.callback == callback
                    && entry.capture == capture
            })
            .map(|entry| entry.id);
        if let Some(id) = id {
            remove_listener(&this, id)?;
        }
        Ok(())
    }

    fn dispatch_event(This(this): This<Class<'js, Self>>, event: Value<'js>) -> Result<bool> {
        let event = event
            .as_object()
            .ok_or_else(|| Exception::throw_type(this.ctx(), "Expected an Event"))?;
        dispatch(this.as_inner(), event)
    }
}

fn remove_listener<'js>(target: &Class<'js, Target<'js>>, id: u64) -> Result<()> {
    let removed = {
        let mut state = target.try_borrow_mut()?;
        state
            .listeners
            .iter()
            .position(|listener| listener.id == id)
            .map(|index| state.listeners.remove(index))
    };
    if let Some(removed) = removed {
        removed.removed.set(true);
        if let Some(registration) = removed.abort {
            registration.remove()?;
        }
    }
    Ok(())
}

fn signal_target<'js>(signal: &Object<'js>) -> Result<Class<'js, Target<'js>>> {
    Class::<Target>::from_object(signal)
        .ok_or_else(|| Exception::throw_type(signal.ctx(), "Expected an AbortSignal"))
}

fn optional_signal<'js>(value: Value<'js>) -> Result<Option<Object<'js>>> {
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let signal = value
        .as_object()
        .ok_or_else(|| Exception::throw_type(value.ctx(), "Expected an AbortSignal"))?;
    abort_reason(signal)?;
    Ok(Some(signal.clone()))
}

pub(super) fn abort_reason<'js>(signal: &Object<'js>) -> Result<Option<Value<'js>>> {
    signal_target(signal)?
        .try_borrow()?
        .signal
        .as_ref()
        .map(|state| state.reason.clone())
        .ok_or_else(|| Exception::throw_type(signal.ctx(), "Expected an AbortSignal"))
}

fn register<'js>(
    signal: &Object<'js>,
    operation: AbortAction<'js>,
) -> Result<Option<Registration<'js>>> {
    let target = signal_target(signal)?;
    let mut target = target.try_borrow_mut()?;
    let state = target
        .signal
        .as_mut()
        .ok_or_else(|| Exception::throw_type(signal.ctx(), "Expected an AbortSignal"))?;
    if state.reason.is_some() {
        return Ok(None);
    }
    let id = state
        .next_action
        .checked_add(1)
        .ok_or(rquickjs::Error::Allocation)?;
    state.next_action = id;
    state.actions.push(Action { id, operation });
    Ok(Some(Registration {
        signal: signal.clone(),
        id,
    }))
}

pub(super) fn on_abort<'js>(
    signal: &Object<'js>,
    callback: Function<'js>,
) -> Result<Function<'js>> {
    let detach = Class::instance(signal.ctx().clone(), DetachAbort { registration: None })?;
    let registration = if let Some(reason) = abort_reason(signal)? {
        callback.call::<_, ()>((reason,))?;
        None
    } else {
        register(signal, AbortAction::Callback(callback))?
    };
    detach.try_borrow_mut()?.registration = registration;
    detach
        .into_value()
        .into_function()
        .ok_or(rquickjs::Error::Unknown)
}

fn abort_signal<'js>(signal: &Object<'js>, reason: Option<Value<'js>>) -> Result<()> {
    if abort_reason(signal)?.is_some() {
        return Ok(());
    }
    let ctx = signal.ctx();
    let reason = match reason.filter(|value| !value.is_undefined()) {
        Some(reason) => reason,
        None => named_error(ctx, "AbortError", "AbortError")?,
    };
    let actions = {
        let target = signal_target(signal)?;
        let mut target = target.try_borrow_mut()?;
        let state = target
            .signal
            .as_mut()
            .ok_or_else(|| Exception::throw_type(ctx, "Expected an AbortSignal"))?;
        state.reason = Some(reason.clone());
        std::mem::take(&mut state.actions)
    };
    for action in actions {
        match action.operation {
            AbortAction::Callback(callback) => {
                if let Err(error) = callback.call::<_, ()>((reason.clone(),)) {
                    report_listener_error(ctx, error)?;
                }
            }
            AbortAction::RemoveListener { target, listener } => {
                if let Some(target) = Class::<Target>::from_object(&target) {
                    remove_listener(&target, listener)?;
                }
            }
        }
    }
    dispatch(signal, &event(ctx, "abort")?)?;
    Ok(())
}

pub(super) fn dispatch<'js>(this: &Object<'js>, event: &Object<'js>) -> Result<bool> {
    let ctx = this.ctx();
    let target = Class::<Target>::from_object(this)
        .ok_or_else(|| Exception::throw_type(ctx, "Expected an EventTarget"))?;
    let event = Class::<Event>::from_object(event)
        .ok_or_else(|| Exception::throw_type(ctx, "Expected an Event"))?;
    let kind = {
        let mut state = event.try_borrow_mut()?;
        if state.dispatch.active {
            drop(state);
            return Err(ctx.throw(named_error(
                ctx,
                "InvalidStateError",
                "Event is already being dispatched",
            )?));
        }
        state.target = Some(this.clone());
        state.dispatch.active = true;
        state.kind.clone()
    };
    let result = dispatch_listeners(&target, &event, &kind);
    let mut state = event.try_borrow_mut()?;
    state.dispatch = Dispatch::default();
    result.map(|()| !state.prevented)
}

fn dispatch_listeners<'js>(
    target: &Class<'js, Target<'js>>,
    event: &Class<'js, Event<'js>>,
    kind: &Atom<'js>,
) -> Result<()> {
    for capture in [true, false] {
        if event.try_borrow()?.dispatch.stopped {
            break;
        }
        let entries: Vec<_> = target
            .try_borrow()?
            .listeners
            .iter()
            .filter(|listener| &listener.kind == kind && listener.capture == capture)
            .cloned()
            .collect();
        for entry in entries {
            if event.try_borrow()?.dispatch.immediate {
                break;
            }
            if entry.removed.get() {
                continue;
            }
            let callback = if entry.handler {
                let Some(value) = target
                    .try_borrow()?
                    .listeners
                    .iter()
                    .find(|listener| listener.id == entry.id)
                    .map(|listener| listener.callback.clone())
                else {
                    continue;
                };
                value
            } else {
                entry.callback
            };
            if entry.once {
                remove_listener(target, entry.id)?;
            }
            event.try_borrow_mut()?.dispatch.passive = entry.passive;
            let event_value = event.as_inner().clone();
            let result = if let Some(function) = callback.as_function() {
                function.call::<_, ()>((This(target.as_inner().clone()), event_value))
            } else if let Some(object) = callback.as_object() {
                object
                    .get::<_, Function>("handleEvent")
                    .and_then(|function| function.call((This(object.clone()), event_value)))
            } else {
                Ok(())
            };
            if let Err(error) = result {
                report_listener_error(target.ctx(), error)?;
            }
        }
    }
    Ok(())
}

fn report_listener_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> Result<()> {
    let error = match error {
        rquickjs::Error::Exception => ctx.catch(),
        rquickjs::Error::FromJs { .. } => named_error(ctx, "TypeError", &error.to_string())?,
        other => return Err(other),
    };
    let reported = (|| {
        let console: Object = ctx.globals().get("console")?;
        let report: Function = console.get("error")?;
        report.call::<_, ()>((This(console), error))
    })();
    if reported.is_err() && ctx.has_exception() {
        drop(ctx.catch());
    }
    Ok(())
}

pub(super) fn define_handler<'js>(prototype: &Object<'js>, kind: &str) -> Result<()> {
    let kind = kind.to_owned();
    let get_kind = kind.clone();
    let property = format!("on{kind}");
    prototype.prop(
        property,
        Accessor::new(
            move |This(this): This<Class<'js, Target<'js>>>| {
                let get_kind = Atom::from_str(this.ctx().clone(), &get_kind)?;
                let callback = this
                    .try_borrow()?
                    .listeners
                    .iter()
                    .find(|entry| entry.handler && entry.kind == get_kind)
                    .map(|entry| entry.callback.clone());
                Ok::<_, rquickjs::Error>(
                    callback.unwrap_or_else(|| Value::new_null(this.ctx().clone())),
                )
            },
            move |This(this): This<Class<'js, Target<'js>>>, value: Value<'js>| {
                let kind = Atom::from_str(this.ctx().clone(), &kind)?;
                let mut state = this.try_borrow_mut()?;
                let index = state
                    .listeners
                    .iter()
                    .position(|entry| entry.handler && entry.kind == kind);
                match (index, value.is_function()) {
                    (Some(index), true) => state.listeners[index].callback = value,
                    (Some(index), false) => state.listeners.remove(index).removed.set(true),
                    (None, true) => {
                        state.insert(Listener {
                            id: 0,
                            kind,
                            callback: value,
                            capture: false,
                            once: false,
                            passive: false,
                            handler: true,
                            removed: Rc::default(),
                            abort: None,
                        })?;
                    }
                    (None, false) => {}
                }
                Ok::<_, rquickjs::Error>(())
            },
        )
        .configurable()
        .enumerable(),
    )
}

pub(super) fn event<'js>(ctx: &Ctx<'js>, kind: &str) -> Result<Object<'js>> {
    let kind = Atom::from_str(ctx.clone(), kind)?;
    Ok(Class::instance(ctx.clone(), Event::new(kind, false, false))?.into_inner())
}

pub(super) fn target_prototype<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    Class::<Target>::prototype(ctx)?.ok_or(rquickjs::Error::Unknown)
}

pub(super) fn target_with_attachment<'js>(
    prototype: &Object<'js>,
    attachment: Object<'js>,
) -> Result<Object<'js>> {
    Ok(Class::instance_proto(
        Target {
            attachment: Some(attachment),
            ..Target::default()
        },
        prototype.clone(),
    )?
    .into_inner())
}

pub(super) fn target_attachment<'js>(target: &Object<'js>) -> Result<Option<Object<'js>>> {
    let target = Class::<Target>::from_object(target)
        .ok_or_else(|| Exception::throw_type(target.ctx(), "Expected an EventTarget"))?;
    Ok(target.try_borrow()?.attachment.clone())
}

pub(super) fn new_signal<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let prototype = Class::<SignalPrototype>::prototype(ctx)?.ok_or(rquickjs::Error::Unknown)?;
    Ok(Class::instance_proto(
        Target {
            signal: Some(AbortState::default()),
            ..Target::default()
        },
        prototype,
    )?
    .into_inner())
}

pub(super) fn named_error<'js>(ctx: &Ctx<'js>, name: &str, message: &str) -> Result<Value<'js>> {
    if name == "TypeError" {
        Exception::throw_type(ctx, message);
        return Ok(ctx.catch());
    }
    let error = Exception::from_message(ctx.clone(), message)?.into_object();
    error.set("name", name)?;
    Ok(error.into_value())
}

fn event_kind<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Atom<'js>> {
    use rquickjs::FromJs;
    let text: rquickjs::String = if value.is_symbol() {
        let string: Function = ctx.globals().get("String")?;
        string.call((value,))?
    } else {
        Coerced::<rquickjs::String>::from_js(ctx, value)?.0
    };
    Atom::from_value(ctx.clone(), &text.into_value())
}

struct Construct<F>(F);
struct ConstructorArgument;

impl<'js, F> IntoJsFunc<'js, ConstructorArgument> for Construct<F>
where
    F: Fn(Params<'_, 'js>) -> Result<Value<'js>>,
{
    fn param_requirements() -> ParamRequirement {
        ParamRequirement::any()
    }

    fn call(&self, params: Params<'_, 'js>) -> Result<Value<'js>> {
        if !params.is_constructor() {
            return Err(Exception::throw_type(
                params.ctx(),
                "Constructor requires 'new'",
            ));
        }
        (self.0)(params)
    }
}

pub(super) fn constructor<'js>(
    ctx: &Ctx<'js>,
    prototype: Object<'js>,
    name: &str,
    arity: usize,
    body: impl Fn(Params<'_, 'js>) -> Result<Value<'js>> + 'js,
) -> Result<Function<'js>> {
    let constructor = Constructor::new_prototype(ctx, prototype, Construct(body))?;
    let function = constructor
        .into_value()
        .into_function()
        .ok_or(rquickjs::Error::Unknown)?;
    function.set_name(name)?;
    function.set_length(arity)?;
    Ok(function)
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, globals: &Object<'js>, portable: bool) -> Result<()> {
    let event_prototype = Class::<Event>::prototype(ctx)?.ok_or(rquickjs::Error::Unknown)?;
    globals.set(
        "Event",
        constructor(ctx, event_prototype, "Event", 1, |args| {
            let ctx = args.ctx();
            let kind = args
                .arg(0)
                .ok_or_else(|| Exception::throw_type(ctx, "Expected an event type"))?;
            let kind = event_kind(ctx, kind)?;
            let init = args.arg(1).and_then(Value::into_object);
            let flag = |name| {
                init.as_ref().map_or(Ok(false), |object| {
                    object.get::<_, Coerced<bool>>(name).map(|v| v.0)
                })
            };
            Ok(Class::instance(
                ctx.clone(),
                Event::new(kind, flag("bubbles")?, flag("cancelable")?),
            )?
            .into_value())
        })?,
    )?;
    let event_target_prototype = target_prototype(ctx)?;
    let target_constructor = constructor(
        ctx,
        event_target_prototype.clone(),
        "EventTarget",
        0,
        |args| Ok(Class::instance(args.ctx().clone(), Target::default())?.into_value()),
    )?;
    let signal_prototype =
        Class::<SignalPrototype>::prototype(ctx)?.ok_or(rquickjs::Error::Unknown)?;
    signal_prototype.set_prototype(Some(&event_target_prototype))?;
    signal_prototype.prop(
        "aborted",
        Accessor::new_get(|This(signal): This<Object<'js>>| {
            abort_reason(&signal).map(|reason| reason.is_some())
        })
        .configurable(),
    )?;
    signal_prototype.prop(
        "reason",
        Accessor::new_get(|This(signal): This<Object<'js>>| abort_reason(&signal)).configurable(),
    )?;
    signal_prototype.prop(
        "throwIfAborted",
        Property::from(Function::new(
            ctx.clone(),
            |This(signal): This<Object<'js>>| -> Result<()> {
                if let Some(reason) = abort_reason(&signal)? {
                    return Err(signal.ctx().throw(reason));
                }
                Ok(())
            },
        )?)
        .writable()
        .configurable(),
    )?;
    define_handler(&signal_prototype, "abort")?;
    let signal_constructor = constructor(ctx, signal_prototype, "AbortSignal", 0, move |args| {
        if portable {
            return Err(Exception::throw_type(args.ctx(), "Illegal constructor"));
        }
        Ok(new_signal(args.ctx())?.into_value())
    })?;
    signal_constructor.set_prototype(target_constructor.as_object())?;
    signal_constructor.prop(
        "abort",
        Property::from(Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, reason: Opt<Value<'js>>| -> Result<Object<'js>> {
                let signal = new_signal(&ctx)?;
                abort_signal(&signal, reason.0)?;
                Ok(signal)
            },
        )?)
        .writable()
        .configurable(),
    )?;
    globals.set("EventTarget", target_constructor)?;
    globals.set("AbortSignal", signal_constructor)?;
    let controller_prototype =
        Class::<Controller>::prototype(ctx)?.ok_or(rquickjs::Error::Unknown)?;
    globals.set(
        "AbortController",
        constructor(ctx, controller_prototype, "AbortController", 0, |args| {
            Ok(Class::instance(
                args.ctx().clone(),
                Controller {
                    signal: new_signal(args.ctx())?,
                },
            )?
            .into_value())
        })?,
    )?;
    Ok(())
}

pub(super) const DECLARATIONS: &str = r#"
interface EventInit {
  bubbles?: boolean;
  cancelable?: boolean;
}

declare class Event {
  constructor(type: string, init?: EventInit);
  readonly type: string;
  readonly bubbles: boolean;
  readonly cancelable: boolean;
  readonly target: EventTarget | null;
  readonly currentTarget: EventTarget | null;
  readonly eventPhase: 0 | 2;
  readonly defaultPrevented: boolean;
  cancelBubble: boolean;
  preventDefault(): void;
  stopPropagation(): void;
  stopImmediatePropagation(): void;
}

type EventListener = ((this: EventTarget, event: Event) => void) | { handleEvent(event: Event): void };

interface EventListenerOptions {
  capture?: boolean;
}

interface AddEventListenerOptions extends EventListenerOptions {
  once?: boolean;
  passive?: boolean;
  signal?: AbortSignal;
}

declare class EventTarget {
  addEventListener(type: string, listener: EventListener | null, options?: boolean | AddEventListenerOptions): void;
  removeEventListener(type: string, listener: EventListener | null, options?: boolean | EventListenerOptions): void;
  dispatchEvent(event: Event): boolean;
}

declare class AbortSignal extends EventTarget {
"#;

pub(super) const LEGACY_ABORT_SIGNAL_CONSTRUCTOR: &str = "  constructor();\n";
pub(super) const PORTABLE_ABORT_SIGNAL_CONSTRUCTOR: &str = "  private constructor();\n";

pub(super) const ABORT_DECLARATIONS: &str = r#"
  readonly aborted: boolean;
  readonly reason: unknown;
  onabort: ((this: AbortSignal, event: Event) => void) | null;
  throwIfAborted(): void;
  static abort(reason?: unknown): AbortSignal;
}

declare class AbortController {
  readonly signal: AbortSignal;
  abort(reason?: unknown): void;
}
"#;
