//! Replayable changes that can execute against the live module graph.

use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
};

use oxc::{
    allocator::Allocator,
    ast::ast::{
        BindingPattern, Declaration, ExportDefaultDeclarationKind, Expression, Function, Program,
        Statement, VariableDeclaration, VariableDeclarationKind,
    },
    ast_visit::{Visit, walk},
    parser::Parser,
    semantic::Semantic,
    span::{GetSpan, SourceType, Span},
};
use rolldown_common::Output;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub(crate) const MARKER: &str = "// impasto:hot/1\n";
const STATE: &str = "\n// impasto:hot-state\n";
const STATE_END: &str = "\n// impasto:hot-state-end\n";
const PATCH: &str = "// impasto:hot-patch/1\n";

/// Module metadata computed before JSX is lowered.
#[derive(Debug, Default)]
pub(crate) struct ComponentModule {
    boundary: Option<String>,
    replay: Option<String>,
    components: BTreeMap<String, String>,
    functions: BTreeMap<String, String>,
    values: BTreeMap<String, Initializer>,
    default: Option<String>,
}

#[derive(Debug)]
enum Initializer {
    Component(String),
    Preserve,
    Pure(String),
}

pub(crate) fn components(
    program: &Program<'_>,
    source: &str,
    semantic: &Semantic<'_>,
) -> ComponentModule {
    let mut metadata = ComponentModule::default();
    let mut components = Vec::new();
    let mut safe = true;
    let mut helpers = BTreeMap::new();
    for statement in &program.body {
        match statement {
            Statement::ImportDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::TSGlobalDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_) => {}
            Statement::FunctionDeclaration(function) => {
                if !register(function, &mut components)
                    && let Some(id) = &function.id
                    && let Some(symbol) = id.symbol_id.get()
                {
                    helpers.insert(symbol, (id.name.to_string(), function.span));
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::FunctionDeclaration(function) =
                    &export.declaration
                {
                    register(function, &mut components);
                } else if let Some(expression) = export.declaration.as_expression()
                    && let Expression::ArrowFunctionExpression(arrow) =
                        expression.get_inner_expression()
                    && !arrow.r#async
                    && has_jsx(expression)
                {
                    components.push(("default".to_owned(), expression.span(), false));
                } else if !export
                    .declaration
                    .as_expression()
                    .is_some_and(pure_initializer)
                {
                    safe = false;
                }
            }
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::FunctionDeclaration(function) => {
                    register(function, &mut components);
                }
                Declaration::VariableDeclaration(declaration) => {
                    safe &= initializers(declaration, source, &mut components, &mut metadata);
                }
                Declaration::TSInterfaceDeclaration(_) | Declaration::TSTypeAliasDeclaration(_) => {
                }
                _ => safe = false,
            },
            Statement::VariableDeclaration(declaration) => {
                safe &= initializers(declaration, source, &mut components, &mut metadata);
                if safe && declaration.kind == VariableDeclarationKind::Const {
                    for item in &declaration.declarations {
                        let BindingPattern::BindingIdentifier(id) = &item.id else {
                            continue;
                        };
                        let Some(value) = &item.init else { continue };
                        if matches!(
                            value.get_inner_expression(),
                            Expression::ArrowFunctionExpression(_)
                                | Expression::FunctionExpression(_)
                        ) && !components
                            .iter()
                            .any(|(name, _, _)| name == id.name.as_str())
                            && let Some(symbol) = id.symbol_id.get()
                        {
                            helpers.insert(symbol, (id.name.to_string(), value.span()));
                        }
                    }
                }
            }
            _ => safe = false,
        }
    }
    let references = |symbol| {
        semantic
            .scoping()
            .get_resolved_references(symbol)
            .filter(|reference| !reference.is_type())
            .map(|reference| semantic.nodes().get_node(reference.node_id()).span())
    };
    loop {
        let escaped = helpers
            .keys()
            .copied()
            .filter(|symbol| {
                references(*symbol).any(|reference| {
                    !components
                        .iter()
                        .any(|(_, span, _)| covers(*span, reference))
                        && !helpers.values().any(|(_, span)| covers(*span, reference))
                })
            })
            .collect::<Vec<_>>();
        if escaped.is_empty() {
            break;
        }
        for symbol in escaped {
            helpers.remove(&symbol);
        }
    }
    let callers = helpers
        .iter()
        .map(|(&symbol, (name, span))| (name, *span, references(symbol).collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    let mut ranges = components
        .iter()
        .map(|(name, span, _)| (name, *span))
        .chain(helpers.values().map(|(name, span)| (name, *span)))
        .collect::<Vec<_>>();
    ranges.sort_unstable_by_key(|(_, span)| span.start);
    let mut rest = DefaultHasher::new();
    let mut start = 0;
    for (name, span) in ranges {
        source.get(start..span.start as usize).hash(&mut rest);
        name.hash(&mut rest);
        start = span.end as usize;
    }
    for (name, span, is_function) in components {
        let mut body = DefaultHasher::new();
        source
            .get(span.start as usize..span.end as usize)
            .hash(&mut body);
        let mut queue = vec![span];
        let mut dependencies = BTreeMap::new();
        while let Some(caller) = queue.pop() {
            for (name, span, references) in &callers {
                if !dependencies.contains_key(*name)
                    && references
                        .iter()
                        .any(|reference| covers(caller, *reference))
                {
                    dependencies.insert(*name, *span);
                    queue.push(*span);
                }
            }
        }
        for (name, span) in dependencies {
            name.hash(&mut body);
            span.source_text(source).hash(&mut body);
        }
        let signature = format!("{:016x}", body.finish());
        metadata.components.insert(name.clone(), signature.clone());
        if name == "default" {
            metadata.default = Some(signature);
        } else if is_function {
            metadata.functions.insert(name, signature);
        } else {
            metadata
                .values
                .insert(name, Initializer::Component(signature));
        }
    }
    source.get(start..).hash(&mut rest);
    metadata.replay = safe.then(|| {
        let mut effects = DefaultHasher::new();
        for statement in &program.body {
            let declaration = match statement {
                Statement::VariableDeclaration(declaration) => declaration,
                Statement::ExportDeclaration(export) => match &export.declaration {
                    Declaration::VariableDeclaration(declaration) => declaration,
                    _ => continue,
                },
                _ => continue,
            };
            for item in &declaration.declarations {
                if matches!(
                    metadata.values.get(&binding_names(item)),
                    Some(Initializer::Preserve)
                ) {
                    item.span.source_text(source).hash(&mut effects);
                }
            }
        }
        if metadata
            .values
            .values()
            .any(|value| matches!(value, Initializer::Preserve))
        {
            format!("{:016x}", effects.finish())
        } else {
            String::new()
        }
    });
    metadata.boundary = (safe
        && (metadata.default.is_some()
            || !metadata.functions.is_empty()
            || metadata
                .values
                .values()
                .any(|value| matches!(value, Initializer::Component(_)))))
    .then(|| format!("{:016x}", rest.finish()));
    metadata
}

fn covers(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn has_jsx(expression: &Expression<'_>) -> bool {
    let mut jsx = ContainsJsx(false);
    match expression.get_inner_expression() {
        Expression::ArrowFunctionExpression(function) => {
            if let Some(body) = function.body.as_expression() {
                jsx.visit_expression(body);
            } else if let Some(body) = function.body.as_function_body() {
                jsx.visit_function_body(body);
            }
        }
        Expression::FunctionExpression(function) => {
            if let Some(body) = &function.body {
                jsx.visit_function_body(body);
            }
        }
        _ => {}
    }
    jsx.0
}

fn binding_names(item: &oxc::ast::ast::VariableDeclarator<'_>) -> String {
    item.id
        .get_binding_identifiers()
        .into_iter()
        .map(|id| id.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn initializers(
    declaration: &VariableDeclaration<'_>,
    source: &str,
    components: &mut Vec<(String, Span, bool)>,
    metadata: &mut ComponentModule,
) -> bool {
    let mut safe = declaration.kind == VariableDeclarationKind::Const;
    for item in &declaration.declarations {
        let Some(value) = &item.init else {
            safe = false;
            continue;
        };
        let key = binding_names(item);
        if has_jsx(value) && item.id.is_binding_identifier() && component_name(&key) {
            components.push((key, value.span(), false));
            continue;
        }
        if matches!(
            value,
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)
        ) {
            continue;
        }
        let mut await_expression = ContainsAwait(false);
        await_expression.visit_expression(value);
        if key.is_empty() || await_expression.0 {
            safe = false;
        } else {
            let kind = if pure_initializer(value) {
                let mut hash = DefaultHasher::new();
                value.span().source_text(source).hash(&mut hash);
                Initializer::Pure(format!("{:016x}", hash.finish()))
            } else {
                Initializer::Preserve
            };
            metadata.values.insert(key, kind);
        }
    }
    safe
}

fn pure_initializer(expression: &Expression<'_>) -> bool {
    use oxc::ast::ast::{ArrayExpressionElement, ObjectPropertyKind, PropertyKind};

    match expression.get_inner_expression() {
        Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::RegExpLiteral(_) => true,
        Expression::ArrayExpression(array) => array.elements.iter().all(|element| match element {
            ArrayExpressionElement::Elision(_) => true,
            _ => element.as_expression().is_some_and(pure_initializer),
        }),
        Expression::ObjectExpression(object) => object.properties.iter().all(|property| {
            matches!(property, ObjectPropertyKind::ObjectProperty(property)
                if !property.computed && property.kind == PropertyKind::Init
                    && pure_initializer(&property.value))
        }),
        _ => false,
    }
}

fn component_name(name: &str) -> bool {
    name.starts_with(|ch: char| ch.is_ascii_uppercase())
}

fn register(function: &Function<'_>, components: &mut Vec<(String, Span, bool)>) -> bool {
    let Some(id) = &function.id else { return false };
    if !component_name(&id.name) || function.r#async || function.generator {
        return false;
    }
    let mut jsx = ContainsJsx(false);
    if let Some(body) = &function.body {
        jsx.visit_function_body(body);
    }
    if jsx.0 {
        components.push((id.name.to_string(), function.span(), true));
    }
    jsx.0
}

struct ContainsJsx(bool);
impl<'a> Visit<'a> for ContainsJsx {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        match expression {
            Expression::JSXElement(_) | Expression::JSXFragment(_) => self.0 = true,
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {}
            _ if !self.0 => walk::walk_expression(self, expression),
            _ => {}
        }
    }
}

struct ContainsAwait(bool);
impl<'a> Visit<'a> for ContainsAwait {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        match expression {
            Expression::AwaitExpression(_) => self.0 = true,
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {}
            _ if !self.0 => walk::walk_expression(self, expression),
            _ => {}
        }
    }
}

impl ComponentModule {
    pub(crate) fn instrument(
        &self,
        id: &str,
        code: &mut String,
        map: &mut oxc_sourcemap::SourceMap<'static>,
    ) {
        if self.replay.is_none() {
            return;
        }
        if !self.values.is_empty() || self.default.is_some() {
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
            let mut edits = Vec::new();
            for statement in &parsed.program.body {
                let declaration = match statement {
                    Statement::ExportDefaultDeclaration(export) => {
                        if let Some(signature) = &self.default
                            && let Some(expression) = export.declaration.as_expression()
                        {
                            let span = expression.span();
                            edits.push((span, format!(
                                "globalThis.__impasto_hot.component({id:?}, \"default\", {signature:?}, ("
                            )));
                        }
                        continue;
                    }
                    Statement::VariableDeclaration(declaration) => declaration,
                    Statement::ExportDeclaration(export) => match &export.declaration {
                        Declaration::VariableDeclaration(declaration) => declaration,
                        _ => continue,
                    },
                    _ => continue,
                };
                for item in &declaration.declarations {
                    let Some(value) = &item.init else { continue };
                    let name = binding_names(item);
                    let span = value.span();
                    let prefix = match self.values.get(&name) {
                        Some(Initializer::Component(signature)) => format!(
                            "globalThis.__impasto_hot.component({id:?}, {name:?}, {signature:?}, ("
                        ),
                        Some(Initializer::Preserve) => {
                            format!("globalThis.__impasto_hot.keep({id:?}, {name:?}, null, () => (")
                        }
                        Some(Initializer::Pure(signature)) => {
                            format!(
                                "globalThis.__impasto_hot.keep({id:?}, {name:?}, {signature:?}, () => ("
                            )
                        }
                        None => continue,
                    };
                    edits.push((span, prefix));
                }
            }
            shift_map(code, map, &edits);
            for (span, prefix) in edits.into_iter().rev() {
                code.insert_str(span.end as usize, "))");
                code.insert_str(span.start as usize, &prefix);
            }
        }
        for (name, signature) in &self.functions {
            use std::fmt::Write as _;
            let _ = write!(
                code,
                "\n{name} = globalThis.__impasto_hot.component({id:?}, {name:?}, {signature:?}, {name});"
            );
        }
    }
}

fn shift_map(code: &str, map: &mut oxc_sourcemap::SourceMap<'static>, edits: &[(Span, String)]) {
    use oxc_sourcemap::{SourceMap, Token};
    if edits.is_empty() {
        return;
    }
    let starts = std::iter::once(0usize)
        .chain(code.match_indices('\n').map(|(index, _)| index + 1))
        .collect::<Vec<_>>();
    let mut offsets = BTreeMap::<u32, Vec<(u32, u32)>>::new();
    for (span, prefix) in edits {
        for (position, width) in [
            (span.start as usize, prefix.encode_utf16().count()),
            (span.end as usize, 2),
        ] {
            let line = starts.partition_point(|start| *start <= position) - 1;
            let column = code[starts[line]..position].encode_utf16().count();
            if let (Ok(line), Ok(column), Ok(width)) = (
                u32::try_from(line),
                u32::try_from(column),
                u32::try_from(width),
            ) {
                offsets.entry(line).or_default().push((column, width));
            }
        }
    }
    let mut parts = std::mem::take(map).into_parts();
    for token in &mut parts.tokens {
        let line = token.get_dst_line();
        let col = token.get_dst_col();
        if let Some(offsets) = offsets.get(&line) {
            let shift: u32 = offsets
                .iter()
                .filter(|(at, _)| *at <= col)
                .map(|(_, width)| width)
                .sum();
            *token = Token::new(
                line,
                col.saturating_add(shift),
                token.get_src_line(),
                token.get_src_col(),
                token.get_source_id(),
                token.get_name_id(),
            );
        }
    }
    *map = SourceMap::from_parts(parts);
}

#[derive(Serialize)]
struct FactoryMap {
    sources: Vec<String>,
    mappings: String,
}

#[derive(Serialize)]
struct Factory<'a> {
    body: &'a str,
    location: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    map: Option<FactoryMap>,
    boundary: Option<&'a str>,
    replay: Option<&'a str>,
    components: Option<&'a BTreeMap<String, String>>,
    dependencies: BTreeMap<&'a str, &'a str>,
}

#[derive(Serialize)]
struct State<'a> {
    entry: &'a str,
    version: String,
    natives: &'a BTreeSet<String>,
    factories: BTreeMap<&'a str, Factory<'a>>,
}

/// Assemble CommonJS chunks into a single ESM entry for QuickJS.
pub(crate) fn script(
    output: &[Output],
    metadata: &BTreeMap<String, ComponentModule>,
    externals: &BTreeSet<String>,
) -> anyhow::Result<String> {
    use std::fmt::Write as _;
    let mut code = MARKER.to_owned();
    code.push_str("import { createMemo, createSignal, untrack } from 'reactive';\n");
    code.push_str("import { compileHotFactory } from 'scene';\n");
    let mut native = String::new();
    for (index, name) in externals.iter().enumerate() {
        writeln!(
            code,
            "import * as native{index} from {};",
            serde_json::to_string(name)?
        )?;
        write!(
            native,
            "[{}]: {{ ...native{index}, __esModule: true }}, ",
            serde_json::to_string(name)?
        )?;
    }
    let mut entry = None;
    let mut factories = BTreeMap::new();
    for asset in output {
        let Output::Chunk(chunk) = asset else {
            continue;
        };
        let filename = chunk.filename.as_str();
        if chunk.is_entry {
            entry = Some(filename);
        }
        let module = chunk.module_ids.first().map(|module| module.as_str());
        let transformed = module.and_then(|module| metadata.get(module));
        let generated = module.is_some_and(|id| {
            id.strip_prefix('\0') == Some(crate::i18n::MODULE)
                || std::path::Path::new(id)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(extension, "png" | "jpg" | "jpeg" | "webp" | "wgsl" | "svg")
                    })
        });
        let dependencies = chunk
            .imports
            .iter()
            .chain(&chunk.dynamic_imports)
            .filter_map(|import| {
                let import = import.as_str();
                output
                    .iter()
                    .find_map(|asset| match asset {
                        Output::Chunk(chunk) if chunk.filename.as_str() == import => {
                            Some(chunk.filename.as_str())
                        }
                        _ => None,
                    })
                    .map(|id| (import, id))
            })
            .collect();
        let mut location = DefaultHasher::new();
        filename.hash(&mut location);
        chunk.code.hash(&mut location);
        let location = format!("impasto-hot-{:016x}", location.finish());
        factories.insert(
            filename,
            Factory {
                body: &chunk.code,
                location,
                map: chunk.map.as_ref().map(|map| {
                    let mut map = map.to_json();
                    if let [source] = map.sources.as_mut_slice()
                        && let [module] = chunk.module_ids.as_slice()
                        && std::path::Path::new(module.as_str()).is_absolute()
                    {
                        *source = module.as_str().to_owned();
                    }
                    FactoryMap {
                        sources: map.sources,
                        mappings: map.mappings,
                    }
                }),
                boundary: transformed.and_then(|item| item.boundary.as_deref()),
                replay: transformed
                    .and_then(|item| item.replay.as_deref())
                    .or(generated.then_some("")),
                components: transformed.map(|item| &item.components),
                dependencies,
            },
        );
    }
    let entry = entry.ok_or_else(|| anyhow::anyhow!("watch bundle has no entry chunk"))?;
    let mut version = DefaultHasher::new();
    entry.hash(&mut version);
    externals.hash(&mut version);
    for (id, factory) in &factories {
        id.hash(&mut version);
        factory.body.hash(&mut version);
        factory
            .map
            .as_ref()
            .map(|map| (&map.sources, &map.mappings))
            .hash(&mut version);
        factory.boundary.hash(&mut version);
        factory.replay.hash(&mut version);
        factory.dependencies.hash(&mut version);
    }
    code.push_str(include_str!("hot.js"));
    write!(
        code,
        "\nconst natives = {{ {native} }};\nconst state = {STATE}{}{STATE_END};\nstart(state, natives);\n",
        serde_json::to_string(&State {
            entry,
            version: format!("{:016x}", version.finish()),
            natives: externals,
            factories,
        })?
    )?;
    Ok(code)
}

/// A development update and the reason it can or cannot reuse the live VM.
#[derive(Debug)]
pub enum DeltaOutcome {
    Patch {
        code: String,
        changed_modules: Vec<String>,
        updated_components: Vec<String>,
    },
    FullReload {
        reason: ReloadReason,
    },
}

/// The compatibility check that required a new script session.
#[derive(Debug)]
pub enum ReloadReason {
    EntryModuleChanged(String),
    TopLevelSideEffectChanged {
        module: String,
        detail: &'static str,
    },
    NativeExportsChanged(Vec<String>),
    SessionNotHotCapable,
    RuntimePatchRejected(String),
}

impl std::fmt::Display for ReloadReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EntryModuleChanged(module) => write!(formatter, "entry module {module} changed"),
            Self::TopLevelSideEffectChanged { module, detail } => {
                write!(formatter, "{module}: {detail}")
            }
            Self::NativeExportsChanged(modules) => {
                write!(formatter, "native imports changed: {}", modules.join(", "))
            }
            Self::SessionNotHotCapable => formatter.write_str("live bundle version is unavailable"),
            Self::RuntimePatchRejected(error) => {
                write!(
                    formatter,
                    "runtime rejected patch: {}",
                    error.lines().next().unwrap_or_default()
                )
            }
        }
    }
}

/// Compare two development bundles and emit the minimum compatible graph update.
pub fn delta(previous: &str, next: &str) -> DeltaOutcome {
    match plan_delta(previous, next) {
        Ok((code, changed_modules, updated_components)) => DeltaOutcome::Patch {
            code,
            changed_modules,
            updated_components,
        },
        Err(reason) => DeltaOutcome::FullReload { reason },
    }
}

fn plan_delta(
    previous: &str,
    next: &str,
) -> Result<(String, Vec<String>, Vec<String>), ReloadReason> {
    let incompatible = || ReloadReason::SessionNotHotCapable;
    let previous = read_state(previous).ok_or_else(incompatible)?;
    let next = read_state(next).ok_or_else(incompatible)?;
    if previous.entry != next.entry {
        return Err(ReloadReason::EntryModuleChanged(next.entry.to_owned()));
    }
    if previous.natives != next.natives {
        return Err(ReloadReason::NativeExportsChanged(
            previous
                .natives
                .symmetric_difference(&next.natives)
                .cloned()
                .collect(),
        ));
    }

    let before = read_factories(&previous.factories)?;
    let after = read_factories(&next.factories)?;
    let ids = before.keys().chain(after.keys()).collect::<BTreeSet<_>>();
    let mut factories = BTreeMap::new();
    let mut removed = Vec::new();
    let mut changed_modules = Vec::new();
    let mut replay = BTreeMap::new();
    let mut importers = BTreeMap::<&str, Vec<&str>>::new();
    for (id, factory) in &after {
        for dependency in factory.dependencies.values() {
            importers.entry(dependency).or_default().push(id);
        }
    }
    let mut queue = Vec::new();
    for id in ids {
        let original = previous.factories.get(id).map(|raw| raw.get());
        let replacement = next.factories.get(id).copied();
        if original == replacement.map(RawValue::get) {
            continue;
        }
        if let Some(replacement) = replacement {
            factories.insert(id.as_str(), replacement);
        } else {
            removed.push(id.as_str());
        }
        let prior = before.get(id);
        let updated = after.get(id);
        if matches!((prior, updated), (Some(prior), Some(updated)) if
            prior.body.get() == updated.body.get() && prior.dependencies == updated.dependencies)
        {
            continue;
        }

        changed_modules.push(id.clone());
        if id == next.entry {
            return Err(ReloadReason::EntryModuleChanged(id.clone()));
        }
        let compatible = match (prior, updated) {
            (Some(prior), Some(updated)) => {
                prior.replay.is_some() && prior.replay == updated.replay
            }
            (None, Some(updated)) => updated.replay == Some(""),
            (Some(_), None) => true,
            (None, None) => false,
        };
        if !compatible {
            return Err(side_effect(id));
        }
        if let Some(updated) = updated {
            let force = prior.is_none_or(|prior| {
                prior.boundary != updated.boundary || prior.dependencies != updated.dependencies
            });
            replay.insert(id.as_str(), force);
            if updated.boundary.is_none() {
                queue.push(id.as_str());
            }
        }
    }

    while let Some(id) = queue.pop() {
        for importer in importers.get(id).into_iter().flatten().copied() {
            // An added module is discovered through the import that changed.
            if !before.contains_key(id) && replay.contains_key(importer) {
                continue;
            }
            if importer == next.entry {
                return Err(ReloadReason::EntryModuleChanged(importer.to_owned()));
            }
            let factory = &after[importer];
            if factory.replay.is_none()
                || (id.ends_with(".wgsl")
                    && factory.replay.is_some_and(|effects| !effects.is_empty()))
            {
                // A top-level shader handle retains the registered program.
                // A candidate VM creates a new one from the changed source.
                return Err(side_effect(importer));
            }
            let seen = replay.insert(importer, true).is_some();
            if !seen && factory.boundary.is_none() {
                queue.push(importer);
            }
        }
    }
    let mut updated = BTreeSet::new();
    for (id, force) in &replay {
        let Some(components) = &after[*id].components else {
            continue;
        };
        let prior = before
            .get(*id)
            .and_then(|factory| factory.components.as_ref());
        for (name, signature) in components {
            if *force || prior.and_then(|names| names.get(name)) != Some(signature) {
                updated.insert(name.clone());
            }
        }
    }
    let updated_components = updated.into_iter().collect();
    let patch = serde_json::to_string(&Patch {
        from: previous.version,
        to: next.version,
        factories,
        removed,
        replay,
    })
    .map_err(|_| incompatible())?;
    Ok((
        format!("{PATCH}{patch}"),
        changed_modules,
        updated_components,
    ))
}

fn read_factories<'a>(
    factories: &BTreeMap<String, &'a RawValue>,
) -> Result<BTreeMap<String, FactoryView<'a>>, ReloadReason> {
    factories
        .iter()
        .map(|(id, raw)| {
            let factory =
                serde_json::from_str(raw.get()).map_err(|_| ReloadReason::SessionNotHotCapable)?;
            Ok((id.clone(), factory))
        })
        .collect()
}

fn side_effect(module: &str) -> ReloadReason {
    ReloadReason::TopLevelSideEffectChanged {
        module: module.to_owned(),
        detail: "top-level code cannot be replayed safely",
    }
}

#[derive(Deserialize)]
struct StateView<'a> {
    entry: &'a str,
    version: &'a str,
    natives: BTreeSet<String>,
    #[serde(borrow)]
    factories: BTreeMap<String, &'a RawValue>,
}

#[derive(Deserialize)]
struct FactoryView<'a> {
    body: &'a RawValue,
    boundary: Option<&'a str>,
    replay: Option<&'a str>,
    components: Option<BTreeMap<String, String>>,
    dependencies: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct Patch<'a> {
    from: &'a str,
    to: &'a str,
    factories: BTreeMap<&'a str, &'a RawValue>,
    removed: Vec<&'a str>,
    replay: BTreeMap<&'a str, bool>,
}

fn read_state(source: &str) -> Option<StateView<'_>> {
    let (_, state) = source.strip_prefix(MARKER)?.split_once(STATE)?;
    let (state, _) = state.split_once(STATE_END)?;
    serde_json::from_str(state).ok()
}
