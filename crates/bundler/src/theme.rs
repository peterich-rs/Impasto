use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context as _, Result, bail};
use oxc::{allocator::Allocator, ast::ast::Statement, parser::Parser, span::SourceType};
use serde::Deserialize;

/// Statically declared application extensions to the framework's theme.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeTokens {
    /// Application color keys.
    pub colors: BTreeSet<String>,
    /// Application length keys.
    pub lengths: BTreeSet<String>,
}

impl ThemeTokens {
    /// Read `[theme.tokens]` and the optional literal `src/theme.ts` export.
    ///
    /// # Errors
    /// Returns a diagnostic when a configured token table is invalid.
    pub fn load(root: &Path) -> Result<Self> {
        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct Theme {
            tokens: ThemeTokens,
        }
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Manifest {
            theme: Theme,
        }
        let manifest = root.join("app.toml");
        let mut tokens = if manifest.is_file() {
            toml::from_str::<Manifest>(&fs::read_to_string(&manifest)?)
                .with_context(|| manifest.display().to_string())?
                .theme
                .tokens
        } else {
            Self::default()
        };
        let source_path = root.join("src/theme.ts");
        if source_path.is_file() {
            let source = fs::read_to_string(&source_path)?;
            let allocator = Allocator::default();
            use oxc::ast::ast::{BindingPattern, Declaration, VariableDeclarationKind};
            let parsed = Parser::new(&allocator, &source, SourceType::ts()).parse();
            for statement in parsed.program.body {
                let Statement::ExportDeclaration(export) = statement else {
                    continue;
                };
                let Declaration::VariableDeclaration(declaration) = &export.declaration else {
                    continue;
                };
                for binding in &declaration.declarations {
                    let BindingPattern::BindingIdentifier(name) = &binding.id else {
                        continue;
                    };
                    if name.name != "themeTokens" {
                        continue;
                    }
                    let literal = binding
                        .init
                        .as_ref()
                        .filter(|_| declaration.kind == VariableDeclarationKind::Const)
                        .and_then(crate::styles::literal)
                        .with_context(|| {
                            format!(
                                "{}: export themeTokens as a const literal",
                                source_path.display()
                            )
                        })?;
                    let extra: Self = serde_json::from_value(literal).with_context(|| {
                        format!("{}: invalid themeTokens", source_path.display())
                    })?;
                    tokens.colors.extend(extra.colors);
                    tokens.lengths.extend(extra.lengths);
                }
            }
        }
        for name in tokens.colors.iter().chain(&tokens.lengths) {
            if name.trim() != name || name.is_empty() || name.chars().any(char::is_control) {
                bail!("{}: invalid theme token {name:?}", root.display());
            }
        }
        Ok(tokens)
    }
}
