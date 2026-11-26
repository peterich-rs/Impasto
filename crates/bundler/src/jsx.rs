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
            Expression, ImportDeclarationSpecifier, JSXAttributeItem, JSXAttributeName,
            JSXAttributeValue, JSXChild, JSXElement, JSXElementName, JSXExpression,
            JSXMemberExpression, JSXMemberExpressionObject, ObjectPropertyKind, Program,
            PropertyKind, Statement,
        },
        builder::AstBuilder,
    },
    ast_visit::{Visit, VisitMut, walk_mut},
    diagnostics::OxcDiagnostic,
    parser::Parser,
    semantic::Scoping,
    span::{GetSpan, SourceType, Span},
    syntax::scope::{ScopeFlags, ScopeId},
};

const RENDERER: &str = "@impasto/solid-renderer";

pub(crate) fn lower<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    scoping: &Scoping,
    source: &str,
    hot: bool,
) -> Result<(), OxcDiagnostic> {
    let mut compiler = Compiler {
        allocator,
        scoping,
        scopes: Vec::new(),
        primitives: HashMap::new(),
        virtual_lists: HashSet::new(),
        namespace: "__impasto_jsx".to_owned(),
        used: false,
        styles: Vec::new(),
        used_styles: HashSet::new(),
        classes: Vec::new(),
        used_classes: HashSet::new(),
        templates: Vec::new(),
        error: None,
        hot,
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
            if import.source.value == "@impasto/solid-ui"
                && specifier.imported.name() == "VirtualList"
                && let Some(symbol) = specifier.local.symbol_id.get()
            {
                compiler.virtual_lists.insert(symbol);
            }
            if import.source.value != "@impasto/solid-renderer/host" {
                continue;
            }
            let tag = match specifier.imported.name().as_str() {
                "View" => "view",
                "Stack" => "stack",
                "Scroll" => "scroll",
                "Text" => "text",
                _ => continue,
            };
            if let Some(symbol) = specifier.local.symbol_id.get() {
                compiler.primitives.insert(symbol, tag);
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
                "import {{ defineStyle as {namespace}_defineStyle, applyStaticStyle as {namespace}_applyStaticStyle }} from 'scene';"
            );
            let styles = compiler
                .styles
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    if compiler.used_styles.contains(&index) {
                        format!("{namespace}_defineStyle({})", quote(value))
                    } else {
                        "undefined".to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(head, "const {namespace}_styles = [{styles}];");
        }
        if !compiler.used_classes.is_empty() {
            let _ = write!(
                head,
                "import {{ defineClass as {namespace}_defineClass, applyStaticClass as {namespace}_applyStaticClass }} from 'scene';"
            );
            let classes = compiler
                .classes
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    if compiler.used_classes.contains(&index) {
                        format!("{namespace}_defineClass({})", quote(value))
                    } else {
                        "undefined".to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(head, "const {namespace}_classes = [{classes}];");
        }
        if !compiler.templates.is_empty() {
            let _ = write!(
                head,
                "import {{ defineTemplate as {namespace}_defineTemplate, instantiateTemplate as {namespace}_instantiateTemplate }} from 'scene';"
            );
            let templates = compiler
                .templates
                .iter()
                .map(|template| format!("{namespace}_defineTemplate({template})"))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(head, "const {namespace}_templates = [{templates}];");
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
    scopes: Vec<ScopeId>,
    primitives: HashMap<oxc::syntax::symbol::SymbolId, &'static str>,
    virtual_lists: HashSet<oxc::syntax::symbol::SymbolId>,
    namespace: String,
    used: bool,
    styles: Vec<String>,
    used_styles: HashSet<usize>,
    classes: Vec<String>,
    used_classes: HashSet<usize>,
    templates: Vec<String>,
    error: Option<OxcDiagnostic>,
    hot: bool,
}

#[derive(Default)]
struct Template {
    rows: Vec<serde_json::Value>,
    bindings: String,
    next_range: usize,
}

impl Template {
    fn range(
        &mut self,
        rt: &str,
        parent: usize,
        pending: &mut Vec<String>,
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
        let _ = write!(
            self.bindings,
            "const {rt}_range{range} = [{}];",
            pending.join(", ")
        );
        let _ = write!(
            ranges,
            "{rt}.insert({rt}_nodes[{parent}], {rt}_range{range}, {marker});"
        );
        pending.clear();
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
        index: usize,
        opacity: Option<String>,
    },
    StaticClass(usize),
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
    fn static_class(&mut self, span: Span, text: &str, can_pool: bool) -> Option<Attribute> {
        if can_pool {
            match engine::style::encode_static_class(text) {
                Ok(encoded) => Some(Attribute::StaticClass(intern(&mut self.classes, encoded))),
                Err(error) => {
                    self.fail(span, error);
                    None
                }
            }
        } else {
            if let Err(error) = engine::style::parse_class(text) {
                self.fail(span, error.to_string());
            }
            None
        }
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
        host: Option<&str>,
        virtual_list: bool,
    ) -> Vec<Attribute> {
        let can_pool = host.is_some()
            && !opening
                .iter()
                .any(|attr| matches!(attr, JSXAttributeItem::SpreadAttribute(_)));
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
                        if can_pool
                            && name == "style"
                            && let Some((literal, opacity)) = static_style(&expression)
                        {
                            let style = crate::styles::style_value(&literal);
                            match engine::style::codec::from_value(&style)
                                .map_err(|error| error.to_string())
                                .and_then(|style| engine::style::static_pool::encode_style(&style))
                            {
                                Ok(encoded) => {
                                    let index = intern(&mut self.styles, encoded);
                                    let opacity = opacity.map(|value| {
                                        code.expression(value.clone_in(self.allocator))
                                    });
                                    return Some(Attribute::StaticStyle { index, opacity });
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

    fn template_host(&self, element: &JSXElement<'_>) -> Option<&'static str> {
        let opening = &element.opening_element;
        let JSXElementName::IdentifierReference(id) = &opening.name else {
            return None;
        };
        let scope = self
            .scopes
            .last()
            .copied()
            .unwrap_or(self.scoping.root_scope_id());
        let tag = *self
            .primitives
            .get(&self.scoping.find_binding(scope, id.name)?)?;
        // A ref can observe a node before its children have been mounted.
        let mut names = HashSet::new();
        opening
            .attributes
            .iter()
            .all(|attribute| {
                let JSXAttributeItem::Attribute(attribute) = attribute else {
                    return false;
                };
                let JSXAttributeName::Identifier(name) = &attribute.name else {
                    return false;
                };
                !matches!(name.name.as_str(), "ref" | "children")
                    && names.insert(name.name.as_str())
            })
            .then_some(tag)
    }

    fn template_node(
        &mut self,
        element: JSXElement<'a>,
        tag: &str,
        parent: usize,
        template: &mut Template,
        code: &mut Code<'a>,
    ) {
        use serde_json::json;

        let index = template.rows.len();
        template.rows.push(serde_json::Value::Null);
        let node = format!("{}_nodes[{index}]", self.namespace);
        let mut style = None;
        let mut class = None;
        for attribute in self.attributes(
            element.opening_element.unbox().attributes,
            code,
            Some(tag),
            false,
        ) {
            match attribute {
                Attribute::StaticStyle {
                    index,
                    opacity: None,
                } => style = Some(index),
                Attribute::StaticClass(index) => class = Some(index),
                attribute => self.host_property(&mut template.bindings, &node, attribute, false),
            }
        }
        template.rows[index] = json!([
            tag,
            parent,
            (tag == "text").then_some(""),
            style.map(|index| &self.styles[index]),
            class.map(|index| &self.classes[index])
        ]);

        // Evaluate child expressions in order, then install their ranges. Each run
        // of expressions uses the next template node as its insertion marker.
        let mut pending = Vec::new();
        let mut ranges = String::new();
        for child in element.children {
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
                            pending.push(quote(&text));
                        }
                    }
                }
                other => {
                    pending.extend(self.child(other, code, true));
                }
            }
        }
        template.range(&self.namespace, index, &mut pending, 0, &mut ranges);
        template.bindings.push_str(&ranges);
    }

    fn element(&mut self, element: JSXElement<'a>, code: &mut Code<'a>) -> String {
        self.used = true;
        if let Some(tag) = self.template_host(&element)
            && element.children.iter().any(|child| match child {
                JSXChild::Element(child) => self.template_host(child).is_some(),
                JSXChild::Text(text) => clean_text(&text.value).is_some(),
                _ => false,
            })
        {
            let mut template = Template::default();
            self.template_node(element, tag, 0, &mut template, code);
            let index = intern(
                &mut self.templates,
                serde_json::Value::Array(template.rows).to_string(),
            );
            let rt = &self.namespace;
            let mount = format!("{rt}_instantiateTemplate({rt}_templates[{index}]");
            if template.bindings.is_empty() {
                return format!("{mount})");
            }
            return format!(
                "(() => {{ const {rt}_nodes = {mount}, true); {} return {rt}_nodes[0]; }})()",
                template.bindings
            );
        }
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
                    symbol.and_then(|symbol| self.primitives.get(&symbol).copied()),
                    symbol.is_some_and(|symbol| self.virtual_lists.contains(&symbol)),
                )
            }
            JSXElementName::MemberExpression(member) => (member_name(member), None, false),
            other => {
                self.fail(other.span(), "expected an Impasto JSX component");
                return "undefined".to_owned();
            }
        };
        let attributes = self.attributes(opening.attributes, code, host, virtual_list);
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
            return self.host(tag, attributes, children);
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

    fn host(&mut self, tag: &str, attributes: Vec<Attribute>, children: Option<String>) -> String {
        let rt = self.namespace.clone();
        let node = format!("{rt}_node");
        let mut code = format!(
            "(() => {{ const {node} = {rt}.createElement({});",
            quote(tag)
        );
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
            Attribute::StaticStyle { index, opacity } => {
                self.used_styles.insert(index);
                if let Some(opacity) = opacity {
                    let _ = write!(
                        code,
                        "{rt}.effect(() => ({opacity}), next => {rt}_applyStaticStyle({node}, {rt}_styles[{index}], next));"
                    );
                } else {
                    let _ = write!(code, "{rt}_applyStaticStyle({node}, {rt}_styles[{index}]);");
                }
                return;
            }
            Attribute::StaticClass(index) => {
                self.used_classes.insert(index);
                let _ = write!(
                    code,
                    "{rt}_applyStaticClass({node}, {rt}_classes[{index}]);"
                );
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
                let _ = write!(
                    code,
                    "{rt}.effect(() => ({value}), (next, previous) => {rt}.setProp({node}, {key}, next, previous));"
                );
            }
            _ => {
                let _ = write!(code, "{rt}.setProp({node}, {key}, ({value}));");
            }
        }
    }
}

fn property_groups(attributes: Vec<Attribute>) -> Vec<String> {
    let mut groups = Vec::new();
    let mut properties = Vec::new();
    for attribute in attributes {
        match attribute {
            Attribute::StaticStyle { .. } | Attribute::StaticClass(_) => {}
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

fn static_style<'b, 'a>(
    expression: &'b Expression<'a>,
) -> Option<(serde_json::Value, Option<&'b Expression<'a>>)> {
    if let Some(value) = crate::styles::literal(expression) {
        return Some((value, None));
    }
    let Expression::ObjectExpression(object) = expression.without_parentheses() else {
        return None;
    };
    let mut values = serde_json::Map::new();
    let mut opacity = None;
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            return None;
        };
        if property.computed || property.method || property.kind != PropertyKind::Init {
            return None;
        }
        let key = property.key.static_name()?;
        if key == "__proto__"
            || values.contains_key(key.as_ref())
            || (key == "opacity" && opacity.is_some())
        {
            return None;
        }
        if let Some(value) = crate::styles::literal(&property.value) {
            values.insert(key.into_owned(), value);
        } else if key == "opacity" && is_dynamic(&property.value) {
            opacity = Some(&property.value);
        } else {
            return None;
        }
    }
    (!values.is_empty()).then_some((serde_json::Value::Object(values), opacity))
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
