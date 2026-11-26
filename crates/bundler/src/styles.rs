//! Evaluate only literals whose values are known without executing app code.

use std::collections::{HashMap, HashSet};

use engine::style::codec::Value;
use oxc::ast::ast::{
    ArrayExpressionElement, BindingPattern, Expression, ImportDeclarationSpecifier,
    JSXAttributeName, JSXElementName, ObjectPropertyKind, PropertyKind, UnaryOperator,
    VariableDeclarationKind,
};
use oxc::{
    ast::AstKind,
    semantic::Semantic,
    span::{GetSpan, Span},
    syntax::symbol::SymbolId,
};
use serde_json::{Map, Number, Value as Json};

#[derive(Default)]
pub(super) struct Constants {
    reads: HashMap<Span, (Json, String)>,
    themes: HashSet<Span>,
    #[cfg(feature = "bundle")]
    pub(super) theme_tokens: Option<crate::ThemeTokens>,
}

impl Constants {
    #[cfg(feature = "bundle")]
    pub(super) fn with_theme_tokens(mut self, theme: Option<&crate::ThemeTokens>) -> Self {
        self.theme_tokens = theme.cloned();
        self
    }

    pub(super) fn analyze(semantic: &Semantic<'_>, mut values: HashMap<SymbolId, Json>) -> Self {
        let nodes = semantic.nodes();
        let scoping = semantic.scoping();
        let mut theme_hooks = HashSet::new();
        let mut themes = HashSet::new();
        for node in nodes.iter() {
            if let AstKind::ImportDeclaration(import) = node.kind()
                && import.source.value == "@impasto/solid-ui"
                && !import.import_kind.is_type()
            {
                for item in import.specifiers.iter().flatten() {
                    if let ImportDeclarationSpecifier::ImportSpecifier(specifier) = item
                        && !specifier.import_kind.is_type()
                        && specifier.imported.name() == "useTheme"
                        && let Some(symbol) = specifier.local.symbol_id.get()
                    {
                        theme_hooks.insert(symbol);
                    }
                }
            }
        }
        for node in nodes.iter() {
            let AstKind::VariableDeclarator(declaration) = node.kind() else {
                continue;
            };
            if !matches!(nodes.parent_kind(node.id()), AstKind::VariableDeclaration(variable)
                if variable.kind == VariableDeclarationKind::Const)
            {
                continue;
            }
            let BindingPattern::BindingIdentifier(binding) = &declaration.id else {
                continue;
            };
            let Some(symbol) = binding.symbol_id.get() else {
                continue;
            };
            if let Some(Expression::CallExpression(call)) = declaration
                .init
                .as_ref()
                .map(Expression::get_inner_expression)
                && call.arguments.is_empty()
                && !call.optional
                && let Expression::Identifier(hook) = call.callee.get_inner_expression()
                && let Some(reference) = hook.reference_id.get()
                && scoping
                    .get_reference(reference)
                    .symbol_id()
                    .is_some_and(|symbol| theme_hooks.contains(&symbol))
            {
                themes.extend(
                    scoping
                        .get_resolved_references(symbol)
                        .map(|reference| nodes.get_node(reference.node_id()).span())
                        .filter(|span| span.start > binding.span.start),
                );
            }
            let Some(value) = declaration.init.as_ref().and_then(literal) else {
                continue;
            };
            if value.is_object() || value.is_array() {
                if nodes.ancestor_kinds(node.id()).any(|kind| matches!(kind, AstKind::ExportDeclaration(_)))
                    || !scoping.get_resolved_references(symbol).all(|reference| {
                        let parent = nodes.parent_node(reference.node_id());
                        match parent.kind() {
                            AstKind::StaticMemberExpression(member) => value
                                .get(member.property.name.as_str())
                                .is_some_and(|value| !value.is_object() && !value.is_array())
                                && !nodes.ancestor_kinds(parent.id()).any(|kind| match kind {
                                    AstKind::AssignmentExpression(assignment) => {
                                        let target = assignment.left.span();
                                        target.start <= member.span.start && member.span.end <= target.end
                                    }
                                    AstKind::UpdateExpression(_) |
                                    AstKind::ForInStatement(_) |
                                    AstKind::ForOfStatement(_) => true,
                                    AstKind::UnaryExpression(unary) => unary.operator == UnaryOperator::Delete,
                                    _ => false,
                                }),
                            AstKind::JSXExpressionContainer(_) => {
                                let slot = nodes.parent_node(parent.id());
                                let opening = nodes.parent_node(slot.id());
                                matches!(slot.kind(), AstKind::JSXAttribute(attr)
                                    if matches!(&attr.name, JSXAttributeName::Identifier(name) if name.name == "style"))
                                    && matches!(opening.kind(), AstKind::JSXOpeningElement(element)
                                        if matches!(&element.name, JSXElementName::IdentifierReference(id)
                                            if crate::jsx::style_primitive(semantic, opening.scope_id(), &id.name)))
                            }
                            _ => false,
                        }
                    })
                {
                    continue;
                }
            } else if scoping
                .get_resolved_references(symbol)
                .any(oxc::syntax::reference::Reference::is_write)
            {
                continue;
            }
            values.insert(symbol, value);
        }

        let mut reads = HashMap::new();
        for (symbol, value) in values {
            let name = scoping.symbol_name(symbol);
            for reference in scoping.get_resolved_references(symbol) {
                let node = nodes.get_node(reference.node_id());
                if node.span().start <= scoping.symbol_span(symbol).start {
                    continue;
                }
                reads.insert(node.span(), (value.clone(), name.to_owned()));
            }
        }
        Self {
            reads,
            themes,
            #[cfg(feature = "bundle")]
            theme_tokens: None,
        }
    }

    pub(super) fn validate_tokens(&self, style: &Json) -> Result<(), String> {
        #[cfg(feature = "bundle")]
        if let Some(tokens) = &self.theme_tokens {
            use engine::style::Environment;
            let fragments = style
                .as_array()
                .map_or_else(|| std::slice::from_ref(style), Vec::as_slice);
            for fragment in fragments.iter().filter_map(Json::as_object) {
                for (property, value) in fragment {
                    let Some(name) = value.get("token").and_then(Json::as_str) else {
                        continue;
                    };
                    let known = if color_property(property) {
                        Environment::is_builtin_color(name) || tokens.colors.contains(name)
                    } else {
                        Environment::is_builtin_length(name) || tokens.lengths.contains(name)
                    };
                    if !known {
                        return Err(format!("unknown theme token {name:?} for {property}"));
                    }
                }
            }
        }
        let _ = (self, style);
        Ok(())
    }

    pub(super) fn theme_token(&self, property: &str, expression: &Expression<'_>) -> Option<Json> {
        use engine::style::{Environment, codec};

        let Expression::StaticMemberExpression(member) = expression.get_inner_expression() else {
            return None;
        };
        let Expression::StaticMemberExpression(group) = member.object.get_inner_expression() else {
            return None;
        };
        let Expression::CallExpression(read) = group.object.get_inner_expression() else {
            return None;
        };
        let Expression::Identifier(binding) = read.callee.get_inner_expression() else {
            return None;
        };
        if read.optional
            || group.optional
            || member.optional
            || !read.arguments.is_empty()
            || !self.themes.contains(&binding.span)
        {
            return None;
        }
        let is_color = color_property(property);
        let key = member.property.name.as_str();
        let name = match group.property.name.as_str() {
            "color" if is_color && Environment::is_builtin_color(key) => key.to_owned(),
            prefix @ ("space" | "shape" | "radius" | "fontSize") if !is_color => {
                let name = format!("{prefix}.{key}");
                Environment::is_builtin_length(&name).then_some(name)?
            }
            _ => return None,
        };
        let token = serde_json::json!({ "token": name });
        codec::declarations(property, &style_value(&token)).ok()?;
        Some(token)
    }

    pub(super) fn literal(
        &self,
        expression: &Expression<'_>,
        guards: &mut Vec<String>,
    ) -> Option<Json> {
        let checkpoint = guards.len();
        let result = evaluate(expression, self, guards);
        if result.is_none() {
            guards.truncate(checkpoint);
        }
        result
    }
}

fn color_property(name: &str) -> bool {
    matches!(
        name,
        "color" | "backgroundColor" | "borderColor" | "outlineColor"
    )
}

pub(super) fn literal(expression: &Expression<'_>) -> Option<Json> {
    evaluate(expression, &Constants::default(), &mut Vec::new())
}

fn evaluate(
    expression: &Expression<'_>,
    constants: &Constants,
    guards: &mut Vec<String>,
) -> Option<Json> {
    let expression = expression.get_inner_expression();
    Some(match expression {
        Expression::Identifier(identifier) => {
            let (value, name) = constants.reads.get(&identifier.span)?;
            if !guards.contains(name) {
                guards.push(name.clone());
            }
            value.clone()
        }
        Expression::StaticMemberExpression(member) => evaluate(&member.object, constants, guards)?
            .get(member.property.name.as_str())?
            .clone(),
        Expression::StringLiteral(literal) => Json::String(literal.value.to_string()),
        Expression::NumericLiteral(literal) => Json::Number(Number::from_f64(literal.value)?),
        Expression::BooleanLiteral(literal) => Json::Bool(literal.value),
        Expression::NullLiteral(_) => Json::Null,
        Expression::TemplateLiteral(literal) if literal.expressions.is_empty() => {
            Json::String(literal.quasis.first()?.value.cooked?.to_string())
        }
        Expression::UnaryExpression(literal)
            if literal.operator == UnaryOperator::UnaryNegation =>
        {
            Json::Number(Number::from_f64(
                -evaluate(&literal.argument, constants, guards)?.as_f64()?,
            )?)
        }
        Expression::BinaryExpression(binary) => {
            use oxc::syntax::operator::BinaryOperator as Op;
            let a = evaluate(&binary.left, constants, guards)?.as_f64()?;
            let b = evaluate(&binary.right, constants, guards)?.as_f64()?;
            Json::Number(Number::from_f64(match binary.operator {
                Op::Addition => a + b,
                Op::Subtraction => a - b,
                Op::Multiplication => a * b,
                Op::Division => a / b,
                Op::Remainder => a % b,
                Op::Exponential => a.powf(b),
                _ => return None,
            })?)
        }
        Expression::ArrayExpression(array) => Json::Array(
            array
                .elements
                .iter()
                .map(|element| match element {
                    ArrayExpressionElement::SpreadElement(_)
                    | ArrayExpressionElement::Elision(_) => None,
                    element => evaluate(element.to_expression(), constants, guards),
                })
                .collect::<Option<Vec<_>>>()?,
        ),
        Expression::ObjectExpression(object) => Json::Object(
            object
                .properties
                .iter()
                .map(|property| {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        return None;
                    };
                    if property.computed || property.method || property.kind != PropertyKind::Init {
                        return None;
                    }
                    let name = property.key.static_name()?.to_string();
                    if name == "__proto__" {
                        return None;
                    }
                    let value = evaluate(&property.value, constants, guards)?;
                    Some((name, value))
                })
                .collect::<Option<Map<_, _>>>()?,
        ),
        _ => return None,
    })
}

#[cfg(feature = "bundle")]
pub(super) fn linked_constants(
    path: &std::path::Path,
    program: &oxc::ast::ast::Program<'_>,
    assets: &std::path::Path,
    package_entry: &dyn Fn(&str) -> Option<std::path::PathBuf>,
    dependencies: &mut std::collections::BTreeSet<std::path::PathBuf>,
) -> HashMap<SymbolId, Json> {
    use oxc::ast::ast::Statement;

    let mut values = HashMap::new();
    let mut visiting = HashSet::new();
    for statement in &program.body {
        let Statement::ImportDeclaration(import) = statement else {
            continue;
        };
        if import.import_kind.is_type() {
            continue;
        }
        let Some(target) = import_source(path, import.source.value.as_str(), package_entry) else {
            continue;
        };
        let image = target
            .extension()
            .is_some_and(|ext| matches!(ext.to_str(), Some("png" | "jpg" | "jpeg" | "webp")))
            .then(|| crate::assets::image(assets, &target).ok())
            .flatten();
        let exported = exported_constants(&target, dependencies, &mut visiting, package_entry, 0);
        for specifier in import.specifiers.iter().flatten() {
            let (symbol, value) = match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier)
                    if !specifier.import_kind.is_type() =>
                {
                    (
                        specifier.local.symbol_id.get(),
                        exported
                            .get(specifier.imported.name().as_str())
                            .and_then(Option::as_ref),
                    )
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                    if image.is_some() {
                        dependencies.insert(target.clone());
                    }
                    (
                        specifier.local.symbol_id.get(),
                        image
                            .as_ref()
                            .or_else(|| exported.get("default").and_then(Option::as_ref)),
                    )
                }
                _ => continue,
            };
            if let (Some(symbol), Some(value)) = (symbol, value) {
                values.insert(symbol, value.clone());
            }
        }
    }
    values
}

#[cfg(feature = "bundle")]
fn import_source(
    owner: &std::path::Path,
    name: &str,
    package_entry: &dyn Fn(&str) -> Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    relative_import(owner, name).or_else(|| package_entry(name))
}

#[cfg(feature = "bundle")]
pub(super) fn builtin_export(name: &str, packages: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = name.strip_prefix("@impasto/")?;
    let (package, subpath) = name.split_once('/').unwrap_or((name, ""));
    if package.is_empty() || package.starts_with('.') || package.contains('\\') {
        return None;
    }
    let directory = packages.join(package);
    let manifest: Json =
        serde_json::from_slice(&std::fs::read(directory.join("package.json")).ok()?).ok()?;
    let key = if subpath.is_empty() {
        ".".to_owned()
    } else {
        format!("./{subpath}")
    };
    let target = directory
        .join(manifest["exports"][key].as_str()?)
        .canonicalize()
        .ok()?;
    target
        .starts_with(directory.canonicalize().ok()?)
        .then_some(target)
}

#[cfg(feature = "bundle")]
fn relative_import(owner: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    if !(name.starts_with("./") || name.starts_with("../")) {
        return None;
    }
    let path = owner.parent()?.join(name);
    if path.is_file() {
        return path.canonicalize().ok();
    }
    let extensions = ["tsx", "ts", "jsx", "js", "json"];
    extensions
        .iter()
        .map(|ext| std::path::PathBuf::from(format!("{}.{ext}", path.display())))
        .chain(
            (path.is_dir() && !path.join("package.json").is_file())
                .then(|| path.join("index"))
                .into_iter()
                .flat_map(|index| extensions.iter().map(move |ext| index.with_extension(ext))),
        )
        .find(|candidate| candidate.is_file())?
        .canonicalize()
        .ok()
}

#[cfg(feature = "bundle")]
type Exports = HashMap<String, Option<Json>>;

#[cfg(feature = "bundle")]
fn exported_constants(
    path: &std::path::Path,
    dependencies: &mut std::collections::BTreeSet<std::path::PathBuf>,
    visiting: &mut HashSet<std::path::PathBuf>,
    package_entry: &dyn Fn(&str) -> Option<std::path::PathBuf>,
    depth: usize,
) -> Exports {
    if depth >= 8
        || !path
            .extension()
            .is_some_and(|ext| matches!(ext.to_str(), Some("ts" | "tsx" | "js" | "jsx")))
        || !visiting.insert(path.to_owned())
    {
        return Exports::new();
    }
    dependencies.insert(path.to_owned());
    let result =
        read_exports(path, dependencies, visiting, package_entry, depth).unwrap_or_default();
    visiting.remove(path);
    result
}

#[cfg(feature = "bundle")]
fn read_exports(
    path: &std::path::Path,
    dependencies: &mut std::collections::BTreeSet<std::path::PathBuf>,
    visiting: &mut HashSet<std::path::PathBuf>,
    package_entry: &dyn Fn(&str) -> Option<std::path::PathBuf>,
    depth: usize,
) -> Option<Exports> {
    use oxc::{
        allocator::Allocator,
        ast::ast::{Declaration, Statement},
        parser::Parser,
        semantic::SemanticBuilder,
        span::SourceType,
    };

    let source = std::fs::read_to_string(path).ok()?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &source, SourceType::from_path(path).ok()?).parse();
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let semantic = SemanticBuilder::new()
        .with_build_nodes(true)
        .build(&parsed.program)
        .semantic;
    let scoping = semantic.scoping();
    let intrinsic_freeze = scoping
        .find_binding(scoping.root_scope_id(), "Object".into())
        .is_none();
    let mut locals = Exports::new();
    let mut constants = Constants::default();
    let mut explicit = Exports::new();
    let mut stars = Exports::new();
    let mut unsupported_star = false;
    for statement in &parsed.program.body {
        let (variable, exported) = match statement {
            Statement::VariableDeclaration(variable) => (variable, false),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::VariableDeclaration(variable) => (variable, true),
                Declaration::FunctionDeclaration(function) => {
                    if let Some(id) = &function.id {
                        explicit.insert(id.name.to_string(), None);
                    }
                    continue;
                }
                Declaration::ClassDeclaration(class) => {
                    if let Some(id) = &class.id {
                        explicit.insert(id.name.to_string(), None);
                    }
                    continue;
                }
                _ => continue,
            },
            _ => continue,
        };
        for item in &variable.declarations {
            let BindingPattern::BindingIdentifier(binding) = &item.id else {
                continue;
            };
            let value = if variable.kind == VariableDeclarationKind::Const {
                item.init
                    .as_ref()
                    .and_then(|init| frozen_literal(init, &constants, intrinsic_freeze))
            } else {
                None
            };
            if let (Some(symbol), Some(value)) = (binding.symbol_id.get(), value.as_ref()) {
                for reference in scoping.get_resolved_references(symbol) {
                    let span = semantic.nodes().get_node(reference.node_id()).span();
                    if span.start > item.span.end {
                        constants
                            .reads
                            .insert(span, (value.clone(), binding.name.to_string()));
                    }
                }
            }
            locals.insert(binding.name.to_string(), value.clone());
            if exported {
                explicit.insert(binding.name.to_string(), value);
            }
        }
    }
    for statement in &parsed.program.body {
        match statement {
            Statement::ExportNamedDeclaration(export) if !export.export_kind.is_type() => {
                for specifier in &export.specifiers {
                    if !specifier.export_kind.is_type() {
                        let value = locals
                            .get(specifier.local.name().as_str())
                            .cloned()
                            .flatten();
                        explicit.insert(specifier.exported.name().to_string(), value);
                    }
                }
            }
            Statement::ExportFromDeclaration(export) if !export.export_kind.is_type() => {
                let from = import_source(path, export.source.value.as_str(), package_entry)
                    .map(|target| {
                        exported_constants(
                            &target,
                            dependencies,
                            visiting,
                            package_entry,
                            depth + 1,
                        )
                    })
                    .unwrap_or_default();
                for specifier in &export.specifiers {
                    if !specifier.export_kind.is_type() {
                        let value = from.get(specifier.local.name().as_str()).cloned().flatten();
                        explicit.insert(specifier.exported.name().to_string(), value);
                    }
                }
            }
            Statement::ExportAllDeclaration(export) if !export.export_kind.is_type() => {
                if let Some(name) = &export.exported {
                    explicit.insert(name.name().to_string(), None);
                } else if let Some(target) =
                    import_source(path, export.source.value.as_str(), package_entry)
                {
                    let exports = exported_constants(
                        &target,
                        dependencies,
                        visiting,
                        package_entry,
                        depth + 1,
                    );
                    if exports.is_empty() {
                        unsupported_star = true;
                    }
                    for (name, value) in exports {
                        if name != "default" {
                            stars
                                .entry(name)
                                .and_modify(|existing| *existing = None)
                                .or_insert(value);
                        }
                    }
                } else {
                    unsupported_star = true;
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                let value = export
                    .declaration
                    .as_expression()
                    .and_then(|init| frozen_literal(init, &constants, intrinsic_freeze));
                explicit.insert("default".to_owned(), value);
            }
            _ => {}
        }
    }
    if unsupported_star {
        stars.clear();
    }
    stars.extend(explicit);
    Some(stars)
}

#[cfg(feature = "bundle")]
fn frozen_literal(
    expression: &Expression<'_>,
    constants: &Constants,
    intrinsic_freeze: bool,
) -> Option<Json> {
    use oxc::ast::ast::{Argument, TSType, TSTypeName};
    match expression.without_parentheses() {
        Expression::TSAsExpression(assertion)
            if matches!(&assertion.type_annotation,
            TSType::TSTypeReference(reference) if matches!(&reference.type_name,
                TSTypeName::IdentifierReference(name) if name.name == "const")) =>
        {
            return constants.literal(&assertion.expression, &mut Vec::new());
        }
        Expression::TSSatisfiesExpression(assertion) => {
            return frozen_literal(&assertion.expression, constants, intrinsic_freeze);
        }
        Expression::CallExpression(call)
            if intrinsic_freeze
                && !call.optional
                && let Expression::StaticMemberExpression(member) =
                    call.callee.get_inner_expression()
                && !member.optional
                && member.property.name == "freeze"
                && matches!(member.object.get_inner_expression(), Expression::Identifier(object) if object.name == "Object")
                && let [argument] = call.arguments.as_slice()
                && !matches!(argument, Argument::SpreadElement(_)) =>
        {
            match argument.to_expression().get_inner_expression() {
                Expression::ObjectExpression(object) => {
                    return object
                        .properties
                        .iter()
                        .map(|property| {
                            let ObjectPropertyKind::ObjectProperty(property) = property else {
                                return None;
                            };
                            if property.computed
                                || property.method
                                || property.kind != PropertyKind::Init
                            {
                                return None;
                            }
                            let name = property.key.static_name()?.to_string();
                            if name == "__proto__" {
                                return None;
                            }
                            Some((
                                name,
                                frozen_literal(&property.value, constants, intrinsic_freeze)?,
                            ))
                        })
                        .collect::<Option<Map<_, _>>>()
                        .map(Json::Object);
                }
                Expression::ArrayExpression(array) => {
                    return array
                        .elements
                        .iter()
                        .map(|element| {
                            if matches!(
                                element,
                                ArrayExpressionElement::Elision(_)
                                    | ArrayExpressionElement::SpreadElement(_)
                            ) {
                                return None;
                            }
                            frozen_literal(element.to_expression(), constants, intrinsic_freeze)
                        })
                        .collect::<Option<Vec<_>>>()
                        .map(Json::Array);
                }
                _ => {}
            }
        }
        _ => {}
    }
    constants
        .literal(expression, &mut Vec::new())
        .filter(|value| !value.is_object() && !value.is_array())
}

pub(super) fn style_value(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Bool(*value),
        Json::Number(value) => Value::Number(value.as_f64().unwrap_or_default()),
        Json::String(value) => Value::String(value.clone()),
        Json::Array(value) => Value::Array(value.iter().map(style_value).collect()),
        Json::Object(value) => Value::Object(
            value
                .iter()
                .map(|(name, value)| (name.clone(), style_value(value)))
                .collect(),
        ),
    }
}

pub(super) fn validate_surface(
    value: &Json,
    surface: engine::style::StyleSurface,
) -> Result<(), String> {
    let style =
        engine::style::codec::from_value(&style_value(value)).map_err(|error| error.to_string())?;
    for declaration in style.fragments().iter().flatten() {
        if !engine::style::author_key_allowed(surface, declaration) {
            return Err(format!(
                "style property \"{}\" is managed by {}",
                declaration.key().name(),
                surface.name()
            ));
        }
    }
    Ok(())
}
