//! Native module declarations and callable exports, shared by all runtime drivers.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;

use crate::host::{
    HostEvent, Invoke, MethodExec, Module, ModuleRegistry, NativeResource, PluginValue, Settle,
    StreamSink,
};
use rquickjs::function::Rest;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::object::Property;
use rquickjs::{Ctx, Exception, Function, JsLifetime, Object, Persistent, Value};

use super::Saved;
use super::codec::{decode_args_with, encode};

#[derive(Clone)]
pub struct ModuleBindings {
    registry: ModuleRegistry,
    exec: Arc<dyn MethodExec>,
}

impl ModuleBindings {
    pub fn new(registry: ModuleRegistry, exec: Arc<dyn MethodExec>) -> Self {
        Self { registry, exec }
    }

    pub fn registry(&self) -> &ModuleRegistry {
        &self.registry
    }
}

/// Runtime operations used while a native module is on the JavaScript stack.
/// Implementations keep the runtime alive until its context releases this binder,
/// or use a weak handle to the runtime state.
pub(crate) trait ModuleRuntime {
    fn enter<'a>(&'a self, ctx: &Ctx<'_>) -> Option<super::vm::Active<'a>>;
    fn before(&self, ctx: &Ctx<'_>, removing_listener: bool) -> rquickjs::Result<()>;
    fn reserve_plugin<'js>(
        &self,
        ctx: &Ctx<'js>,
        awaits_user: bool,
    ) -> rquickjs::Result<(Value<'js>, Settle)>;
    fn open_resource<'js>(
        &self,
        ctx: &Ctx<'js>,
        create: &mut dyn FnMut() -> Result<Box<dyn NativeResource>, String>,
    ) -> rquickjs::Result<Value<'js>>;
    fn reserve_stream<'js>(
        &self,
        ctx: &Ctx<'js>,
        signal: Option<Object<'js>>,
    ) -> rquickjs::Result<(Value<'js>, StreamSink)>;
    fn add_listener(&self, module: &str, event: &str, callback: Saved);
    fn remove_listener(&self, module: &str, event: &str, callback: &Saved);
}

#[derive(Clone)]
struct Binder {
    bindings: ModuleBindings,
    runtime: Rc<dyn ModuleRuntime>,
}

// SAFETY: the binder contains only native metadata and the runtime provider.
// Its layout is independent of the current JavaScript context lifetime.
unsafe impl<'js> JsLifetime<'js> for Binder {
    type Changed<'to> = Self;
}

pub(crate) fn install(
    ctx: &Ctx<'_>,
    bindings: ModuleBindings,
    runtime: Rc<dyn ModuleRuntime>,
) -> rquickjs::Result<()> {
    ctx.store_userdata(Binder { bindings, runtime })
        .map_err(|_| Exception::throw_type(ctx, "module binder is in use"))?;
    Ok(())
}

pub fn declare_host_module<'js>(
    ctx: &Ctx<'js>,
    name: &str,
) -> rquickjs::Result<rquickjs::Module<'js>> {
    let linked = ctx
        .userdata::<Binder>()
        .is_some_and(|binder| binder.bindings.registry.by_specifier(name).is_some());
    if !linked {
        return Err(Exception::throw_reference(
            ctx,
            &format!("module '{name}' is not linked"),
        ));
    }
    rquickjs::Module::declare_def::<BoundModule, _>(ctx.clone(), name)
}

struct BoundModule;

impl ModuleDef for BoundModule {
    fn declare(decl: &Declarations<'_>) -> rquickjs::Result<()> {
        let specifier: String = decl.module().name()?;
        let ctx = decl.module().ctx();
        let (module, _) = module_binder(ctx, &specifier)?;
        for name in module.export_names() {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let specifier: String = exports.module().name()?;
        let (module, binder) = module_binder(ctx, &specifier)?;
        let object = module
            .exports
            .object
            .then(|| Object::new_proto(ctx.clone(), None))
            .transpose()?;
        for method in module.methods {
            let exec = Arc::clone(&binder.bindings.exec);
            let runtime = Rc::clone(&binder.runtime);
            let invoke = method.invoke;
            let awaits_user = method.awaits_user;
            let strict = method.strict_arguments;
            let function =
                Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                    let _active = runtime.enter(&ctx);
                    runtime.before(&ctx, false)?;
                    let mut args = args.0;
                    let signal = matches!(&invoke, Invoke::Stream(_))
                        .then(|| crate::standard::native_stream::signal(&mut args))
                        .flatten();
                    let args = decode_args_with(&ctx, args, strict)?;
                    call(&ctx, &*runtime, &*exec, &invoke, args, signal, awaits_user)
                })?;
            export_function(exports, object.as_ref(), method.name, function)?;
        }
        let events = module.exports.events;
        if !events.is_empty() {
            for (name, remove) in [("addEventListener", false), ("removeEventListener", true)] {
                let runtime = Rc::clone(&binder.runtime);
                let module_name = module.name;
                let function = Function::new(
                    ctx.clone(),
                    move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<()> {
                        let _active = runtime.enter(&ctx);
                        runtime.before(&ctx, remove)?;
                        let [event, callback, ..] = &args.0[..] else {
                            return Ok(());
                        };
                        let (Some(event), true) = (event.as_string(), callback.is_function())
                        else {
                            return Ok(());
                        };
                        let event = event.to_string()?;
                        if !events.iter().any(|entry| entry.name == event) {
                            return Ok(());
                        }
                        let callback = Persistent::save(&ctx, callback.clone());
                        if remove {
                            runtime.remove_listener(module_name, &event, &callback);
                        } else {
                            runtime.add_listener(module_name, &event, callback);
                        }
                        Ok(())
                    },
                )?;
                export_function(exports, object.as_ref(), name, function)?;
            }
        }
        if let Some(object) = object {
            exports.export(module.name, object)?;
        }
        Ok(())
    }
}

fn module_binder(ctx: &Ctx<'_>, specifier: &str) -> rquickjs::Result<(Module, Binder)> {
    let binder = ctx
        .userdata::<Binder>()
        .map(|binder| (*binder).clone())
        .ok_or_else(|| Exception::throw_type(ctx, "module binder is not installed"))?;
    let module = binder
        .bindings
        .registry
        .by_specifier(specifier)
        .cloned()
        .ok_or_else(|| {
            Exception::throw_type(ctx, &format!("module '{specifier}' is not linked"))
        })?;
    Ok((module, binder))
}

fn export_function<'js>(
    exports: &Exports<'js>,
    object: Option<&Object<'js>>,
    name: &str,
    function: Function<'js>,
) -> rquickjs::Result<()> {
    if let Some(object) = object {
        object.prop(name, Property::from(function.clone()).enumerable())?;
    }
    exports.export(name, function)?;
    Ok(())
}

pub(super) fn call_resource<'js>(
    ctx: &Ctx<'js>,
    method: crate::host::Method,
    mut args: Vec<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let binder = ctx
        .userdata::<Binder>()
        .map(|binder| (*binder).clone())
        .ok_or_else(|| Exception::throw_type(ctx, "module binder is not installed"))?;
    let runtime = &*binder.runtime;
    let _active = runtime.enter(ctx);
    runtime.before(ctx, false)?;
    let signal = matches!(&method.invoke, Invoke::Stream(_))
        .then(|| crate::standard::native_stream::signal(&mut args))
        .flatten();
    let args = decode_args_with(ctx, args, method.strict_arguments)?;
    call(
        ctx,
        runtime,
        &*binder.bindings.exec,
        &method.invoke,
        args,
        signal,
        method.awaits_user,
    )
}

fn call<'js>(
    ctx: &Ctx<'js>,
    runtime: &dyn ModuleRuntime,
    exec: &dyn MethodExec,
    invoke: &Invoke,
    args: Vec<PluginValue>,
    signal: Option<Object<'js>>,
    awaits_user: bool,
) -> rquickjs::Result<Value<'js>> {
    match invoke {
        Invoke::Sync(invoke) => {
            let value = catch_unwind(AssertUnwindSafe(|| invoke(&args)))
                .map_err(|_| Exception::throw_type(ctx, "host callback panicked"))?
                .map_err(|message| Exception::throw_type(ctx, &message))?;
            encode(ctx, &value)
        }
        Invoke::Resource(open) => runtime.open_resource(ctx, &mut || open(&args)),
        Invoke::Stream(start) => {
            let (stream, sink) = runtime.reserve_stream(ctx, signal)?;
            if !sink.cancellation().is_cancelled() {
                let _ = catch_unwind(AssertUnwindSafe(|| start(args, sink, exec)));
            }
            Ok(stream)
        }
        Invoke::Promise(invoke) => {
            let (promise, settle) = runtime.reserve_plugin(ctx, awaits_user)?;
            // The one-shot completion rejects on drop if the method unwinds.
            let _ = catch_unwind(AssertUnwindSafe(|| invoke(&args, settle, exec)));
            Ok(promise)
        }
    }
}

/// Deliver to the subscription snapshot. Encode a fresh payload per callback
/// so a listener cannot change the value observed by the rest of the snapshot.
pub(crate) fn deliver_event(
    ctx: &Ctx<'_>,
    callbacks: Vec<Saved>,
    event: &HostEvent,
    mut on_error: impl FnMut(&Ctx<'_>, rquickjs::Error) -> bool,
) -> usize {
    let mut delivered = 0;
    for callback in callbacks {
        let result = callback
            .restore(ctx)
            .and_then(Function::from_value)
            .and_then(|function| function.call::<_, ()>((encode(ctx, &event.payload)?,)));
        delivered += 1;
        if let Err(error) = result
            && !on_error(ctx, error)
        {
            break;
        }
    }
    delivered
}
