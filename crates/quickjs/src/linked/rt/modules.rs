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
// Public functions installed by scene.js. Runtime and compiler use the same
// export view; the module test checks these names against the installed object.
const SCENE_SCRIPT_NAMES: &[&str] = &[
    "align",
    "animate",
    "append",
    "blur",
    "caretAt",
    "caretAtUnit",
    "clip",
    "column",
    "decorated",
    "detach",
    "discard",
    "firstChild",
    "fling",
    "flingTarget",
    "focus",
    "focusStep",
    "focused",
    "graphemeCount",
    "graphemeIndex",
    "graphemeUnit",
    "grid",
    "image",
    "insert",
    "isFocusable",
    "isText",
    "layerTreeStats",
    "measurePanel",
    "measureText",
    "metrics",
    "nextSibling",
    "opacity",
    "overscroll",
    "padding",
    "parent",
    "placePanel",
    "pointerPosition",
    "pointerRegion",
    "popFocusScope",
    "pushFocusScope",
    "rect",
    "reduceMotion",
    "registerFont",
    "reload",
    "remove",
    "reparentChildren",
    "row",
    "scroll",
    "scrollIntoView",
    "scrollOffset",
    "scrollTo",
    "selectionRects",
    "setAlign",
    "setAlignSelf",
    "setAlpha",
    "setAspectRatio",
    "setAxisSize",
    "setBasis",
    "setColor",
    "setCross",
    "setDragClaim",
    "setEllipsis",
    "setFamily",
    "setFill",
    "setFocusable",
    "setFontSize",
    "setFontWeight",
    "setGap",
    "setGridLine",
    "setGridPlacement",
    "setGrow",
    "setImage",
    "setImageMode",
    "setImeConfig",
    "setImeEnabled",
    "setImeState",
    "setImeText",
    "setInset",
    "setInsetEdge",
    "setJustify",
    "setLineHeight",
    "setMargin",
    "setMaxAxis",
    "setMaxLines",
    "setMaxSize",
    "setMinAxis",
    "setMinSize",
    "setOverflow",
    "setPadding",
    "setPaddingEdge",
    "setPaddingEdges",
    "setPanel",
    "setPosition",
    "setRadius",
    "setRecognizers",
    "setScrollAxis",
    "setScrollChain",
    "setScrollOffset",
    "setScrollPhysics",
    "setSemantics",
    "setShrink",
    "setSize",
    "setSizeAxis",
    "setSoftWrap",
    "setStyleSurface",
    "setText",
    "setTextAlign",
    "setTextDirection",
    "setTextLimit",
    "setTextRuns",
    "setTextScale",
    "setTransform",
    "setTransformComponent",
    "setWrap",
    "sized",
    "stack",
    "stopScroll",
    "text",
    "textCommand",
    "textEdit",
    "textScale",
    "transform",
    "view",
    "wordRange",
];
const SCENE_ADAPTER_NAMES: &[&str] = &[
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
        let mut names = SCENE_SCRIPT_NAMES
            .iter()
            .chain(SCENE_ADAPTER_NAMES)
            .chain(bindings::CORE_NAMES)
            .copied()
            .chain(bindings::GESTURES.iter().map(|(name, _)| *name))
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
                        let pointer = ptr::from_ref(host).cast_mut();
                        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                            // SAFETY: Host and Inner outlive the module and all its native functions.
                            let host = unsafe { &*pointer };
                            host.with_current(&ctx, || renderer::call(host, &ctx, name, &args.0))
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
                    host.with_current(&ctx, || {
                        if matches!(op, 1 | 3 | 4 | 5 | 6 | 8 | 10 | 11 | 14 | 15 | 16) {
                            canvas::side_effect(&ctx)?;
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
                            return Ok(Value::new_undefined(ctx.clone()));
                        }
                        if result & (1 << 63) != 0 {
                            return ((result & !(1 << 63)) as f64).into_js(&ctx);
                        }
                        match host.inner().graph.take(result) {
                            Some(value) => value.restore(&ctx),
                            None => (-1.0).into_js(&ctx),
                        }
                    })
                },
            )?;
            function.set_name(name)?;
            exports.export(*name, function)?;
        }
        Ok(())
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
}
impl AppLoader {
    pub(super) fn new(dir: Option<&str>) -> Self {
        Self {
            directory: dir.map(PathBuf::from),
            canonical_directory: None,
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
            return js::quickjs::binder::declare_host_module(ctx, name);
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
    fn scene_script_exports_match_the_installed_functions() {
        let runtime = Runtime::new().expect("test runtime");
        let context = rquickjs::Context::full(&runtime).expect("test context");
        context.with(|ctx| {
            ctx.eval::<(), _>("globalThis.core = globalThis.__scene = {}; globalThis.__abi = {};")
                .expect("test inputs");
            ctx.eval::<(), _>(bootstrap::SCENE_JS)
                .expect("scene script");
            let core = ctx
                .globals()
                .get::<_, Object>("core")
                .expect("scene functions");
            let mut actual = core
                .keys::<String>()
                .collect::<rquickjs::Result<Vec<_>>>()
                .expect("scene keys");
            actual.retain(|name| !name.starts_with('_'));
            actual.sort_unstable();
            assert_eq!(actual, SCENE_SCRIPT_NAMES);
        });
    }

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
                if (Object.keys(m{index}).sort().join(',') !== {:?}) throw Error({name:?});",
                names.join(","),
            )
            .expect("write source");
        }
        assert!(
            crate::JsSession::open(
                &source,
                "main.js",
                uicore::MonoShaper::default(),
                Box::new(|_, _| {})
            )
            .is_ok()
        );
    }
}
