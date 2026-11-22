//! Pure TypeScript projection of a host module's export metadata.

use std::fmt::{self, Write as _};

use super::registry::validate_module;
use super::{Invoke, Module, RegistryError};

#[derive(Debug)]
pub enum DeclarationError {
    Registry(RegistryError),
    MissingSignature {
        module: &'static str,
        method: &'static str,
    },
}

impl fmt::Display for DeclarationError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(error) => error.fmt(out),
            Self::MissingSignature { module, method } => {
                write!(out, "{module}.{method} has no TypeScript signature")
            }
        }
    }
}

impl std::error::Error for DeclarationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Registry(error) => Some(error),
            Self::MissingSignature { .. } => None,
        }
    }
}

impl Module {
    /// Renders the declaration body, independently of package paths and I/O.
    pub fn typescript_declaration(&self) -> Result<String, DeclarationError> {
        validate_module(self).map_err(DeclarationError::Registry)?;
        let signatures = signatures(self)?;

        let mut out = String::new();
        if !self.exports.typescript.is_empty() {
            out.push('\n');
            out.push_str(self.exports.typescript);
            out.push_str("\n\n");
        }
        let name = self.name;
        if self.exports.object {
            let _ = writeln!(out, "\nexport interface {name} {{");
            for (method, signature) in &signatures {
                let _ = writeln!(out, "  {method}{signature}");
            }
            let _ = writeln!(out, "}}\n\nexport declare const {name}: {name};\n");
        }
        for (method, signature) in signatures {
            if is_binding_identifier(method) {
                let _ = writeln!(out, "export declare function {method}{signature}");
            } else {
                // Reserved IdentifierNames such as `delete` are legal exports.
                let mut alias = format!("${method}");
                while self.export_names().any(|export| export == alias) {
                    alias.insert(0, '$');
                }
                let _ = writeln!(
                    out,
                    "declare function {alias}{signature}\nexport {{ {alias} as {method} }};"
                );
            }
        }
        Ok(out)
    }
}

fn signatures(module: &Module) -> Result<Vec<(&'static str, String)>, DeclarationError> {
    let mut signatures = Vec::new();
    for method in &module.methods {
        let types = method.types.ok_or(DeclarationError::MissingSignature {
            module: module.specifier,
            method: method.name,
        })?;
        let parameters = types.parameters;
        let returns = types.returns;
        let signature = match method.invoke {
            Invoke::Sync(_) | Invoke::Resource(_) | Invoke::Stream(_) => {
                format!("({parameters}): {returns};")
            }
            Invoke::Promise(_) => format!("({parameters}): Promise<{returns}>;"),
        };
        signatures.push((method.name, signature));
    }
    for &listener in module.listener_exports() {
        for event in module.exports.events {
            let signature = format!(
                "(type: {:?}, listener: (event: {}) => void): void;",
                event.name, event.payload
            );
            signatures.push((listener, signature));
        }
    }
    Ok(signatures)
}

pub(crate) fn is_identifier_name(name: &str) -> bool {
    let mut chars = name.bytes();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || b"_$".contains(&ch))
        && chars.all(|ch| ch.is_ascii_alphanumeric() || b"_$".contains(&ch))
}

pub(crate) fn is_binding_identifier(name: &str) -> bool {
    is_identifier_name(name) && !is_reserved_word(name)
}

fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "arguments"
            | "eval"
            | "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
            | "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
    )
}
