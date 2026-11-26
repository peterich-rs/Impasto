//! Original function ranges used to name minified stack frames.

use std::path::Path;

use oxc::{
    allocator::Allocator,
    ast::ast::{Expression, Function, VariableDeclarator},
    ast_visit::{Visit, walk},
    parser::Parser,
    span::{GetSpan, SourceType, Span},
};

/// Ranges of named functions in the original source, with zero-based UTF-16 coordinates.
pub fn function_symbols(path: &Path, source: &str) -> Vec<(String, [u32; 4])> {
    let Ok(source_type) = SourceType::from_path(path) else {
        return Vec::new();
    };
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Vec::new();
    }
    let mut symbols = Functions::default();
    symbols.visit_program(&parsed.program);
    let mut lines = vec![0];
    lines.extend(source.match_indices('\n').map(|(offset, _)| offset + 1));
    let position = |offset: u32| {
        let offset = offset as usize;
        let index = lines.partition_point(|start| *start <= offset) - 1;
        let col = source.get(lines[index]..offset)?.encode_utf16().count();
        Some((u32::try_from(index).ok()?, u32::try_from(col).ok()?))
    };
    symbols
        .0
        .sort_by_key(|(_, span)| (span.start, std::cmp::Reverse(span.end)));
    symbols
        .0
        .into_iter()
        .filter_map(|(name, span)| {
            let (first_line, first_col) = position(span.start)?;
            let (last_line, last_col) = position(span.end)?;
            Some((name, [first_line, first_col, last_line, last_col]))
        })
        .collect()
}

#[derive(Default)]
struct Functions(Vec<(String, Span)>);

impl<'a> Visit<'a> for Functions {
    fn visit_function(&mut self, function: &Function<'a>, flags: oxc::syntax::scope::ScopeFlags) {
        if let Some(body) = &function.body
            && let Some(id) = &function.id
        {
            self.0.push((id.name.to_string(), body.span));
        }
        walk::walk_function(self, function, flags);
    }

    fn visit_variable_declarator(&mut self, declaration: &VariableDeclarator<'a>) {
        if let (Some(binding), Some(init)) =
            (declaration.id.get_binding_identifier(), &declaration.init)
        {
            let span = match init.without_parentheses() {
                Expression::ArrowFunctionExpression(arrow) => Some(arrow.body.span()),
                Expression::FunctionExpression(function) if function.id.is_none() => {
                    function.body.as_ref().map(|body| body.span)
                }
                _ => None,
            };
            if let Some(span) = span {
                self.0.push((binding.name.to_string(), span));
            }
        }
        walk::walk_variable_declarator(self, declaration);
    }
}
