//! App-confined file loading and the native scene, reactive and host modules.
use super::*;
use rquickjs::{
    Module, Runtime,
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
// Renderer adapters, JS insertion helpers and Canvas exports.
const SCENE_ADAPTER_NAMES: &[&str] = &[
    "append",
    "insert",
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
];

fn scene_names() -> &'static [&'static str] {
    static NAMES: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = bindings::names()
            .chain(SCENE_ADAPTER_NAMES.iter().copied())
            .filter(|name| !name.starts_with('_'))
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        names
    })
}

pub(crate) fn compile_declarations() -> [(&'static str, &'static [&'static str]); 3] {
    [
        ("scene", scene_names()),
        ("reactive", REACTIVE_NAMES),
        ("#reactive", GRAPH_NAMES),
    ]
}

pub(super) struct SceneModule;
impl ModuleDef for SceneModule {
    fn declare(decl: &Declarations<'_>) -> rquickjs::Result<()> {
        for &name in scene_names() {
            decl.declare(name)?;
        }
        Ok(())
    }
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let host = host_from_ctx(ctx)?;
        host.with_current(ctx, || {
            let api = Object::new(ctx.clone())?;
            let original = Object::from_value(
                host.refs
                    .borrow()
                    .scene
                    .clone()
                    .ok_or_else(|| type_error(ctx, "scene is missing"))?
                    .restore(ctx)?,
            )?;
            for &name in scene_names() {
                if name == "scene" {
                    continue;
                }
                let value = match name {
                    "createElement" | "createTextNode" | "setProp" | "insertNode" | "effect"
                    | "memo" | "createComponent" => {
                        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                            let host = host_from_ctx(&ctx)?;
                            host.with_current(&ctx, || renderer::call(&host, &ctx, name, &args.0))
                        })?
                        .into_value()
                    }
                    "insert" => original.get("_insertRange")?,
                    "append" => original.get("_append")?,
                    _ => original.get(name)?,
                };
                api.set(name, value.clone())?;
                exports.export(name, value)?;
            }
            exports.export("scene", api)?;
            Ok(())
        })
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
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        for &name in GRAPH_NAMES {
            let function =
                Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                    let host = host_from_ctx(&ctx)?;
                    host.with_current(&ctx, || graph_call(&host, &ctx, name, &args.0))
                })?;
            function.set_name(name)?;
            exports.export(name, function)?;
        }
        Ok(())
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "reactive handles are exact nonnegative JS numbers within the ID range"
)]
fn graph_call<'js>(
    host: &Host,
    ctx: &Ctx<'js>,
    name: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<Value<'js>> {
    use crate::reactive::Id;
    use reactive::{Cleanup, JsEval};

    if matches!(
        name,
        "createSignal"
            | "set"
            | "createMemo"
            | "createRenderEffect"
            | "createTrackedEffect"
            | "dispose"
            | "root"
            | "onCleanup"
            | "runUpdates"
            | "pushBatch"
            | "popBatch"
    ) {
        canvas::side_effect(ctx)?;
    }

    let graph = &host.inner().graph;
    let raw_id = args.first().and_then(Value::as_number).unwrap_or_default() as u64;
    let id = Id::from_raw(raw_id);
    let optional_id = (raw_id != 0).then_some(id);
    let id_value = |id: Option<Id>| Value::new_float(ctx.clone(), id.map_or(0, Id::to_raw) as f64);
    let saved = |index: usize| {
        args.get(index)
            .map(|value| reactive::Value::save(ctx, host, value.clone()))
    };
    let undefined = || Value::new_undefined(ctx.clone());
    let value = match name {
        "createSignal" => id_value(saved(0).map(|value| graph.create_signal(value))),
        "get" => graph
            .get(JsEval, id)
            .map_or_else(|| Ok(undefined()), |value| value.restore(ctx))?,
        "set" => saved(1).map_or_else(undefined, |value| {
            Value::new_int(ctx.clone(), i32::from(graph.set(JsEval, id, value)))
        }),
        "createMemo" => id_value(saved(0).map(|compute| graph.create_memo(compute))),
        "createRenderEffect" => {
            if let (Some(compute), Some(effect)) = (saved(0), saved(1)) {
                let scheduled = args
                    .get(2)
                    .is_some_and(|value| value.as_number().is_none_or(|number| number as u64 != 0));
                graph.render_effect(JsEval, compute, effect, scheduled);
            }
            undefined()
        }
        "createTrackedEffect" => {
            if let Some(compute) = saved(0) {
                graph.create_tracked_effect(compute);
            }
            undefined()
        }
        "pushOwner" => id_value(graph.push_owner(optional_id)),
        "dispose" => {
            graph.dispose(JsEval, id);
            undefined()
        }
        "owner" => id_value(graph.owner()),
        "root" => id_value(Some(graph.open_root())),
        "onCleanup" => {
            if let Some(cleanup) = saved(0) {
                graph.on_cleanup(Cleanup::Script(cleanup));
            }
            undefined()
        }
        "pushUntrack" => id_value(graph.push_untrack()),
        "popUntrack" => {
            graph.pop_reader(optional_id);
            undefined()
        }
        "runUpdates" => {
            graph.run_updates(JsEval);
            undefined()
        }
        "pushBatch" => {
            graph.push_batch();
            undefined()
        }
        "popBatch" => {
            graph.pop_batch(JsEval);
            undefined()
        }
        "parentOwner" => id_value(graph.owner_parent(id)),
        "isLive" => Value::new_int(ctx.clone(), i32::from(graph.is_live(id))),
        _ => return Err(type_error(ctx, "unknown reactive operation")),
    };

    if ctx.has_exception() {
        Err(rquickjs::Error::Exception)
    } else {
        Ok(value)
    }
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
    registry: Arc<crate::session::ModuleRegistry>,
}
impl AppResolver {
    pub(super) fn new(dir: Option<&str>, registry: Arc<crate::session::ModuleRegistry>) -> Self {
        Self {
            directory: dir.map(PathBuf::from),
            registry,
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
        if matches!(name, "scene" | "reactive" | "#reactive")
            || self.registry.by_specifier(name).is_some()
            || is_host(name)
        {
            return Ok(name.to_owned());
        }
        if name.starts_with("@impasto/") {
            return Err(rquickjs::Exception::throw_reference(
                ctx,
                "bare @impasto/* must be bundled; runtime only loads @impasto/<pkg>",
            ));
        }
        let app = self.directory.as_ref().ok_or_else(|| rejected(ctx))?;
        if name.is_empty() || name.contains(['\\', '\0']) || name.contains("//") {
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
    directory: Option<PathBuf>,
    canonical_directory: Option<PathBuf>,
    registry: Arc<crate::session::ModuleRegistry>,
}
impl AppLoader {
    pub(super) fn new(dir: Option<&str>, registry: Arc<crate::session::ModuleRegistry>) -> Self {
        Self {
            directory: dir.map(PathBuf::from),
            canonical_directory: None,
            registry,
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
        if self.registry.by_specifier(name).is_some() || is_host(name) {
            return javascript::runtime::extension::declare_host_module(ctx, name);
        }
        let app = self.directory.as_ref().ok_or_else(|| rejected(ctx))?;
        let logical = Path::new(name)
            .strip_prefix(app)
            .map_err(|_| rejected(ctx))?;
        let canonical_app = match self.canonical_directory.as_ref() {
            Some(path) => path,
            None => self
                .canonical_directory
                .insert(std::fs::canonicalize(app).map_err(|_| rejected(ctx))?),
        };
        let canonical = physical_path(ctx, canonical_app, logical)?;
        use std::io::Read;
        let file = std::fs::File::open(&canonical).map_err(|_| rejected(ctx))?;
        let metadata = file.metadata().map_err(|_| rejected(ctx))?;
        if !metadata.is_file() || metadata.len() > 16 << 20 {
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

fn physical_path(ctx: &Ctx<'_>, root: &Path, logical: &Path) -> rquickjs::Result<PathBuf> {
    let mut path = root.to_owned();
    for component in logical.components() {
        let Component::Normal(segment) = component else {
            return Err(rejected(ctx));
        };
        // Read the directory entry to check case on case-insensitive volumes.
        // Canonicalize each hop before walking into a symlinked directory.
        let mut entries = std::fs::read_dir(&path).map_err(|_| rejected(ctx))?;
        if !entries.any(|entry| entry.is_ok_and(|entry| entry.file_name() == segment)) {
            return Err(rquickjs::Exception::throw_reference(
                ctx,
                &format!("could not load module '{}'", logical.display()),
            ));
        }
        path = std::fs::canonicalize(path.join(segment)).map_err(|_| rejected(ctx))?;
        if !path.starts_with(root) {
            return Err(rejected(ctx));
        }
    }
    if path.is_file() {
        Ok(path)
    } else {
        Err(rejected(ctx))
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

#[cfg(test)]
#[allow(clippy::expect_used, reason = "module export assertions")]
mod tests {
    use super::*;

    #[test]
    fn runtime_extension_names_match_the_compiler_descriptions() {
        use std::fmt::Write as _;

        let mut source = String::new();
        for (index, (name, names)) in compile_declarations().into_iter().enumerate() {
            let mut names = names.to_vec();
            names.sort_unstable();
            writeln!(
                source,
                "import * as m{index} from {name:?};\n\
                if (Object.keys(m{index}).sort().join(',') !== {:?}) throw Error({name:?});\n\
                for (const key of Object.keys(m{index})) {{\n\
                    if (typeof m{index}[key] !== (key === 'scene' ? 'object' : 'function')) {{\n\
                        throw Error({name:?} + ': missing binding ' + key);\n\
                    }}\n\
                }}",
                names.join(","),
            )
            .expect("write source");
        }
        assert!(
            crate::js::JsSession::open(
                &source,
                "main.js",
                crate::text::MonoShaper::default(),
                Box::new(|_, _| {})
            )
            .is_ok()
        );
    }
}
