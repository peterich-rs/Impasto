//! App-confined file loading and the native scene, reactive and host modules.
use super::*;
use rquickjs::{
    Module,
    function::Rest,
    loader::{ImportAttributes, Loader, Resolver},
    module::{Declarations, Exports, ModuleDef},
};
use std::path::{Component, Path, PathBuf};

pub(super) const GRAPH_NAMES: &[&str] = &[
    "createSignal",
    "get",
    "set",
    "createMemo",
    "createRenderEffect",
    "createTrackedEffect",
    "pushOwner",
    "dispose",
    "owner",
    "root",
    "onCleanup",
    "pushUntrack",
    "popUntrack",
    "runUpdates",
    "pushBatch",
    "popBatch",
    "parentOwner",
    "isLive",
];
const REACTIVE_NAMES: &[&str] = &[
    "createSignal",
    "createMemo",
    "createRenderEffect",
    "createTrackedEffect",
    "createRoot",
    "getOwner",
    "runWithOwner",
    "onCleanup",
    "untrack",
    "batch",
    "createContext",
    "useContext",
    "children",
    "catchError",
];
fn scene_names() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for line in bootstrap::SCENE_JS.lines() {
        if let Some(rest) = line.trim().strip_prefix("core.")
            && let Some((name, _)) = rest.split_once(" = function")
            && !name.starts_with('_')
        {
            names.insert(name.to_owned());
        }
    }
    for name in bindings::CORE_NAMES
        .iter()
        .copied()
        .chain(bindings::GESTURES.iter().map(|(name, _)| *name))
        .chain([
            "scene",
            "createElement",
            "createTextNode",
            "setProp",
            "insertNode",
            "effect",
            "memo",
            "createComponent",
            "Path2D",
            "bindCanvas",
        ])
    {
        if !name.starts_with('_') {
            names.insert(name.to_owned());
        }
    }
    names.into_iter().collect()
}
pub(super) struct SceneModule;
impl ModuleDef for SceneModule {
    fn declare(decl: &Declarations<'_>) -> rquickjs::Result<()> {
        for name in scene_names() {
            decl.declare(name)?;
        }
        Ok(())
    }
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let host = host_from_ctx(ctx)?;
        let _active = host.enter(ctx);
        let api = Object::new(ctx.clone())?;
        let original = Object::from_value(
            host.refs
                .borrow()
                .scene
                .clone()
                .ok_or_else(|| type_error(ctx, "scene is missing"))?
                .restore(ctx)?,
        )?;
        for name in scene_names() {
            if name == "scene" {
                continue;
            }
            let value = match name.as_str() {
                "createElement" | "createTextNode" | "setProp" | "insertNode" | "effect"
                | "memo" | "createComponent" => {
                    let pointer = ptr::from_ref(host).cast_mut();
                    let name = name.clone();
                    Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                        // SAFETY: Host and Inner outlive the module and all its native functions.
                        let host = unsafe { &*pointer };
                        let _active = host.enter(&ctx);
                        renderer::call(host, &ctx, &name, &args.0)
                    })?
                    .into_value()
                }
                "insert" => original.get("_insertRange")?,
                "append" => original.get("_append")?,
                _ => original.get(name.as_str())?,
            };
            api.set(name.as_str(), value.clone())?;
            exports.export(name, value)?;
        }
        exports.export("scene", api)?;
        Ok(())
    }
}
pub(super) struct ReactiveModule;
impl ModuleDef for ReactiveModule {
    fn declare(decl: &Declarations<'_>) -> rquickjs::Result<()> {
        for name in REACTIVE_NAMES {
            decl.declare(*name)?;
        }
        Ok(())
    }
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let host = host_from_ctx(ctx)?;
        let api = Object::from_value(
            host.refs
                .borrow()
                .reactive
                .clone()
                .ok_or_else(|| type_error(ctx, "reactive is missing"))?
                .restore(ctx)?,
        )?;
        for name in REACTIVE_NAMES {
            exports.export(*name, api.get::<_, Value>(*name)?)?;
        }
        Ok(())
    }
}
pub(super) struct GraphModule;
impl ModuleDef for GraphModule {
    fn declare(decl: &Declarations<'_>) -> rquickjs::Result<()> {
        for name in GRAPH_NAMES {
            decl.declare(*name)?;
        }
        Ok(())
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "JavaScript f64 values convert to the existing fixed-width callback record and f32 geometry"
    )]
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let host = host_from_ctx(ctx)?;
        let pointer = ptr::from_ref(host).cast_mut();
        for (index, name) in GRAPH_NAMES.iter().enumerate() {
            let op = index as u32 + 1;
            let function = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                    // SAFETY: Engine clears graph values before destroying this function or Host.
                    let host = unsafe { &*pointer };
                    let _active = host.enter(&ctx);
                    if matches!(op, 1 | 3 | 4 | 5 | 6 | 8 | 10 | 11 | 14 | 15 | 16) {
                        js::host::OperationPolicy::before(host, &ctx)?;
                    }
                    let mut handles = Vec::new();
                    for (index, value) in args.0.into_iter().take(8).enumerate() {
                        let is_id = matches!(op, 2 | 7 | 8 | 13 | 17 | 18)
                            || op == 3 && index == 0
                            || op == 5 && index == 2;
                        if is_id && value.is_number() {
                            handles.push(value.as_number().unwrap_or(0.0) as u64);
                        } else {
                            handles.push(
                                host.inner()
                                    .graph
                                    .hold(reactive::Value::save(&ctx, pointer, value)),
                            );
                        }
                    }
                    let result = reactive::reactive_entry(host.inner(), op, &handles);
                    if ctx.has_exception() {
                        return Err(rquickjs::Error::Exception);
                    }
                    if result == 0 && !matches!(op, 1 | 4 | 7 | 9 | 12) {
                        return Ok(Value::new_undefined(ctx));
                    }
                    if result & (1 << 63) != 0 {
                        return ((result & !(1 << 63)) as f64).into_js(&ctx);
                    }
                    match host.inner().graph.take(result) {
                        Some(value) => value.restore(&ctx),
                        None => (-1.0).into_js(&ctx),
                    }
                },
            )?;
            function.set_name(name)?;
            exports.export(*name, function)?;
        }
        Ok(())
    }
}

pub(super) struct HostModule;
impl ModuleDef for HostModule {
    fn declare(decl: &Declarations<'_>) -> rquickjs::Result<()> {
        let name: String = decl.module().name()?;
        let host = host_from_ctx(decl.module().ctx())?;
        let module = host
            .inner()
            .config
            .registry
            .by_specifier(&name)
            .ok_or_else(|| {
                type_error(
                    decl.module().ctx(),
                    &format!("module '{name}' is not linked"),
                )
            })?;
        for method in &module.methods {
            decl.declare(method.name)?;
        }
        decl.declare("addEventListener")?;
        decl.declare("removeEventListener")?;
        Ok(())
    }
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let specifier: String = exports.module().name()?;
        let host = host_from_ctx(ctx)?;
        let pointer = ptr::from_ref(host).cast_mut();
        let module = host
            .inner()
            .config
            .registry
            .by_specifier(&specifier)
            .ok_or_else(|| type_error(ctx, "module is not linked"))?;
        for method in &module.methods {
            let name = module.name.to_owned();
            let method = method.name.to_owned();
            let export = method.clone();
            exports.export(
                export,
                Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                    // SAFETY: module functions are destroyed with the VM before Host.
                    let host = unsafe { &*pointer };
                    let _active = host.enter(&ctx);
                    js::host::OperationPolicy::before(host, &ctx)?;
                    call_host(host, &ctx, &name, &method, &args.0)
                })?,
            )?;
        }
        for remove in [false, true] {
            let name = module.name.to_owned();
            exports.export(
                if remove {
                    "removeEventListener"
                } else {
                    "addEventListener"
                },
                Function::new(
                    ctx.clone(),
                    move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<()> {
                        // SAFETY: Host outlives the native module's functions.
                        let host = unsafe { &*pointer };
                        let _active = host.enter(&ctx);
                        js::host::OperationPolicy::before(host, &ctx)?;
                        if args.0.len() < 2 || !args.0[0].is_string() || !args.0[1].is_function() {
                            return Ok(());
                        }
                        let event = bindings::string(&ctx, &args.0, 0)?;
                        let saved = Persistent::save(&ctx, args.0[1].clone());
                        if remove {
                            host.runtime.remove_listener(&name, &event, &saved);
                        } else {
                            host.runtime.add_listener(name.clone(), event, saved);
                        }
                        Ok(())
                    },
                )?,
            )?;
        }
        Ok(())
    }
}
fn call_host<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    name: &str,
    method: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    // Decode getters before preparing a call: reentrant module calls cannot overwrite its builder.
    let decoded = args
        .iter()
        .cloned()
        .map(|v| bindings::plugin_from_js(ctx, v, 0))
        .collect::<rquickjs::Result<Vec<_>>>()?;
    // SAFETY: module_prepare copies both borrowed names into a session-owned call.
    let kind = unsafe {
        super::super::bridge::module_prepare(
            host.session(),
            name.as_ptr().cast(),
            name.len(),
            method.as_ptr().cast(),
            method.len(),
        )
    };
    if kind < 0 {
        return Err(type_error(ctx, &last_error(host)));
    }
    let promise = if kind == 1 {
        let (token, promise) = host.make_promise(ctx)?;
        // SAFETY: the prepared call belongs to this live session.
        unsafe {
            super::super::bridge::module_set_token(host.session(), token);
        }
        Some((token, promise))
    } else {
        None
    };
    for value in decoded {
        super::super::bridge::push_prepared(host.inner(), value);
    }
    // SAFETY: module_finish consumes the prepared call after all arguments are owned.
    let finished = unsafe { super::super::bridge::module_finish(host.session()) };
    if finished < 0 {
        let message = last_error(host);
        if let Some((token, promise)) = promise {
            host.settle(ctx, token, false, message.into_js(ctx)?)?;
            return Ok(promise);
        }
        return Err(type_error(ctx, &message));
    }
    if let Some((_, promise)) = promise {
        Ok(promise)
    } else {
        match super::super::bridge::take_sync_result(host.inner()) {
            Some(value) => bindings::plugin_to_js(ctx, &value),
            None => Ok(Value::new_null(ctx.clone())),
        }
    }
}
fn last_error(host: &Host) -> String {
    host.inner()
        .last_error
        .borrow()
        .as_ref()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "module is not linked".to_owned())
}

fn is_host(name: &str) -> bool {
    name.strip_prefix("@impasto/")
        .is_some_and(|s| !s.is_empty() && !s.contains(['/', '\\']))
}
fn rejected(ctx: &Ctx<'_>) -> rquickjs::Error {
    rquickjs::Exception::throw_reference(ctx, "module path is outside the app directory")
}
pub(super) struct AppResolver {
    directory: Option<PathBuf>,
}
impl AppResolver {
    pub(super) fn new(dir: Option<&str>) -> Self {
        Self {
            directory: dir.map(PathBuf::from),
        }
    }
}
impl Resolver for AppResolver {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if matches!(name, "scene" | "reactive" | "#reactive") || is_host(name) {
            return Ok(name.to_owned());
        }
        if name.starts_with("@impasto/") {
            return Err(rquickjs::Exception::throw_reference(
                ctx,
                "bare @impasto/* must be bundled; runtime only loads @impasto/<pkg>",
            ));
        }
        let app = self.directory.as_ref().ok_or_else(|| rejected(ctx))?;
        if name.is_empty() || name.contains('\\') || name.contains("://") {
            return Err(rejected(ctx));
        }
        let raw = if name.starts_with('/') {
            PathBuf::from(name)
        } else {
            Path::new(base)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(app)
                .join(name)
        };
        if raw.as_os_str().len() >= 4096 {
            return Err(rejected(ctx));
        }
        let relative = raw.strip_prefix(app).map_err(|_| rejected(ctx))?;
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if parts.pop().is_none() {
                        return Err(rejected(ctx));
                    }
                }
                Component::Normal(segment) => {
                    let segment = segment.to_str().ok_or_else(|| rejected(ctx))?;
                    if !segment
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                    {
                        return Err(rejected(ctx));
                    }
                    parts.push(segment);
                }
                _ => return Err(rejected(ctx)),
            }
        }
        let mut result = app.clone();
        for segment in parts {
            result.push(segment);
        }
        if result.as_os_str().len() >= 4096 {
            return Err(rejected(ctx));
        }
        if result.extension().is_none_or(|e| e != "js") {
            return Err(rejected(ctx));
        }
        result
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| rejected(ctx))
    }
}
pub(super) struct AppLoader {
    host: *mut Host,
    directory: Option<PathBuf>,
}
impl AppLoader {
    pub(super) fn new(host: *mut Host, dir: Option<&str>) -> Self {
        Self {
            host,
            directory: dir.map(PathBuf::from),
        }
    }
}
impl Loader for AppLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        match name {
            "scene" => return Module::declare_def::<SceneModule, _>(ctx.clone(), name),
            "reactive" => return Module::declare_def::<ReactiveModule, _>(ctx.clone(), name),
            "#reactive" => return Module::declare_def::<GraphModule, _>(ctx.clone(), name),
            _ => {}
        }
        if is_host(name) {
            // SAFETY: the loader lives inside the Runtime owned by Host.
            let host = unsafe { &*self.host };
            if host.inner().config.registry.by_specifier(name).is_none() {
                return Err(rquickjs::Exception::throw_reference(
                    ctx,
                    &format!("module '{name}' is not linked"),
                ));
            }
            return Module::declare_def::<HostModule, _>(ctx.clone(), name);
        }
        let app = self.directory.as_ref().ok_or_else(|| rejected(ctx))?;
        let path = Path::new(name);
        if !path.starts_with(app) {
            return Err(rejected(ctx));
        }
        // Also reject symlinks that escape the app, before reading any bytes.
        let canonical = std::fs::canonicalize(path).map_err(|_| {
            rquickjs::Exception::throw_reference(ctx, &format!("could not load module '{name}'"))
        })?;
        let app = std::fs::canonicalize(app).map_err(|_| rejected(ctx))?;
        if !canonical.starts_with(app) {
            return Err(rejected(ctx));
        }
        use std::io::Read;
        let file = std::fs::File::open(canonical).map_err(|_| rejected(ctx))?;
        if file.metadata().map_err(|_| rejected(ctx))?.len() > 16 << 20 {
            return Err(rejected(ctx));
        }
        let mut source = Vec::new();
        file.take((16 << 20) + 1)
            .read_to_end(&mut source)
            .map_err(|_| rejected(ctx))?;
        if source.len() > 16 << 20 {
            return Err(rejected(ctx));
        }
        Module::declare(ctx.clone(), name, source)
    }
}

struct CompileResolver;
impl Resolver for CompileResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        _base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        Ok(name.to_owned())
    }
}
struct CompileLoader;
struct EmptyModule;
impl ModuleDef for EmptyModule {}
impl Loader for CompileLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        match name {
            "scene" => Module::declare_def::<SceneModule, _>(ctx.clone(), name),
            "reactive" => Module::declare_def::<ReactiveModule, _>(ctx.clone(), name),
            "#reactive" => Module::declare_def::<GraphModule, _>(ctx.clone(), name),
            _ if is_host(name) => Module::declare_def::<EmptyModule, _>(ctx.clone(), name),
            _ => Err(rquickjs::Exception::throw_reference(
                ctx,
                &format!("bundle has an unresolved module: {name}"),
            )),
        }
    }
}
pub(crate) fn prepare_compile(runtime: &Runtime) {
    runtime.set_loader(CompileResolver, CompileLoader);
}
