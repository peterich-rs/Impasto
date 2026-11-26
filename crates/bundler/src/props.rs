//! Diagnose synchronous reads and owner-dependent calls in reactive code.

use std::collections::{HashMap, HashSet};

use oxc::{
    ast::{
        AstKind,
        ast::{
            ArrowFunctionExpression, BindingPattern, CallExpression, ExportDefaultDeclaration,
            ExportDefaultDeclarationKind, Expression, FormalParameters, Function, FunctionBody,
            ImportDeclarationSpecifier, Program, Statement, UnaryOperator, VariableDeclarator,
        },
    },
    ast_visit::{Visit, walk},
    diagnostics::OxcDiagnostic,
    semantic::{Scoping, Semantic},
    span::GetSpan,
    syntax::{scope::ScopeFlags, symbol::SymbolId},
};

const DESTRUCTURE: &str = "destructuring component props reads reactive getters once; read each property from props where it is used";
const EAGER_PROP: &str = "this component reads a prop only once; read it inside JSX, a memo, or a reactive control-flow accessor";
const EAGER_SIGNAL: &str = "this component reads an accessor only once; move the call inside JSX, a memo, or a reactive control-flow accessor";
const LOST_OWNER: &str = "the reactive owner is unavailable after await; register cleanup or effects before awaiting, or restore the owner explicitly";

pub(super) fn check(program: &Program<'_>, semantic: &Semantic<'_>) -> Vec<OxcDiagnostic> {
    let imports = reactive_imports(program);
    let scoping = semantic.scoping();
    let mut signals = HashSet::new();
    for node in semantic.nodes().iter() {
        let AstKind::VariableDeclarator(variable) = node.kind() else {
            continue;
        };
        let Some(Expression::CallExpression(call)) =
            variable.init.as_ref().map(Expression::get_inner_expression)
        else {
            continue;
        };
        let Some(name) = callee(scoping, call).and_then(|symbol| imports.get(&symbol)) else {
            continue;
        };
        let binding = match (&variable.id, *name) {
            (BindingPattern::ArrayPattern(pattern), "createSignal") => {
                pattern.elements.first().and_then(Option::as_ref)
            }
            (binding @ BindingPattern::BindingIdentifier(_), "createMemo") => Some(binding),
            _ => None,
        };
        if let Some(BindingPattern::BindingIdentifier(id)) = binding
            && let Some(symbol) = id.symbol_id.get()
        {
            signals.insert(symbol);
        }
    }
    let mut visitor = Components {
        warnings: Vec::new(),
        scoping,
        imports: &imports,
        signals: &signals,
    };
    visitor.visit_program(program);
    visitor.warnings
}

fn reactive_imports<'a>(program: &Program<'a>) -> HashMap<SymbolId, &'a str> {
    let mut imports = HashMap::new();
    for statement in &program.body {
        let Statement::ImportDeclaration(import) = statement else {
            continue;
        };
        if import.import_kind.is_type()
            || !matches!(
                import.source.value.as_str(),
                "solid-js" | "@impasto/reactive" | "reactive"
            )
        {
            continue;
        }
        for item in import.specifiers.iter().flatten() {
            if let ImportDeclarationSpecifier::ImportSpecifier(specifier) = item
                && !specifier.import_kind.is_type()
                && let Some(symbol) = specifier.local.symbol_id.get()
            {
                imports.insert(symbol, specifier.imported.name().as_str());
            }
        }
    }
    imports
}

fn referenced(scoping: &Scoping, expression: &Expression<'_>) -> Option<SymbolId> {
    let Expression::Identifier(identifier) = expression.get_inner_expression() else {
        return None;
    };
    scoping
        .get_reference(identifier.reference_id.get()?)
        .symbol_id()
}

fn callee(scoping: &Scoping, call: &CallExpression<'_>) -> Option<SymbolId> {
    referenced(scoping, &call.callee)
}

struct Components<'s> {
    warnings: Vec<OxcDiagnostic>,
    scoping: &'s Scoping,
    imports: &'s HashMap<SymbolId, &'s str>,
    signals: &'s HashSet<SymbolId>,
}

impl Components<'_> {
    fn jsx_params(&mut self, params: &FormalParameters<'_>) -> Option<SymbolId> {
        match &params.items.first()?.pattern {
            BindingPattern::ObjectPattern(pattern) => {
                self.warnings
                    .push(OxcDiagnostic::warn(DESTRUCTURE).with_label(pattern.span));
                None
            }
            BindingPattern::BindingIdentifier(id) => id.symbol_id.get(),
            _ => None,
        }
    }

    fn check_component(&mut self, params: &FormalParameters<'_>, body: &FunctionBody<'_>) {
        let mut jsx = HasJsx(false);
        jsx.visit_function_body(body);
        if jsx.0 {
            let props = self.jsx_params(params);
            EagerReads {
                warnings: &mut self.warnings,
                scoping: self.scoping,
                props,
                signals: self.signals,
            }
            .visit_function_body(body);
        }
    }

    fn check_arrow(&mut self, function: &ArrowFunctionExpression<'_>) {
        if let Some(body) = function.body.as_function_body() {
            self.check_component(&function.params, body);
        } else if let Some(expression) = function.body.as_expression() {
            let mut jsx = HasJsx(false);
            jsx.visit_expression(expression);
            if jsx.0 {
                let props = self.jsx_params(&function.params);
                EagerReads {
                    warnings: &mut self.warnings,
                    scoping: self.scoping,
                    props,
                    signals: self.signals,
                }
                .visit_expression(expression);
            }
        }
    }

    fn owner(&mut self) -> OwnerCalls<'_> {
        OwnerCalls {
            warnings: &mut self.warnings,
            scoping: self.scoping,
            imports: self.imports,
            suspended: false,
        }
    }
}

fn component_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
}

impl<'a> Visit<'a> for Components<'_> {
    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        if let Some(body) = &function.body {
            if function
                .id
                .as_ref()
                .is_some_and(|id| component_name(&id.name))
            {
                self.check_component(&function.params, body);
            }
            if function.r#async {
                self.owner().visit_function_body(body);
            }
        }
        walk::walk_function(self, function, flags);
    }

    fn visit_variable_declarator(&mut self, variable: &VariableDeclarator<'a>) {
        if matches!(&variable.id, BindingPattern::BindingIdentifier(id) if component_name(&id.name))
            && let Some(Expression::ArrowFunctionExpression(function)) =
                variable.init.as_ref().map(Expression::get_inner_expression)
        {
            self.check_arrow(function);
        }
        walk::walk_variable_declarator(self, variable);
    }

    fn visit_export_default_declaration(&mut self, export: &ExportDefaultDeclaration<'a>) {
        if let Some(Expression::ArrowFunctionExpression(function)) = export
            .declaration
            .as_expression()
            .map(Expression::get_inner_expression)
        {
            self.check_arrow(function);
        } else if let ExportDefaultDeclarationKind::FunctionDeclaration(function) =
            &export.declaration
            && function
                .id
                .as_ref()
                .is_none_or(|id| !component_name(&id.name))
            && let Some(body) = &function.body
        {
            self.check_component(&function.params, body);
        }
        walk::walk_export_default_declaration(self, export);
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        if function.r#async {
            let mut visitor = self.owner();
            if let Some(body) = function.body.as_function_body() {
                visitor.visit_function_body(body);
            } else if let Some(expression) = function.body.as_expression() {
                visitor.visit_expression(expression);
            }
        }
        walk::walk_arrow_function_expression(self, function);
    }
}

struct HasJsx(bool);

impl<'a> Visit<'a> for HasJsx {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        match expression {
            Expression::JSXElement(_) | Expression::JSXFragment(_) => self.0 = true,
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {}
            _ if !self.0 => walk::walk_expression(self, expression),
            _ => {}
        }
    }

    fn visit_function(&mut self, _: &Function<'a>, _: ScopeFlags) {}
}

struct EagerReads<'s> {
    warnings: &'s mut Vec<OxcDiagnostic>,
    scoping: &'s Scoping,
    props: Option<SymbolId>,
    signals: &'s HashSet<SymbolId>,
}

impl<'a> Visit<'a> for EagerReads<'_> {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        if matches!(
            expression,
            Expression::JSXElement(_)
                | Expression::JSXFragment(_)
                | Expression::ArrowFunctionExpression(_)
                | Expression::FunctionExpression(_)
        ) {
            return;
        }
        let object = match expression {
            Expression::StaticMemberExpression(member) => Some(&member.object),
            Expression::ComputedMemberExpression(member) => Some(&member.object),
            _ => None,
        };
        if self.props.is_some()
            && object.and_then(|object| referenced(self.scoping, object)) == self.props
        {
            self.warnings
                .push(OxcDiagnostic::warn(EAGER_PROP).with_label(expression.span()));
        }
        if let Expression::CallExpression(call) = expression
            && callee(self.scoping, call).is_some_and(|symbol| self.signals.contains(&symbol))
        {
            self.warnings
                .push(OxcDiagnostic::warn(EAGER_SIGNAL).with_label(call.span));
        }
        walk::walk_expression(self, expression);
    }

    fn visit_variable_declarator(&mut self, variable: &VariableDeclarator<'a>) {
        if matches!(variable.id, BindingPattern::ObjectPattern(_))
            && self.props.is_some()
            && variable
                .init
                .as_ref()
                .and_then(|init| referenced(self.scoping, init))
                == self.props
        {
            self.warnings
                .push(OxcDiagnostic::warn(DESTRUCTURE).with_label(variable.id.span()));
        }
        walk::walk_variable_declarator(self, variable);
    }

    fn visit_function(&mut self, _: &Function<'a>, _: ScopeFlags) {}
}

struct OwnerCalls<'s> {
    warnings: &'s mut Vec<OxcDiagnostic>,
    scoping: &'s Scoping,
    imports: &'s HashMap<SymbolId, &'s str>,
    suspended: bool,
}

impl<'a> Visit<'a> for OwnerCalls<'_> {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        match expression {
            Expression::AwaitExpression(awaited) => {
                self.visit_expression(&awaited.argument);
                self.suspended = true;
            }
            Expression::CallExpression(call) => {
                walk::walk_call_expression(self, call);
                if self.suspended
                    && matches!(
                        callee(self.scoping, call).and_then(|symbol| self.imports.get(&symbol)),
                        Some(&("onCleanup" | "createRenderEffect"))
                    )
                {
                    self.warnings
                        .push(OxcDiagnostic::warn(LOST_OWNER).with_label(call.span));
                }
            }
            Expression::ConditionalExpression(branch) => {
                self.visit_expression(&branch.test);
                let before = self.suspended;
                self.visit_expression(&branch.consequent);
                let yes = self.suspended;
                self.suspended = before;
                self.visit_expression(&branch.alternate);
                self.suspended &= yes;
            }
            Expression::LogicalExpression(logical) => {
                self.visit_expression(&logical.left);
                let before = self.suspended;
                self.visit_expression(&logical.right);
                self.suspended = before;
            }
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {}
            Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::Typeof => {}
            _ => walk::walk_expression(self, expression),
        }
    }

    fn visit_statement(&mut self, statement: &Statement<'a>) {
        match statement {
            Statement::IfStatement(branch) => {
                self.visit_expression(&branch.test);
                let before = self.suspended;
                self.visit_statement(&branch.consequent);
                let yes = self.suspended;
                self.suspended = before;
                if let Some(alternate) = &branch.alternate {
                    self.visit_statement(alternate);
                }
                self.suspended &= yes;
            }
            Statement::DoWhileStatement(_)
            | Statement::WhileStatement(_)
            | Statement::ForStatement(_)
            | Statement::ForInStatement(_)
            | Statement::ForOfStatement(_)
            | Statement::SwitchStatement(_)
            | Statement::TryStatement(_) => {
                let before = self.suspended;
                walk::walk_statement(self, statement);
                self.suspended = before;
            }
            _ => walk::walk_statement(self, statement),
        }
    }

    fn visit_function(&mut self, _: &Function<'a>, _: ScopeFlags) {}
}
