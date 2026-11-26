//! Component-only changes that can execute against the live module graph.

use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
};

use oxc::{
    allocator::Allocator,
    ast::ast::{
        Declaration, ExportDefaultDeclarationKind, Expression, Function, Program, Statement,
        VariableDeclaration, VariableDeclarationKind,
    },
    ast_visit::{Visit, walk},
    parser::Parser,
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
    functions: BTreeMap<String, String>,
    values: BTreeMap<String, Initializer>,
}

#[derive(Debug)]
enum Initializer {
    Component(String),
    Preserve,
}

pub(crate) fn components(program: &Program<'_>, source: &str) -> ComponentModule {
    let mut metadata = ComponentModule::default();
    let mut components = Vec::new();
    let mut safe = true;
    for statement in &program.body {
        match statement {
            Statement::ImportDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_) => {}
            Statement::FunctionDeclaration(function) => register(function, &mut components),
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::FunctionDeclaration(function) =
                    &export.declaration
                {
                    register(function, &mut components);
                } else {
                    safe = false;
                }
            }
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::FunctionDeclaration(function) => register(function, &mut components),
                Declaration::VariableDeclaration(declaration) => {
                    safe &= initializers(declaration, &mut components, &mut metadata);
                }
                Declaration::TSInterfaceDeclaration(_) | Declaration::TSTypeAliasDeclaration(_) => {
                }
                _ => safe = false,
            },
            Statement::VariableDeclaration(declaration) => {
                safe &= initializers(declaration, &mut components, &mut metadata);
            }
            _ => safe = false,
        }
    }
    let mut rest = DefaultHasher::new();
    let mut start = 0;
    for (name, span, is_function) in components {
        source.get(start..span.start as usize).hash(&mut rest);
        name.hash(&mut rest);
        start = span.end as usize;
        let mut body = DefaultHasher::new();
        source.get(span.start as usize..start).hash(&mut body);
        let signature = format!("{:016x}", body.finish());
        if is_function {
            metadata.functions.insert(name, signature);
        } else {
            metadata
                .values
                .insert(name, Initializer::Component(signature));
        }
    }
    source.get(start..).hash(&mut rest);
    metadata.boundary = (safe
        && (!metadata.functions.is_empty()
            || metadata
                .values
                .values()
                .any(|value| matches!(value, Initializer::Component(_)))))
    .then(|| format!("{:016x}", rest.finish()));
    metadata
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
        let mut jsx = ContainsJsx(false);
        match value.without_parentheses() {
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
        if jsx.0 && item.id.is_binding_identifier() && component_name(&key) {
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
            // A compatible edit has the same initializer source. Keep the first
            // value so registrations and module-level state survive a patch.
            metadata.values.insert(key, Initializer::Preserve);
        }
    }
    safe
}

fn component_name(name: &str) -> bool {
    name.starts_with(|ch: char| ch.is_ascii_uppercase())
}

fn register(function: &Function<'_>, components: &mut Vec<(String, Span, bool)>) {
    let Some(id) = &function.id else { return };
    if !component_name(&id.name) || function.r#async || function.generator {
        return;
    }
    let mut jsx = ContainsJsx(false);
    if let Some(body) = &function.body {
        jsx.visit_function_body(body);
    }
    if jsx.0 {
        components.push((id.name.to_string(), function.span(), true));
    }
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
    pub(crate) fn instrument(&self, id: &str, code: &mut String) {
        if self.boundary.is_none() {
            return;
        }
        if !self.values.is_empty() {
            let allocator = Allocator::default();
            let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
            let mut edits = Vec::new();
            for statement in &parsed.program.body {
                let declaration = match statement {
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
                    let expression = span.source_text(code);
                    let replacement = match self.values.get(&name) {
                        Some(Initializer::Component(signature)) => format!(
                            "globalThis.__impasto_hot.component({id:?}, {name:?}, {signature:?}, ({expression}))"
                        ),
                        Some(Initializer::Preserve) => format!(
                            "globalThis.__impasto_hot.keep({id:?}, {name:?}, () => ({expression}))"
                        ),
                        None => continue,
                    };
                    edits.push((span, replacement));
                }
            }
            for (span, replacement) in edits.into_iter().rev() {
                code.replace_range(span.start as usize..span.end as usize, &replacement);
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

#[derive(Serialize)]
struct Factory<'a> {
    body: &'a str,
    boundary: Option<&'a str>,
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
        let boundary = chunk
            .module_ids
            .first()
            .and_then(|module| metadata.get(module.as_str()))
            .and_then(|item| item.boundary.as_deref());
        let dependencies = chunk
            .imports
            .iter()
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
        factories.insert(
            filename,
            Factory {
                body: &chunk.code,
                boundary,
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
        factory.boundary.hash(&mut version);
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

/// Returns a component patch for a compatible development bundle pair.
///
/// The patch requires the exact version of the previous module graph.
pub fn delta(previous: &str, next: &str) -> Option<String> {
    let previous = read_state(previous)?;
    let next = read_state(next)?;
    if previous.entry != next.entry || previous.natives.get() != next.natives.get() {
        return None;
    }
    if previous.factories.len() != next.factories.len() {
        return None;
    }
    let mut changed = BTreeMap::new();
    for (name, replacement) in next.factories {
        let original = previous.factories.get(&name)?;
        if original.get() != replacement.get() {
            let boundary = serde_json::from_str::<Boundary>(original.get())
                .ok()?
                .boundary?;
            let updated = serde_json::from_str::<Boundary>(replacement.get())
                .ok()?
                .boundary?;
            if name == next.entry || boundary != updated {
                return None;
            }
            changed.insert(name, replacement);
        }
    }
    let patch = serde_json::to_string(&Patch {
        from: previous.version,
        to: next.version,
        factories: changed,
    })
    .ok()?;
    Some(format!("{PATCH}{patch}"))
}

#[derive(Deserialize)]
struct StateView<'a> {
    entry: &'a str,
    version: &'a str,
    natives: &'a RawValue,
    #[serde(borrow)]
    factories: BTreeMap<String, &'a RawValue>,
}

#[derive(Deserialize)]
struct Boundary<'a> {
    boundary: Option<&'a str>,
}

#[derive(Serialize)]
struct Patch<'a> {
    from: &'a str,
    to: &'a str,
    factories: BTreeMap<String, &'a RawValue>,
}

fn read_state(source: &str) -> Option<StateView<'_>> {
    let (_, state) = source.strip_prefix(MARKER)?.split_once(STATE)?;
    let (state, _) = state.split_once(STATE_END)?;
    serde_json::from_str(state).ok()
}
