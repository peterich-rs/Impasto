//! Static diagnostics for effect program literals.

use std::collections::HashSet;

use oxc::{
    ast::ast::{
        Argument, CallExpression, Expression, ImportDeclarationSpecifier, Program, Statement,
    },
    ast_visit::{Visit, walk},
    diagnostics::OxcDiagnostic,
    semantic::Scoping,
    span::GetSpan,
    syntax::symbol::SymbolId,
};

pub(super) fn check_shaders(program: &Program<'_>, scoping: &Scoping) -> Result<(), OxcDiagnostic> {
    let mut functions = HashSet::new();
    for statement in &program.body {
        let Statement::ImportDeclaration(import) = statement else {
            continue;
        };
        if import.source.value != "@impasto/solid-ui"
            && !import.source.value.ends_with("/ShaderEffect.tsx")
        {
            continue;
        }
        for import in import.specifiers.iter().flatten() {
            if let ImportDeclarationSpecifier::ImportSpecifier(specifier) = import
                && specifier.imported.name() == "createShaderProgram"
                && let Some(symbol) = specifier.local.symbol_id.get()
            {
                functions.insert(symbol);
            }
        }
    }
    let mut visitor = ShaderCheck {
        functions,
        scoping,
        error: None,
    };
    visitor.visit_program(program);
    visitor.error.map_or(Ok(()), Err)
}

struct ShaderCheck<'s> {
    functions: HashSet<SymbolId>,
    scoping: &'s Scoping,
    error: Option<OxcDiagnostic>,
}

impl<'a> Visit<'a> for ShaderCheck<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if self.error.is_some() {
            return;
        }
        if let Expression::Identifier(identifier) = call.callee.without_parentheses()
            && let Some(reference) = identifier.reference_id.get()
            && self
                .scoping
                .get_reference(reference)
                .symbol_id()
                .is_some_and(|symbol| self.functions.contains(&symbol))
            && let Some(argument) = call.arguments.first()
            && !matches!(argument, Argument::SpreadElement(_))
            && let Some(serde_json::Value::String(source)) =
                crate::styles::literal(argument.to_expression())
            && let Err(error) = shader::validate_portability(
                &source,
                engine::scene::shader::MAX_SHADER_SOURCE_BYTES,
            )
        {
            self.error = Some(OxcDiagnostic::error(error).with_label(argument.span()));
            return;
        }
        walk::walk_call_expression(self, call);
    }
}
