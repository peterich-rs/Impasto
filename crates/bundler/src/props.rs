//! Detect prop reads that bypass getter tracking in a JSX component.

const EXPLANATION: &str = "destructuring component props reads reactive getters once; read each property from props where it is used";

use oxc::{
    ast::ast::{
        ArrowFunctionBody, BindingPattern, Expression, FormalParameters, FunctionBody, Program,
        VariableDeclarator,
    },
    ast_visit::{Visit, walk},
    diagnostics::OxcDiagnostic,
    span::GetSpan,
    syntax::scope::ScopeFlags,
};

pub(super) fn check(program: &Program<'_>) -> Vec<OxcDiagnostic> {
    let mut visitor = Components(Vec::new());
    visitor.visit_program(program);
    visitor.0
}

struct Components(Vec<OxcDiagnostic>);

impl Components {
    fn check_component(&mut self, params: &FormalParameters<'_>, body: &FunctionBody<'_>) {
        let Some(parameter) = params.items.first() else {
            return;
        };
        let mut jsx = HasJsx(false);
        jsx.visit_function_body(body);
        if !jsx.0 {
            return;
        }
        match &parameter.pattern {
            BindingPattern::ObjectPattern(pattern) => self
                .0
                .push(OxcDiagnostic::warn(EXPLANATION).with_label(pattern.span)),
            BindingPattern::BindingIdentifier(props) => {
                for statement in &body.statements {
                    let oxc::ast::ast::Statement::VariableDeclaration(declaration) = statement
                    else {
                        continue;
                    };
                    for binding in &declaration.declarations {
                        if matches!(binding.id, BindingPattern::ObjectPattern(_))
                            && matches!(&binding.init, Some(Expression::Identifier(source)) if source.name == props.name)
                        {
                            self.0.push(
                                OxcDiagnostic::warn(EXPLANATION).with_label(binding.id.span()),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn component_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
}

impl<'a> Visit<'a> for Components {
    fn visit_function(&mut self, function: &oxc::ast::ast::Function<'a>, flags: ScopeFlags) {
        if function
            .id
            .as_ref()
            .is_some_and(|id| component_name(&id.name))
            && let Some(body) = &function.body
        {
            self.check_component(&function.params, body);
        }
        walk::walk_function(self, function, flags);
    }

    fn visit_variable_declarator(&mut self, variable: &VariableDeclarator<'a>) {
        if matches!(&variable.id, BindingPattern::BindingIdentifier(id) if component_name(&id.name))
            && let Some(Expression::ArrowFunctionExpression(function)) = &variable.init
        {
            if let ArrowFunctionBody::FunctionBody(body) = &function.body {
                self.check_component(&function.params, body);
            } else if let Some(expression) = function.body.as_expression() {
                let mut jsx = HasJsx(false);
                jsx.visit_expression(expression);
                if jsx.0
                    && let Some(parameter) = function.params.items.first()
                    && let BindingPattern::ObjectPattern(pattern) = &parameter.pattern
                {
                    self.0
                        .push(OxcDiagnostic::warn(EXPLANATION).with_label(pattern.span));
                }
            }
        }
        walk::walk_variable_declarator(self, variable);
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
}
