//! JSX lowering for Impasto's scene and reactive runtime.

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    fmt::Write as _,
};

use oxc::{
    allocator::{Allocator, CloneIn, TakeIn},
    ast::{
        ast::{
            ArrayExpressionElement, Expression, ImportDeclarationSpecifier, JSXAttributeItem,
            JSXAttributeName, JSXAttributeValue, JSXChild, JSXElement, JSXElementName,
            JSXExpression, JSXMemberExpression, JSXMemberExpressionObject, ObjectPropertyKind,
            Program, PropertyKind, Statement,
        },
        builder::AstBuilder,
    },
    ast_visit::{Visit, VisitMut, walk_mut},
    diagnostics::OxcDiagnostic,
    parser::Parser,
    semantic::{Scoping, Semantic},
    span::{GetSpan, SourceType, Span},
    syntax::scope::{ScopeFlags, ScopeId},
};

use crate::pools::PoolModules;

const RENDERER: &str = "@impasto/solid-renderer";
const UI: &str = "@impasto/solid-ui";
const HOST: &str = "@impasto/solid-renderer/host";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Primitive {
    Host(&'static str),
    Box(&'static str),
    Text,
    Image,
}

impl Primitive {
    fn imported(module: &str, name: &str) -> Option<Self> {
        Some(match (module, name) {
            ("@impasto/solid-renderer/host", "View") => Self::Host("view"),
            ("@impasto/solid-renderer/host", "Stack") => Self::Host("stack"),
            ("@impasto/solid-renderer/host", "Scroll") => Self::Host("scroll"),
            ("@impasto/solid-renderer/host", "Text") => Self::Host("text"),
            (UI, "View") => Self::Box("view"),
            (UI, "Row") => Self::Box("row"),
            (UI, "Column") => Self::Box("column"),
            (UI, "Grid") => Self::Box("grid"),
            (UI, "Stack") => Self::Box("stack"),
            (UI, "RepaintBoundary") => Self::Box("repaint"),
            (UI | RENDERER, "Text") => Self::Text,
            (UI | RENDERER | HOST, "Image") => Self::Image,
            _ => return None,
        })
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Host(tag) | Self::Box(tag) => tag,
            Self::Text => "text",
            Self::Image => "image",
        }
    }
}

pub(crate) fn lower<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    scoping: &Scoping,
    source: &str,
    constants: &crate::styles::Constants,
    pools: Option<&mut PoolModules>,
    options: LoweringOptions<'_>,
) -> Result<(), OxcDiagnostic> {
    let LoweringOptions { hot, debug_source } = options;
    let mut compiler = Compiler {
        allocator,
        scoping,
        constants,
        scopes: Vec::new(),
        primitives: HashMap::new(),
        lists: HashSet::new(),
        virtual_lists: HashSet::new(),
        namespace: "__impasto_jsx".to_owned(),
        used: false,
        styles: Vec::new(),
        used_styles: HashSet::new(),
        used_patches: false,
        classes: Vec::new(),
        used_classes: HashSet::new(),
        used_class_list: false,
        templates: Vec::new(),
        error: None,
        linked: pools.is_some(),
        hot,
        debug_source,
        source,
        line_starts: debug_source.map_or_else(Vec::new, |_| {
            std::iter::once(0)
                .chain(source.match_indices('\n').map(|(i, _)| i + 1))
                .collect()
        }),
    };
    while source.contains(&compiler.namespace) {
        compiler.namespace.push('_');
    }
    for statement in &program.body {
        let Statement::ImportDeclaration(import) = statement else {
            continue;
        };
        if import.import_kind.is_type() {
            continue;
        }
        for specifier in import.specifiers.iter().flatten() {
            let ImportDeclarationSpecifier::ImportSpecifier(specifier) = specifier else {
                continue;
            };
            if matches!(
                import.source.value.as_str(),
                "solid-js" | "@impasto/reactive"
            ) && !specifier.import_kind.is_type()
                && specifier.imported.name() == "For"
                && let Some(symbol) = specifier.local.symbol_id.get()
            {
                compiler.lists.insert(symbol);
            }
            if import.source.value == "@impasto/solid-ui"
                && specifier.imported.name() == "VirtualList"
                && let Some(symbol) = specifier.local.symbol_id.get()
            {
                compiler.virtual_lists.insert(symbol);
            }
            if !specifier.import_kind.is_type()
                && let Some(primitive) = Primitive::imported(
                    import.source.value.as_str(),
                    specifier.imported.name().as_str(),
                )
                && let Some(symbol) = specifier.local.symbol_id.get()
            {
                compiler.primitives.insert(symbol, primitive);
            }
        }
    }
    compiler.visit_program(program);
    if let Some(error) = compiler.error {
        return Err(error);
    }
    if compiler.used {
        let namespace = &compiler.namespace;
        let mut head = format!("import * as {namespace} from '{RENDERER}';");
        if !compiler.used_styles.is_empty() {
            let _ = write!(
                head,
                "import {{ applyStaticStyle as {namespace}_applyStaticStyle }} from 'scene';"
            );
        }
        if compiler.used_patches {
            let _ = write!(
                head,
                "import {{ patchInlineStyle as {namespace}_patchInlineStyle }} from 'scene';"
            );
        }
        if !compiler.used_classes.is_empty() {
            let _ = write!(
                head,
                "import {{ applyStaticClass as {namespace}_applyStaticClass }} from 'scene';"
            );
        }
        if compiler.used_class_list {
            let _ = write!(
                head,
                "import {{ applyStaticClassList as {namespace}_applyStaticClassList }} from 'scene';"
            );
        }
        if !compiler.templates.is_empty() {
            let _ = write!(
                head,
                "import {{ instantiateTemplate as {namespace}_instantiateTemplate }} from 'scene';"
            );
        }
        if let Some(pools) = pools {
            compiler.linked_pools(&mut head, pools)?;
        } else {
            compiler.local_pools(&mut head);
        }
        let text = allocator.alloc_str(&head);
        let parsed = Parser::new(allocator, text, SourceType::mjs()).parse();
        if let Some(error) = parsed.diagnostics.into_iter().next() {
            return Err(OxcDiagnostic::error(format!(
                "could not emit Impasto renderer imports: {error}"
            )));
        }
        let mut declarations = parsed.program.body;
        let mut splice = Splice {
            arguments: Vec::new(),
            span: Span::default(),
        };
        for declaration in &mut declarations {
            splice.visit_statement(declaration);
        }
        declarations.extend(program.body.take_in(&AstBuilder::new(allocator)));
        program.body = declarations;
    }
    Ok(())
}

struct Compiler<'a, 's> {
    allocator: &'a Allocator,
    scoping: &'s Scoping,
    constants: &'s crate::styles::Constants,
    scopes: Vec<ScopeId>,
    primitives: HashMap<oxc::syntax::symbol::SymbolId, Primitive>,
    lists: HashSet<oxc::syntax::symbol::SymbolId>,
    virtual_lists: HashSet<oxc::syntax::symbol::SymbolId>,
    namespace: String,
    used: bool,
    styles: Vec<String>,
    used_styles: HashSet<usize>,
    used_patches: bool,
    classes: Vec<String>,
    used_classes: HashSet<usize>,
    used_class_list: bool,
    templates: Vec<String>,
    error: Option<OxcDiagnostic>,
    linked: bool,
    hot: bool,
    debug_source: Option<DebugSource<'s>>,
    source: &'s str,
    line_starts: Vec<usize>,
}

#[derive(Clone, Copy)]
pub(crate) struct DebugSource<'a> {
    pub file: &'a str,
    pub components: &'a HashMap<Span, String>,
}

pub(crate) struct LoweringOptions<'a> {
    pub hot: bool,
    pub debug_source: Option<DebugSource<'a>>,
}

pub(crate) fn component_names(semantic: &Semantic<'_>) -> HashMap<Span, String> {
    use oxc::{ast::AstKind, ast::ast::BindingPattern};

    let nodes = semantic.nodes();
    nodes
        .iter()
        .filter_map(|node| {
            if !matches!(node.kind(), AstKind::JSXElement(_)) {
                return None;
            }
            let name = nodes
                .ancestor_kinds(node.id())
                .filter_map(|kind| match kind {
                    AstKind::Function(function) => {
                        function.id.as_ref().map(|name| name.name.as_str())
                    }
                    AstKind::VariableDeclarator(declaration) => {
                        let BindingPattern::BindingIdentifier(id) = &declaration.id else {
                            return None;
                        };
                        Some(id.name.as_str())
                    }
                    _ => None,
                })
                .find(|name| name.as_bytes().first().is_some_and(u8::is_ascii_uppercase))?;
            Some((node.span(), name.to_owned()))
        })
        .collect()
}

#[derive(Default)]
struct Template {
    rows: Vec<serde_json::Value>,
    bindings: String,
    refs: String,
    next_range: usize,
}

enum TemplateChild {
    Value(String),
    List { props: String, component: String },
}

impl TemplateChild {
    fn value(self, rt: &str) -> String {
        match self {
            Self::Value(value) => value,
            Self::List { props, component } => {
                format!("{rt}.createComponent({component}, {props})")
            }
        }
    }
}

impl Template {
    fn range(
        &mut self,
        rt: &str,
        parent: usize,
        pending: &mut Vec<TemplateChild>,
        anchor: usize,
        ranges: &mut String,
    ) {
        if pending.is_empty() {
            return;
        }
        let range = self.next_range;
        self.next_range += 1;
        let marker = if anchor == 0 {
            "0".to_owned()
        } else {
            format!("{rt}_nodes[{anchor}]")
        };
        let mut children = std::mem::take(pending);
        if let [TemplateChild::List { props, .. }] = children.as_slice() {
            let _ = write!(self.bindings, "const {rt}_range{range} = {props};");
            let _ = write!(
                ranges,
                "{rt}.insertKeyedList({rt}_nodes[{parent}], {rt}_range{range}, {marker});"
            );
            return;
        }
        let value = if children.len() == 1 {
            children
                .pop()
                .map_or_else(String::new, |child| child.value(rt))
        } else {
            let values = children
                .into_iter()
                .map(|child| child.value(rt))
                .collect::<Vec<_>>();
            format!("[{}]", values.join(", "))
        };
        let _ = write!(self.bindings, "const {rt}_range{range} = {value};");
        let _ = write!(
            ranges,
            "{rt}.insert({rt}_nodes[{parent}], {rt}_range{range}, {marker});"
        );
    }
}

struct Code<'a> {
    arguments: Vec<Expression<'a>>,
}

impl<'a> Code<'a> {
    fn expression(&mut self, expression: Expression<'a>) -> String {
        let index = self.arguments.len();
        self.arguments.push(expression);
        format!("__jsx_argument_{index}")
    }
}

enum Attribute {
    Prop {
        name: String,
        value: String,
        dynamic: bool,
    },
    Spread(String),
    StaticStyle {
        index: Option<usize>,
        patches: Vec<StylePatch>,
        guards: Vec<String>,
    },
    StaticClass(usize),
    ClassChoice {
        selection: String,
        indices: Vec<usize>,
    },
    ClassList {
        entries: Vec<(usize, String)>,
        dynamic: bool,
    },
}

struct StylePatch {
    selector: String,
    scalar: bool,
    value: String,
    dynamic: bool,
}

impl<'a> VisitMut<'a> for Compiler<'a, '_> {
    fn enter_scope(&mut self, _: ScopeFlags, id: &Cell<Option<ScopeId>>) {
        self.scopes.push(
            id.get()
                .or_else(|| self.scopes.last().copied())
                .unwrap_or(self.scoping.root_scope_id()),
        );
    }

    fn leave_scope(&mut self) {
        self.scopes.pop();
    }

    fn visit_expression(&mut self, expression: &mut Expression<'a>) {
        if !matches!(
            expression,
            Expression::JSXElement(_) | Expression::JSXFragment(_)
        ) {
            walk_mut::walk_expression(self, expression);
            return;
        }
        let span = expression.span();
        self.used = true;
        let builder = AstBuilder::new(self.allocator);
        let mut code = Code {
            arguments: Vec::new(),
        };
        let generated = match expression.take_in(&builder) {
            Expression::JSXElement(element) => self.element(element.unbox(), &mut code),
            Expression::JSXFragment(fragment) => format!(
                "[{}]",
                self.children(fragment.unbox().children, &mut code, true)
                    .join(", ")
            ),
            _ => return,
        };
        let generated = self.allocator.alloc_str(&generated);
        match Parser::new(self.allocator, generated, SourceType::mjs()).parse_expression() {
            Ok(mut output) => {
                Splice {
                    arguments: code.arguments.into_iter().map(Some).collect(),
                    span,
                }
                .visit_expression(&mut output);
                *expression = output;
            }
            Err(errors) => {
                self.fail(span, format!("could not generate JSX: {errors:?}"));
            }
        }
    }
}

impl<'a> Compiler<'a, '_> {
    fn source_location(&self, output: &mut String, node: &str, span: Span) {
        let Some(debug) = self.debug_source else {
            return;
        };
        let offset = span.start as usize;
        let line = self.line_starts.partition_point(|start| *start <= offset);
        let Some(start) = line
            .checked_sub(1)
            .and_then(|line| self.line_starts.get(line))
        else {
            return;
        };
        let Some(prefix) = self.source.get(*start..offset) else {
            return;
        };
        let column = prefix.encode_utf16().count() + 1;
        let component = debug.components.get(&span).map_or("", String::as_str);
        let _ = write!(
            output,
            "{}.scene.debugSetSource({node}, {}, {line}, {column}, {});",
            self.namespace,
            quote(debug.file),
            quote(component)
        );
    }

    fn local_pools(&self, code: &mut String) {
        let ns = &self.namespace;
        for (kind, define, entries, used) in [
            (
                "style",
                "defineStyle",
                &self.styles,
                Some(&self.used_styles),
            ),
            (
                "class",
                "defineClass",
                &self.classes,
                Some(&self.used_classes),
            ),
            ("template", "defineTemplate", &self.templates, None),
        ] {
            if entries.is_empty() || used.is_some_and(HashSet::is_empty) {
                continue;
            }
            let _ = write!(code, "import {{ {define} as {ns}_{define} }} from 'scene';");
            let values = entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    if used.is_some_and(|used| !used.contains(&index)) {
                        return "undefined".to_owned();
                    }
                    let entry = if kind == "template" {
                        entry.clone()
                    } else {
                        quote(entry)
                    };
                    if self.hot {
                        let _ = write!(code, "let {ns}_{kind}{index};");
                        format!("() => {ns}_{kind}{index} ??= {ns}_{define}({entry})")
                    } else {
                        format!("{ns}_{define}({entry})")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(code, "const {ns}_{kind}s = [{values}];");
        }
    }

    fn linked_pools(
        &self,
        code: &mut String,
        pools: &mut PoolModules,
    ) -> Result<(), OxcDiagnostic> {
        let ns = &self.namespace;
        for (kind, define, entries, used) in [
            (
                "style",
                "defineStyle",
                &self.styles,
                Some(&self.used_styles),
            ),
            (
                "class",
                "defineClass",
                &self.classes,
                Some(&self.used_classes),
            ),
            ("template", "defineTemplate", &self.templates, None),
        ] {
            for (index, entry) in entries.iter().enumerate() {
                if used.is_some_and(|used| !used.contains(&index)) {
                    continue;
                }
                let module = if kind == "template" {
                    pools.template(entry)?
                } else {
                    pools.scalar(kind, define, entry)?
                };
                let _ = write!(code, "import {ns}_{kind}{index} from {};", quote(&module));
            }
        }
        Ok(())
    }

    fn pool_ref(&self, kind: &str, index: usize) -> String {
        let rt = &self.namespace;
        if self.linked && kind != "class" {
            format!("{rt}_{kind}{index}")
        } else if self.linked {
            format!("{rt}_{kind}{index}()")
        } else if self.hot {
            format!("{rt}_{kind}s[{index}]()")
        } else {
            format!("{rt}_{kind}s[{index}]")
        }
    }

    fn class_index(&mut self, span: Span, text: &str) -> Option<usize> {
        match engine::style::encode_static_class(text) {
            Ok(encoded) => Some(intern(&mut self.classes, encoded)),
            Err(error) => {
                self.fail(span, error);
                None
            }
        }
    }

    fn static_class(&mut self, span: Span, text: &str, can_pool: bool) -> Option<Attribute> {
        if can_pool {
            self.class_index(span, text).map(Attribute::StaticClass)
        } else {
            if let Err(error) = engine::style::parse_class(text) {
                self.fail(span, error.to_string());
            }
            None
        }
    }

    fn class_choice(
        &mut self,
        expression: &Expression<'a>,
        code: &mut Code<'a>,
    ) -> Option<Attribute> {
        let mut indices = Vec::new();
        let selection = self.class_branch(expression, code, &mut indices)?;
        Some(Attribute::ClassChoice { selection, indices })
    }

    fn class_branch(
        &mut self,
        expression: &Expression<'a>,
        code: &mut Code<'a>,
        indices: &mut Vec<usize>,
    ) -> Option<String> {
        if let Some(serde_json::Value::String(text)) = crate::styles::literal(expression) {
            let index = self.class_index(expression.span(), &text)?;
            indices.push(index);
            return Some(self.pool_ref("class", index));
        }
        let Expression::ConditionalExpression(choice) = expression.get_inner_expression() else {
            return None;
        };
        let yes = self.class_branch(&choice.consequent, code, indices)?;
        let no = self.class_branch(&choice.alternate, code, indices)?;
        let test = code.expression(choice.test.clone_in(self.allocator));
        Some(format!("(({test}) ? {yes} : {no})"))
    }

    fn class_list(
        &mut self,
        expression: &Expression<'a>,
        code: &mut Code<'a>,
    ) -> Option<Attribute> {
        let Expression::ObjectExpression(object) = expression.get_inner_expression() else {
            return None;
        };
        let fields = object
            .properties
            .iter()
            .map(plain_property)
            .collect::<Option<Vec<_>>>()?;
        let mut names = HashSet::new();
        if !fields.iter().all(|(name, _)| names.insert(name.as_ref())) {
            return None;
        }
        let mut entries = Vec::new();
        let mut dynamic = false;
        for (name, expression) in fields {
            if matches!(
                crate::styles::literal(expression),
                Some(serde_json::Value::Bool(false))
            ) {
                continue;
            }
            let index = self.class_index(expression.span(), &name)?;
            dynamic |= is_dynamic(expression);
            let test = code.expression(expression.clone_in(self.allocator));
            entries.push((index, test));
        }
        Some(Attribute::ClassList { entries, dynamic })
    }

    fn fail(&mut self, span: Span, message: impl Into<String>) {
        self.error
            .get_or_insert_with(|| OxcDiagnostic::error(message.into()).with_label(span));
    }

    fn child_expression(
        &mut self,
        expression: JSXExpression<'a>,
    ) -> Option<(Expression<'a>, bool)> {
        if matches!(expression, JSXExpression::EmptyExpression(_)) {
            return None;
        }
        let mut expression = expression.into_expression();
        let dynamic = is_dynamic(&expression);
        self.visit_expression(&mut expression);
        Some((expression, dynamic))
    }

    fn children(
        &mut self,
        children: oxc::allocator::Vec<'a, JSXChild<'a>>,
        code: &mut Code<'a>,
        lazy: bool,
    ) -> Vec<String> {
        children
            .into_iter()
            .filter_map(|child| self.child(child, code, lazy))
            .collect()
    }

    fn child(&mut self, child: JSXChild<'a>, code: &mut Code<'a>, lazy: bool) -> Option<String> {
        Some(match child {
            JSXChild::Text(text) => quote(&clean_text(&text.value)?),
            JSXChild::Element(child) => self.element(child.unbox(), code),
            JSXChild::Fragment(fragment) => format!(
                "[{}]",
                self.children(fragment.unbox().children, code, lazy)
                    .join(", ")
            ),
            JSXChild::ExpressionContainer(child) => {
                let (expression, dynamic) = self.child_expression(child.unbox().expression)?;
                let value = code.expression(expression);
                if dynamic && lazy {
                    format!("{}.memo(() => ({value}))", self.namespace)
                } else {
                    value
                }
            }
            JSXChild::Spread(child) => {
                self.fail(child.span, "spread children are unsupported");
                return None;
            }
        })
    }

    fn attributes(
        &mut self,
        opening: oxc::allocator::Vec<'a, JSXAttributeItem<'a>>,
        code: &mut Code<'a>,
        host: Option<Primitive>,
        virtual_list: bool,
    ) -> Vec<Attribute> {
        let can_pool = host.is_some()
            && !opening
                .iter()
                .any(|attr| matches!(attr, JSXAttributeItem::SpreadAttribute(_)))
            && opening
                .iter()
                .filter(|attr| {
                    matches!(attr, JSXAttributeItem::Attribute(attr)
                if matches!(&attr.name, JSXAttributeName::Identifier(id) if id.name == "style"))
                })
                .take(2)
                .count()
                < 2;
        opening
            .into_iter()
            .filter_map(|attribute| {
                let attribute = match attribute {
                    JSXAttributeItem::Attribute(attribute) => attribute.unbox(),
                    JSXAttributeItem::SpreadAttribute(spread) => {
                        let mut expression = spread.unbox().argument;
                        self.visit_expression(&mut expression);
                        return Some(Attribute::Spread(code.expression(expression)));
                    }
                };
                let span = attribute.span;
                let name = match attribute.name {
                    JSXAttributeName::Identifier(id) => id.name.to_string(),
                    JSXAttributeName::NamespacedName(name) => {
                        self.fail(name.span, "namespaced JSX props are unsupported");
                        return None;
                    }
                };
                let (value, dynamic) = match attribute.value {
                    None => ("true".to_owned(), false),
                    Some(JSXAttributeValue::StringLiteral(string)) => {
                        if host.is_some()
                            && name == "class"
                            && let Some(class) = self.static_class(span, &string.value, can_pool)
                        {
                            return Some(class);
                        }
                        (quote(&string.value), false)
                    }
                    Some(JSXAttributeValue::ExpressionContainer(container)) => {
                        let (expression, dynamic) =
                            self.child_expression(container.unbox().expression)?;
                        if host.is_some()
                            && name == "class"
                            && let Some(serde_json::Value::String(text)) =
                                crate::styles::literal(&expression)
                            && let Some(class) = self.static_class(span, &text, can_pool)
                        {
                            return Some(class);
                        }
                        if can_pool
                            && name == "class"
                            && let Some(class) = self.class_choice(&expression, code)
                        {
                            return Some(class);
                        }
                        if can_pool
                            && name == "classList"
                            && !matches!(host, Some(Primitive::Host(_)))
                            && let Some(class_list) = self.class_list(&expression, code)
                        {
                            return Some(class_list);
                        }
                        if virtual_list
                            && name == "contentProps"
                            && let Some(value) = crate::styles::literal(&expression)
                            && let Some(style) = value.get("style")
                            && let Err(error) = crate::styles::validate_surface(
                                style,
                                engine::style::StyleSurface::ListContent,
                            )
                        {
                            self.fail(expression.span(), error);
                        }
                        let mut guards = Vec::new();
                        if can_pool
                            && name == "style"
                            && let Some((literal, patches)) =
                                static_style(&expression, self.constants, &mut guards)
                        {
                            let style = crate::styles::style_value(&literal);
                            match self
                                .constants
                                .validate_tokens(&literal)
                                .and_then(|()| {
                                    engine::style::codec::from_value(&style)
                                        .map_err(|error| error.to_string())
                                })
                                .and_then(|style| {
                                    (!style.is_empty())
                                        .then(|| engine::style::static_pool::encode_style(&style))
                                        .transpose()
                                }) {
                                Ok(encoded) => {
                                    let index =
                                        encoded.map(|encoded| intern(&mut self.styles, encoded));
                                    let patches = patches
                                        .into_iter()
                                        .map(|patch| patch.lower(self.allocator, code))
                                        .collect();
                                    return Some(Attribute::StaticStyle {
                                        index,
                                        patches,
                                        guards,
                                    });
                                }
                                Err(error) => self.fail(expression.span(), error),
                            }
                        }
                        (code.expression(expression), dynamic)
                    }
                    Some(JSXAttributeValue::Element(element)) => {
                        (self.element(element.unbox(), code), true)
                    }
                    Some(JSXAttributeValue::Fragment(fragment)) => (
                        format!(
                            "[{}]",
                            self.children(fragment.unbox().children, code, true)
                                .join(", ")
                        ),
                        true,
                    ),
                };
                Some(Attribute::Prop {
                    name,
                    value,
                    dynamic,
                })
            })
            .collect()
    }

    fn template_host(&self, element: &JSXElement<'_>) -> Option<Primitive> {
        let opening = &element.opening_element;
        let JSXElementName::IdentifierReference(id) = &opening.name else {
            return None;
        };
        let scope = self
            .scopes
            .last()
            .copied()
            .unwrap_or(self.scoping.root_scope_id());
        let primitive = *self
            .primitives
            .get(&self.scoping.find_binding(scope, id.name)?)?;

        // Rich text and props from a spread keep their component's runtime contract.
        let mut names = HashSet::new();
        for attribute in &opening.attributes {
            let JSXAttributeItem::Attribute(attribute) = attribute else {
                return None;
            };
            let JSXAttributeName::Identifier(name) = &attribute.name else {
                return None;
            };
            let name = name.name.as_str();
            if name == "children" || !names.insert(name) {
                return None;
            }
            let supported = match primitive {
                Primitive::Host(_) => true,
                Primitive::Text => matches!(name, "style" | "class" | "classList" | "ref"),
                Primitive::Box(_) => name == "ref" || is_box_prop(name),
                Primitive::Image => matches!(
                    name,
                    "style"
                        | "class"
                        | "classList"
                        | "ref"
                        | "source"
                        | "intrinsicSize"
                        | "alt"
                        | "required"
                ),
            };
            if !supported {
                return None;
            }
        }
        if primitive == Primitive::Text
            && element.children.iter().any(|child| match child {
                JSXChild::Text(_) => false,
                JSXChild::ExpressionContainer(child) => child
                    .expression
                    .as_expression()
                    .is_some_and(|expression| !text_expression(expression)),
                _ => true,
            })
        {
            return None;
        }
        if primitive == Primitive::Image && !element.children.is_empty() {
            return None;
        }
        Some(primitive)
    }

    fn direct_list<'b>(&self, list: &'b JSXElement<'_>) -> Option<&'b str> {
        let JSXElementName::IdentifierReference(name) = &list.opening_element.name else {
            return None;
        };
        let scope = self
            .scopes
            .last()
            .copied()
            .unwrap_or(self.scoping.root_scope_id());
        (self
            .scoping
            .find_binding(scope, name.name)
            .is_some_and(|binding| self.lists.contains(&binding))
            && list
                .opening_element
                .attributes
                .iter()
                .all(|attr| matches!(attr, JSXAttributeItem::Attribute(_)))
            && matches!(list.children.as_slice(), [JSXChild::ExpressionContainer(child)]
                if matches!(child.expression.as_expression(), Some(
                    Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_)))))
        .then_some(name.name.as_str())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "validated dimensions are stored in scene points"
    )]
    fn static_image(
        &self,
        element: &JSXElement<'_>,
    ) -> Option<(engine::scene::template::TemplateImage, Vec<String>)> {
        use serde_json::Value;

        let mut source = Value::Null;
        let mut intrinsic = Value::Null;
        let mut required = false;
        let mut alt = None;
        let mut guards = Vec::new();
        for attr in &element.opening_element.attributes {
            let JSXAttributeItem::Attribute(attr) = attr else {
                return None;
            };
            let JSXAttributeName::Identifier(name) = &attr.name else {
                return None;
            };
            let name = name.name.as_str();
            if !matches!(name, "source" | "intrinsicSize" | "required" | "alt") {
                continue;
            }
            let value = match &attr.value {
                None => Value::Bool(true),
                Some(JSXAttributeValue::StringLiteral(value)) => {
                    Value::String(value.value.to_string())
                }
                Some(JSXAttributeValue::ExpressionContainer(value)) => self
                    .constants
                    .literal(value.expression.as_expression()?, &mut guards)?,
                _ => return None,
            };
            match name {
                "source" => source = value,
                "intrinsicSize" => intrinsic = value,
                "required" => required = value.as_bool()?,
                "alt" if !value.is_null() => alt = Some(value.as_str()?.to_owned()),
                _ => {}
            }
        }
        let uri = match &source {
            Value::Null => "",
            Value::String(uri) => uri,
            Value::Object(asset) => asset.get("uri")?.as_str()?,
            _ => return None,
        };
        let dimension = |name| {
            let number = intrinsic.get(name).or_else(|| source.get(name))?.as_f64()?;
            (number >= 0. && number <= f64::from(f32::MAX)).then(|| (number as f32).to_bits())
        };
        let intrinsic_size_bits = if intrinsic.is_null() && !source.is_object() {
            None
        } else {
            Some((dimension("width")?, dimension("height")?))
        };
        Some((
            engine::scene::template::TemplateImage {
                uri: uri.to_owned(),
                intrinsic_size_bits,
                required,
                alt,
            },
            guards,
        ))
    }

    fn template_text(
        &mut self,
        children: oxc::allocator::Vec<'a, JSXChild<'a>>,
        template: &mut Template,
        node: &str,
        code: &mut Code<'a>,
    ) -> String {
        let mut text = String::new();
        let mut parts = Vec::new();
        for child in children {
            match child {
                JSXChild::Text(value) => {
                    text.push_str(&clean_text(&value.value).unwrap_or_default())
                }
                JSXChild::ExpressionContainer(child) => {
                    let Some((expression, _)) = self.child_expression(child.unbox().expression)
                    else {
                        continue;
                    };
                    if let Some(value) = crate::styles::literal(&expression) {
                        match value {
                            serde_json::Value::String(value) => {
                                text.push_str(&value);
                                continue;
                            }
                            serde_json::Value::Bool(_) | serde_json::Value::Null => continue,
                            _ => {}
                        }
                    }
                    if !text.is_empty() {
                        parts.push(quote(&text));
                        text.clear();
                    }
                    parts.push(format!(
                        "{}.textContent(({}))",
                        self.namespace,
                        code.expression(expression)
                    ));
                }
                _ => {}
            }
        }
        if parts.is_empty() {
            return text;
        }
        if !text.is_empty() {
            parts.push(quote(&text));
        }
        let rt = &self.namespace;
        let _ = write!(
            template.bindings,
            "{rt}.bindTextValue({node}, () => ({}));",
            parts.join(" + "),
        );
        String::new()
    }

    fn template_node(
        &mut self,
        element: JSXElement<'a>,
        primitive: Primitive,
        parent: usize,
        template: &mut Template,
        code: &mut Code<'a>,
    ) {
        use serde_json::json;

        let index = template.rows.len();
        template.rows.push(serde_json::Value::Null);
        let node = format!("{}_nodes[{index}]", self.namespace);
        self.source_location(&mut template.bindings, &node, element.span);
        let mut style = None;
        let mut class = None;
        let mut node_ref = None;
        let mut semantics = [None, None, None];
        let mut image_props = [None, None, None, None];
        let image = if primitive == Primitive::Image {
            self.static_image(&element).map(|(image, guards)| {
                const_guards(&mut template.bindings, guards);
                image
            })
        } else {
            None
        };
        let tag = primitive.tag();
        for attribute in self.attributes(
            element.opening_element.unbox().attributes,
            code,
            Some(primitive),
            false,
        ) {
            match attribute {
                Attribute::StaticStyle {
                    index,
                    patches,
                    guards,
                } => {
                    style = index;
                    const_guards(&mut template.bindings, guards);
                    self.style_patches(&mut template.bindings, &node, patches);
                }
                Attribute::StaticClass(index) => class = Some(index),
                Attribute::Prop { name, value, .. } if name == "ref" => node_ref = Some(value),
                Attribute::Prop { name, value, .. }
                    if primitive == Primitive::Image
                        && let Some(index) = ["source", "intrinsicSize", "alt", "required"]
                            .iter()
                            .position(|prop| *prop == name) =>
                {
                    image_props[index] = Some(value);
                }
                Attribute::Prop {
                    name,
                    value,
                    dynamic,
                } if matches!(primitive, Primitive::Box(_))
                    && let Some(index) = semantics_index(&name) =>
                {
                    semantics[index] = Some((value, dynamic));
                }
                Attribute::Prop {
                    name,
                    value,
                    dynamic,
                } if name == "focusable" && matches!(primitive, Primitive::Box(_)) => {
                    let rt = &self.namespace;
                    if dynamic {
                        let _ = write!(
                            template.bindings,
                            "{rt}.effect(() => ({value}), next => {{ if (next !== undefined) {rt}.scene.setFocusable({node}, next); }});"
                        );
                    } else {
                        let _ = write!(
                            template.bindings,
                            "{rt}.scene.setFocusable({node}, ({value}));"
                        );
                    }
                }
                attribute => self.host_property(&mut template.bindings, &node, attribute, false),
            }
        }
        if semantics.iter().any(Option::is_some) {
            self.template_semantics(&node, semantics, &mut template.bindings);
        }
        if primitive == Primitive::Image && image.is_none() {
            let [source, size, alt, required] =
                image_props.map(|value| value.unwrap_or_else(|| "undefined".to_owned()));
            let rt = &self.namespace;
            let _ = write!(
                template.bindings,
                "{rt}.bindImageSource({node}, () => [({source}), ({size}), ({alt})], ({required}));"
            );
        }
        let mut children = Some(element.children);
        let text = match primitive {
            Primitive::Text => children
                .take()
                .map(|children| self.template_text(children, template, &node, code)),
            Primitive::Host("text") => Some(String::new()),
            _ => None,
        };
        template.rows[index] = json!([
            tag,
            parent,
            text,
            style.map(|index| &self.styles[index]),
            class.map(|index| &self.classes[index]),
            image,
        ]);

        // Evaluate child expressions in order, then install their ranges. Each run
        // of expressions uses the next template node as its insertion marker.
        let mut pending = Vec::new();
        let mut ranges = String::new();
        for child in children.into_iter().flatten() {
            if let JSXChild::Element(ref element) = child
                && let Some(component) = self.direct_list(element).map(str::to_owned)
                && let JSXChild::Element(child) = child
            {
                let child = child.unbox();
                let mut attrs =
                    self.attributes(child.opening_element.unbox().attributes, code, None, false);
                let Some(value) = self.children(child.children, code, false).pop() else {
                    continue;
                };
                attrs.push(Attribute::Prop {
                    name: "children".to_owned(),
                    value,
                    dynamic: false,
                });
                let props = property_groups(attrs).join(", ");
                pending.push(TemplateChild::List { props, component });
                continue;
            }
            let tag = match &child {
                JSXChild::Element(child) if template.rows.len() < 256 => self.template_host(child),
                _ => None,
            };
            match child {
                JSXChild::Element(child) if let Some(tag) = tag => {
                    template.range(
                        &self.namespace,
                        index,
                        &mut pending,
                        template.rows.len(),
                        &mut ranges,
                    );
                    self.template_node(child.unbox(), tag, index, template, code);
                }
                JSXChild::Text(text) => {
                    if let Some(text) = clean_text(&text.value) {
                        if template.rows.len() < 256 {
                            template.range(
                                &self.namespace,
                                index,
                                &mut pending,
                                template.rows.len(),
                                &mut ranges,
                            );
                            template.rows.push(json!(["text", index, text]));
                        } else {
                            pending.push(TemplateChild::Value(quote(&text)));
                        }
                    }
                }
                other => {
                    pending.extend(self.child(other, code, true).map(TemplateChild::Value));
                }
            }
        }
        template.range(&self.namespace, index, &mut pending, 0, &mut ranges);
        template.bindings.push_str(&ranges);
        if let Some(value) = node_ref {
            let rt = &self.namespace;
            if matches!(primitive, Primitive::Host(_)) {
                let _ = write!(template.refs, "{rt}.ref(() => ({value}), {node});");
            } else {
                let _ = write!(template.refs, "{rt}.bindNodeRef({node}, () => ({value}));");
            }
        }
    }

    fn template_semantics(
        &self,
        node: &str,
        semantics: [Option<(String, bool)>; 3],
        code: &mut String,
    ) {
        let dynamic = semantics.iter().flatten().any(|(_, dynamic)| *dynamic);
        let [role, label, hint] = semantics.map(|value| {
            value.map_or_else(|| "undefined".to_owned(), |(value, _)| format!("({value})"))
        });
        let rt = &self.namespace;
        if dynamic {
            let _ = write!(
                code,
                "{rt}.bindBoxSemantics({node}, () => [{role}, {label}, {hint}]);"
            );
        } else {
            let _ = write!(
                code,
                "{rt}.setBoxSemantics({node}, {role}, {label}, {hint});"
            );
        }
    }

    fn element(&mut self, element: JSXElement<'a>, code: &mut Code<'a>) -> String {
        self.used = true;
        if let Some(tag) = self.template_host(&element)
            && (!matches!(tag, Primitive::Host(_))
                || element.opening_element.attributes.iter().any(|attribute| {
                    matches!(attribute, JSXAttributeItem::Attribute(attribute)
                        if matches!(&attribute.name, JSXAttributeName::Identifier(name) if name.name == "ref"))
                })
                || element.children.iter().any(|child| match child {
                    JSXChild::Element(child) => self.template_host(child).is_some() || self.direct_list(child).is_some(),
                    JSXChild::Text(text) => clean_text(&text.value).is_some(),
                    _ => false,
                }))
        {
            let mut template = Template::default();
            self.template_node(element, tag, 0, &mut template, code);
            let index = intern(
                &mut self.templates,
                serde_json::Value::Array(template.rows).to_string(),
            );
            let rt = &self.namespace;
            let template_ref = self.pool_ref("template", index);
            if template.bindings.is_empty() && template.refs.is_empty() {
                return format!("{rt}_instantiateTemplate({template_ref})");
            }
            let Template { bindings, refs, .. } = template;
            return format!("{rt}.bindTemplate({template_ref}, {rt}_nodes => {{ {bindings}{refs} }})");
        }
        let span = element.span;
        let opening = element.opening_element.unbox();
        let (name, host, virtual_list) = match &opening.name {
            JSXElementName::Identifier(id) => {
                let tag = id.name.as_str();
                let replacement = match tag {
                    "view" => "View",
                    "stack" => "Stack",
                    "scroll" => "Scroll",
                    "image" => "Image",
                    _ => "a PascalCase component",
                };
                self.fail(
                    id.span,
                    format!(
                        "<{tag}> is not an Impasto component. Import {replacement} explicitly."
                    ),
                );
                return "undefined".to_owned();
            }
            JSXElementName::IdentifierReference(id) => {
                let scope = self
                    .scopes
                    .last()
                    .copied()
                    .unwrap_or(self.scoping.root_scope_id());
                let symbol = self.scoping.find_binding(scope, id.name);
                (
                    id.name.to_string(),
                    symbol.and_then(|symbol| match self.primitives.get(&symbol)? {
                        Primitive::Host(tag) => Some(*tag),
                        _ => None,
                    }),
                    symbol.is_some_and(|symbol| self.virtual_lists.contains(&symbol)),
                )
            }
            JSXElementName::MemberExpression(member) => (member_name(member), None, false),
            other => {
                self.fail(other.span(), "expected an Impasto JSX component");
                return "undefined".to_owned();
            }
        };
        let attributes = self.attributes(
            opening.attributes,
            code,
            host.map(Primitive::Host),
            virtual_list,
        );
        let multiple = element
            .children
            .iter()
            .filter(|child| match child {
                JSXChild::Text(text) => clean_text(&text.value).is_some(),
                JSXChild::ExpressionContainer(child) => {
                    !matches!(child.expression, JSXExpression::EmptyExpression(_))
                }
                _ => true,
            })
            .take(2)
            .count()
            > 1;
        let children = self.children(element.children, code, host.is_some() || multiple);
        let children = match children.as_slice() {
            [] => None,
            [one] => Some(one.clone()),
            _ => Some(format!("[{}]", children.join(", "))),
        };
        if let Some(tag) = host {
            return self.host(tag, attributes, children, span);
        }
        let mut attributes = attributes;
        if let Some(value) = children {
            attributes.push(Attribute::Prop {
                name: "children".to_owned(),
                value,
                dynamic: true,
            });
        }
        let parts = property_groups(attributes);
        let runtime = &self.namespace;
        let props = match parts.as_slice() {
            [] => "{}".to_owned(),
            [one] => one.clone(),
            _ => format!("{runtime}.mergeProps({})", parts.join(", ")),
        };
        if self.hot {
            format!("globalThis.__impasto_hot.render({runtime}.createComponent, {name}, {props})")
        } else {
            format!("{runtime}.createComponent({name}, {props})")
        }
    }

    fn host(
        &mut self,
        tag: &str,
        attributes: Vec<Attribute>,
        children: Option<String>,
        span: Span,
    ) -> String {
        let rt = self.namespace.clone();
        let node = format!("{rt}_node");
        let mut code = format!(
            "(() => {{ const {node} = {rt}.createElement({});",
            quote(tag)
        );
        self.source_location(&mut code, &node, span);
        let has_spread = attributes
            .iter()
            .any(|attribute| matches!(attribute, Attribute::Spread(_)));
        if has_spread {
            let mut spread_properties = Vec::new();
            for attribute in attributes {
                if let Attribute::Prop { name, value, .. } = &attribute
                    && name == "ref"
                {
                    let _ = write!(code, "{rt}.ref(() => ({value}), {node});");
                } else {
                    spread_properties.push(attribute);
                }
            }
            let parts = property_groups(spread_properties);
            let parts = parts.join(", ");
            let skip = children.is_some();
            let _ = write!(code, "{rt}.spread({node}, [{parts}], {skip});");
        } else {
            for attribute in attributes {
                self.host_property(&mut code, &node, attribute, children.is_some());
            }
        }
        if let Some(children) = children {
            let _ = write!(code, "{rt}.insert({node}, ({children}));");
        }
        let _ = write!(code, "return {node}; }})()");
        code
    }

    fn style_patches(&mut self, code: &mut String, node: &str, patches: Vec<StylePatch>) {
        let rt = &self.namespace;
        for StylePatch {
            selector,
            scalar,
            value,
            dynamic,
        } in patches
        {
            if scalar {
                if dynamic {
                    let _ = write!(
                        code,
                        "{rt}.bindScalarStyle({node}, {selector}, () => ({value}));"
                    );
                } else {
                    let _ = write!(
                        code,
                        "{rt}.patchScalarStyle({node}, {selector}, ({value}));"
                    );
                }
            } else if dynamic {
                let _ = write!(
                    code,
                    "{rt}.bindInlineStyle({node}, {selector}, () => ({value}));"
                );
            } else {
                self.used_patches = true;
                let _ = write!(
                    code,
                    "{rt}_patchInlineStyle({node}, {selector}, ({value}));"
                );
            }
        }
    }

    fn host_property(
        &mut self,
        code: &mut String,
        node: &str,
        attribute: Attribute,
        has_children: bool,
    ) {
        let rt = &self.namespace;
        let (name, value, dynamic) = match attribute {
            Attribute::Prop {
                name,
                value,
                dynamic,
            } => (name, value, dynamic),
            Attribute::StaticStyle {
                index,
                patches,
                guards,
            } => {
                const_guards(code, guards);
                if let Some(index) = index {
                    self.used_styles.insert(index);
                    let style = self.pool_ref("style", index);
                    let _ = write!(code, "{rt}_applyStaticStyle({node}, {style});");
                }
                self.style_patches(code, node, patches);
                return;
            }
            Attribute::StaticClass(index) => {
                self.used_classes.insert(index);
                let class = self.pool_ref("class", index);
                let _ = write!(code, "{rt}_applyStaticClass({node}, {class});");
                return;
            }
            Attribute::ClassChoice { selection, indices } => {
                self.used_classes.extend(indices);
                let _ = write!(code, "{rt}.bindStaticClass({node}, () => {selection});");
                return;
            }
            Attribute::ClassList { entries, dynamic } => {
                if entries.is_empty() {
                    return;
                }
                let mut values = Vec::new();
                for (index, test) in entries {
                    self.used_classes.insert(index);
                    let class = self.pool_ref("class", index);
                    values.push(format!("({test}) ? {class} : undefined"));
                }
                let values = values.join(", ");
                if dynamic {
                    let _ = write!(code, "{rt}.bindStaticClassList({node}, () => [{values}]);");
                } else {
                    self.used_class_list = true;
                    let _ = write!(code, "{rt}_applyStaticClassList({node}, [{values}]);");
                }
                return;
            }
            Attribute::Spread(_) => return,
        };
        let key = quote(&name);
        match name.as_str() {
            "children" if !has_children => {
                let _ = write!(code, "{rt}.insert({node}, () => ({value}));");
            }
            "children" => {}
            "ref" => {
                let _ = write!(code, "{rt}.ref(() => ({value}), {node});");
            }
            _ if dynamic => {
                let _ = write!(code, "{rt}.bindProp({node}, {key}, () => ({value}));");
            }
            _ => {
                let _ = write!(code, "{rt}.setProp({node}, {key}, ({value}));");
            }
        }
    }
}

fn const_guards(code: &mut String, guards: Vec<String>) {
    for name in guards {
        // Preserve the original lexical read, including its temporal dead zone.
        let _ = write!(code, "void {name};");
    }
}

pub(super) fn style_primitive(semantic: &Semantic<'_>, scope: ScopeId, name: &str) -> bool {
    use oxc::ast::AstKind;

    let scoping = semantic.scoping();
    let nodes = semantic.nodes();
    let Some(symbol) = scoping.find_binding(scope, name.into()) else {
        return false;
    };
    let declaration = scoping.symbol_declaration(symbol);
    matches!(nodes.kind(declaration), AstKind::ImportSpecifier(specifier)
        if matches!(nodes.parent_kind(declaration), AstKind::ImportDeclaration(import)
            if Primitive::imported(import.source.value.as_str(), specifier.imported.name().as_str()).is_some()))
}

fn semantics_index(name: &str) -> Option<usize> {
    match name {
        "role" => Some(0),
        "accessibilityLabel" => Some(1),
        "accessibilityHint" => Some(2),
        _ => None,
    }
}

fn is_box_prop(name: &str) -> bool {
    matches!(
        name,
        "style"
            | "class"
            | "classList"
            | "focusable"
            | "recognizers"
            | "onClick"
            | "onPress"
            | "onRelease"
            | "onFocus"
            | "onBlur"
            | "onKey"
            | "onText"
            | "onTextReplaced"
            | "onLink"
            | "onComposing"
            | "onDragBegin"
            | "onDragBy"
            | "onDragEnd"
            | "onHover"
            | "onLeave"
            | "onSecondaryClick"
            | "onZoom"
            | "onLongPress"
            | "onWheel"
    ) || semantics_index(name).is_some()
}

fn text_expression(expression: &Expression<'_>) -> bool {
    struct ContainsJsx(bool);
    impl<'a> Visit<'a> for ContainsJsx {
        fn visit_expression(&mut self, expression: &Expression<'a>) {
            if matches!(
                expression,
                Expression::JSXElement(_) | Expression::JSXFragment(_)
            ) {
                self.0 = true;
            } else if !self.0 {
                oxc::ast_visit::walk::walk_expression(self, expression);
            }
        }
    }
    let mut jsx = ContainsJsx(false);
    jsx.visit_expression(expression);
    !jsx.0
}

fn property_groups(attributes: Vec<Attribute>) -> Vec<String> {
    let mut groups = Vec::new();
    let mut properties = Vec::new();
    for attribute in attributes {
        match attribute {
            Attribute::StaticStyle { .. }
            | Attribute::StaticClass(_)
            | Attribute::ClassChoice { .. }
            | Attribute::ClassList { .. } => {}
            Attribute::Spread(source) => {
                if !properties.is_empty() {
                    groups.push(format!("{{{}}}", properties.join(", ")));
                    properties.clear();
                }
                groups.push(format!("({source})"));
            }
            Attribute::Prop {
                name,
                value,
                dynamic,
            } => {
                let key = quote(&name);
                let property = if dynamic {
                    format!("get {key}() {{ return ({value}); }}")
                } else {
                    format!("[{key}]: ({value})")
                };
                properties.push(property);
            }
        }
    }
    if !properties.is_empty() {
        groups.push(format!("{{{}}}", properties.join(", ")));
    }
    groups
}

fn plain_property<'b, 'a>(
    field: &'b ObjectPropertyKind<'a>,
) -> Option<(std::borrow::Cow<'a, str>, &'b Expression<'a>)> {
    let ObjectPropertyKind::ObjectProperty(property) = field else {
        return None;
    };
    if property.computed || property.method || property.kind != PropertyKind::Init {
        return None;
    }
    let name = property.key.static_name()?;
    (name != "__proto__").then_some((name, &property.value))
}

type StyleFields<'b, 'a> = (serde_json::Value, Vec<StylePatchSyntax<'b, 'a>>);

type FragmentFields<'b, 'a> = Vec<(String, FieldValue<'b, 'a>)>;

enum StylePatchSyntax<'b, 'a> {
    Property(String, &'b Expression<'a>),
    Fragment(Vec<FragmentFields<'b, 'a>>),
}

#[derive(Clone)]
enum FieldValue<'b, 'a> {
    Static(serde_json::Value),
    Dynamic(&'b Expression<'a>),
}

impl<'a> StylePatchSyntax<'_, 'a> {
    fn lower(self, allocator: &'a Allocator, code: &mut Code<'a>) -> StylePatch {
        match self {
            Self::Property(key, expression) => {
                let op = engine::style::codec::ScalarStyleOp::parse(&key);
                StylePatch {
                    selector: op.map_or_else(|| quote(&key), |op| op.code().to_string()),
                    scalar: op.is_some(),
                    value: code.expression(expression.clone_in(allocator)),
                    dynamic: is_dynamic(expression),
                }
            }
            Self::Fragment(fragments) => {
                let mut dynamic = false;
                let mut keys = Vec::new();
                let mut values = Vec::new();
                for fields in fragments {
                    let mut properties = Vec::new();
                    for (name, value) in fields {
                        let name = quote(&name);
                        let value = match value {
                            FieldValue::Static(value) => value.to_string(),
                            FieldValue::Dynamic(expression) => {
                                dynamic |= is_dynamic(expression);
                                code.expression(expression.clone_in(allocator))
                            }
                        };
                        properties.push(format!("{name}: ({value})"));
                        keys.push(name);
                    }
                    values.push(format!("{{{}}}", properties.join(", ")));
                }
                StylePatch {
                    selector: format!("[{}]", keys.join(", ")),
                    scalar: false,
                    value: format!("[{}]", values.join(", ")),
                    dynamic,
                }
            }
        }
    }
}

fn static_style<'b, 'a>(
    expression: &'b Expression<'a>,
    constants: &crate::styles::Constants,
    guards: &mut Vec<String>,
) -> Option<StyleFields<'b, 'a>> {
    use engine::style::codec::{self, Value};

    if let Some(value) = constants.literal(expression, guards) {
        return Some((value, Vec::new()));
    }
    let mut fragments = Vec::new();
    style_fragments(expression, constants, guards, &mut fragments)?;
    let mut base = Vec::new();
    let mut fields = Vec::new();
    let mut owners = HashMap::<_, Vec<usize>>::new();
    for (fragment, properties) in fragments.into_iter().enumerate() {
        let mut values = serde_json::Map::new();
        for (name, value) in properties {
            if let FieldValue::Static(value) = &value {
                values.insert(name.clone(), value.clone());
            }
            let declarations = codec::declarations(&name, &Value::Null).ok()?;
            let keys = declarations
                .iter()
                .map(engine::style::Declaration::key)
                .collect::<Vec<_>>();
            for key in &keys {
                owners.entry(*key).or_default().push(fields.len());
            }
            fields.push((fragment, name, value, keys));
        }
        if !values.is_empty() {
            base.push(serde_json::Value::Object(values));
        }
    }

    let mut patches = Vec::new();
    let mut seen = HashSet::new();
    for (index, (_, name, value, _)) in fields.iter().enumerate() {
        let FieldValue::Dynamic(expression) = value else {
            continue;
        };
        if !seen.insert(index) {
            continue;
        }

        // Shorthands and aliases may own the same longhands. Evaluate each
        // connected group with the original codec precedence and order.
        let mut pending = vec![index];
        let mut group = Vec::new();
        while let Some(index) = pending.pop() {
            group.push(index);
            for key in &fields[index].3 {
                pending.extend(
                    owners
                        .get(key)
                        .into_iter()
                        .flatten()
                        .copied()
                        .filter(|index| seen.insert(*index)),
                );
            }
        }
        if group.len() == 1 {
            patches.push(StylePatchSyntax::Property(name.clone(), expression));
        } else {
            group.sort_unstable();
            let mut fragments = std::collections::BTreeMap::<_, Vec<_>>::new();
            for index in group {
                let (fragment, key, value, _) = &fields[index];
                fragments
                    .entry(*fragment)
                    .or_default()
                    .push((key.clone(), value.clone()));
            }
            patches.push(StylePatchSyntax::Fragment(
                fragments.into_values().collect(),
            ));
        }
    }
    Some((serde_json::Value::Array(base), patches))
}

fn style_fragments<'b, 'a>(
    expression: &'b Expression<'a>,
    constants: &crate::styles::Constants,
    guards: &mut Vec<String>,
    fragments: &mut Vec<FragmentFields<'b, 'a>>,
) -> Option<()> {
    match expression.get_inner_expression() {
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                if matches!(
                    element,
                    ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_)
                ) {
                    return None;
                }
                style_fragments(element.to_expression(), constants, guards, fragments)?;
            }
        }
        Expression::ObjectExpression(object) => {
            let mut names = HashSet::new();
            let mut fields = Vec::new();
            for property in &object.properties {
                let (name, expression) = plain_property(property)?;
                if !names.insert(name.clone()) {
                    return None;
                }
                let value = constants
                    .literal(expression, guards)
                    .or_else(|| constants.theme_token(&name, expression))
                    .map_or(FieldValue::Dynamic(expression), FieldValue::Static);
                fields.push((name.into_owned(), value));
            }
            fragments.push(fields);
        }
        _ => {
            fn add_static<'b, 'a>(
                value: serde_json::Value,
                fragments: &mut Vec<FragmentFields<'b, 'a>>,
            ) -> Option<()> {
                match value {
                    serde_json::Value::Object(fields) => fragments.push(
                        fields
                            .into_iter()
                            .map(|(name, value)| (name, FieldValue::Static(value)))
                            .collect(),
                    ),
                    serde_json::Value::Array(values) => {
                        for value in values {
                            add_static(value, fragments)?;
                        }
                    }
                    serde_json::Value::Null | serde_json::Value::Bool(false) => {}
                    _ => return None,
                }
                Some(())
            }
            add_static(constants.literal(expression, guards)?, fragments)?;
        }
    }
    Some(())
}

fn member_name(member: &JSXMemberExpression<'_>) -> String {
    let object = match &member.object {
        JSXMemberExpressionObject::IdentifierReference(reference) => reference.name.to_string(),
        JSXMemberExpressionObject::MemberExpression(inner) => member_name(inner),
        JSXMemberExpressionObject::ThisExpression(_) => "this".to_owned(),
    };
    format!("{object}.{}", member.property.name)
}

fn is_dynamic(expression: &Expression<'_>) -> bool {
    #[derive(Default)]
    struct Reads(bool);

    impl<'a> Visit<'a> for Reads {
        fn visit_expression(&mut self, expression: &Expression<'a>) {
            match expression {
                Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {}
                Expression::CallExpression(_)
                | Expression::NewExpression(_)
                | Expression::ChainExpression(_)
                | Expression::StaticMemberExpression(_)
                | Expression::ComputedMemberExpression(_)
                | Expression::PrivateFieldExpression(_)
                | Expression::TaggedTemplateExpression(_)
                | Expression::JSXElement(_)
                | Expression::JSXFragment(_) => self.0 = true,
                _ => oxc::ast_visit::walk::walk_expression(self, expression),
            }
        }
    }
    let mut reads = Reads::default();
    reads.visit_expression(expression);
    reads.0
}

fn quote(value: &str) -> String {
    serde_json::Value::String(value.to_owned()).to_string()
}

fn intern(pool: &mut Vec<String>, value: String) -> usize {
    if let Some(index) = pool.iter().position(|entry| entry == &value) {
        return index;
    }
    let index = pool.len();
    pool.push(value);
    index
}

fn clean_text(raw: &str) -> Option<String> {
    let raw = raw.replace('\t', " ");
    let lines: Vec<_> = raw.split(['\r', '\n']).collect();
    let text = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = if index == 0 { line } else { line.trim_start() };
            let line = if index + 1 == lines.len() {
                line
            } else {
                line.trim_end()
            };
            (!line.is_empty()).then_some(line)
        })
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}

struct Splice<'a> {
    arguments: Vec<Option<Expression<'a>>>,
    span: Span,
}

impl<'a> VisitMut<'a> for Splice<'a> {
    fn visit_expression(&mut self, expression: &mut Expression<'a>) {
        if let Expression::Identifier(id) = expression
            && let Some(index) = id
                .name
                .strip_prefix("__jsx_argument_")
                .and_then(|value| value.parse::<usize>().ok())
            && let Some(value) = self.arguments.get_mut(index).and_then(Option::take)
        {
            *expression = value;
        } else {
            walk_mut::walk_expression(self, expression);
        }
    }

    fn visit_span(&mut self, span: &mut Span) {
        *span = self.span;
    }
}
