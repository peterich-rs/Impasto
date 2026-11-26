//! Compile and validate application translation catalogs.
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const MODULE: &str = "virtual:impasto-locales";

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum Message {
    Text(String),
    Plural {
        one: String,
        other: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        zero: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Signature {
    plural: bool,
    parameters: BTreeSet<String>,
}

type Messages = BTreeMap<String, Message>;
type Catalogs = BTreeMap<String, Messages>;

#[derive(Debug, Default)]
pub(crate) struct Locales {
    default: String,
    locales: Catalogs,
    signatures: BTreeMap<String, Signature>,
    pub(crate) inputs: BTreeSet<PathBuf>,
}

#[derive(Default, Deserialize)]
pub(crate) struct Config {
    default_locale: Option<String>,
}

impl Locales {
    pub(crate) fn load(root: &Path) -> Result<Self> {
        #[derive(Deserialize)]
        struct Manifest {
            #[serde(default)]
            i18n: Config,
        }
        let directory = root.join("locales");
        let files = match fs::read_dir(&directory) {
            Ok(files) => files,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut this = Self::default();
        let mut signatures = BTreeMap::new();
        for file in files {
            let path = file?.path();
            if !path.is_file() || path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let locale = path
                .file_stem()
                .and_then(|name| name.to_str())
                .filter(|name| valid_locale(name))
                .with_context(|| format!("{}: expected a BCP 47 locale filename", path.display()))?
                .to_ascii_lowercase();
            let mut messages = Messages::new();
            let json = serde_json::from_slice(&fs::read(&path)?)
                .with_context(|| format!("parse {}", path.display()))?;
            flatten("", json, &mut messages).with_context(|| format!("{}", path.display()))?;
            let shapes = messages
                .iter()
                .map(|(key, message)| {
                    Ok((
                        key.clone(),
                        message
                            .signature()
                            .with_context(|| format!("{}: {key}", path.display()))?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            if this.locales.insert(locale.clone(), messages).is_some() {
                bail!("{}: duplicate locale {locale}", path.display());
            }
            signatures.insert(locale, (path.clone(), shapes));
            this.inputs.insert(path);
        }
        if this.locales.is_empty() {
            return Ok(this);
        }
        let manifest = root.join("app.toml");
        let default = if manifest.is_file() {
            this.inputs.insert(manifest.clone());
            toml::from_str::<Manifest>(&fs::read_to_string(&manifest)?)?
                .i18n
                .default_locale
        } else {
            None
        };
        this.default = default
            .map(|locale| locale.to_ascii_lowercase())
            .or_else(|| this.locales.contains_key("en").then(|| "en".to_owned()))
            .or_else(|| this.locales.keys().next().cloned())
            .context("no default locale")?;
        let (_, base) = signatures.get(&this.default).with_context(|| {
            format!(
                "default locale {:?} has no catalog in {}",
                this.default,
                directory.display()
            )
        })?;
        for (path, catalog) in signatures.values() {
            for (key, signature) in base {
                match catalog.get(key) {
                    None => bail!("{}: missing translation {key}", path.display()),
                    Some(other) if signature != other => {
                        bail!(
                            "{}: placeholder or plural form mismatch for {key}",
                            path.display()
                        );
                    }
                    Some(_) => {}
                }
            }
            if let Some(key) = catalog.keys().find(|key| !base.contains_key(*key)) {
                bail!("{}: unknown translation {key}", path.display());
            }
        }
        this.signatures = base.clone();
        Ok(this)
    }

    pub(crate) fn source(&self) -> Result<String> {
        let mut source = format!(
            "export const defaultLocale = {};\n",
            serde_json::to_string(&self.default)?
        );
        let mut names = Vec::new();
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for (index, key) in self.signatures.keys().enumerate() {
            let names_per_locale = self
                .locales
                .iter()
                .filter_map(|(locale, messages)| Some((locale, messages.get(key)?)))
                .collect::<BTreeMap<_, _>>();
            let name = format!("message{index}");
            writeln!(
                source,
                "export const {name} = {};",
                serde_json::to_string(&names_per_locale)?
            )?;
            let property = format!("[{}]: {name}", serde_json::to_string(key)?);
            if let Some((namespace, _)) = key.split_once('.') {
                groups.entry(namespace).or_default().push(property.clone());
            }
            names.push(property);
        }
        for (namespace, pieces) in groups.values().enumerate() {
            writeln!(
                source,
                "export const namespace{namespace} = {{{}}};",
                pieces.join(",")
            )?;
        }
        writeln!(source, "export const messages = {{{}}};", names.join(","))?;
        Ok(source)
    }

    fn types(&self) -> Result<String> {
        let mut output = "// Generated by Impasto.\nexport {};\ndeclare global {\n  interface ImpastoMessages {\n".to_owned();
        for (key, signature) in &self.signatures {
            write!(output, "    {}: {{", serde_json::to_string(key)?)?;
            for name in &signature.parameters {
                let value = if signature.plural && name == "count" {
                    "number"
                } else {
                    "string | number"
                };
                write!(output, " {}: {value};", serde_json::to_string(name)?)?;
            }
            output.push_str(" };\n");
        }
        output.push_str("  }\n}\n");
        Ok(output)
    }
}

fn valid_locale(tag: &str) -> bool {
    let mut parts = tag.split('-');
    parts.next().is_some_and(|part| {
        !part.is_empty() && part.len() <= 8 && part.bytes().all(|byte| byte.is_ascii_alphabetic())
    }) && parts.all(|part| {
        !part.is_empty() && part.len() <= 8 && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

fn flatten(prefix: &str, json: Value, output: &mut Messages) -> Result<()> {
    let fields = json.as_object().context("expected a catalog object")?;
    for (name, value) in fields {
        if name.split('.').any(str::is_empty) || name.chars().any(char::is_control) {
            bail!("invalid translation key {name:?}");
        }
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let message = match value {
            Value::String(value) => Message::Text(value.clone()),
            Value::Object(forms)
                if forms.contains_key("one")
                    || forms.contains_key("other")
                    || forms.contains_key("zero") =>
            {
                if forms
                    .keys()
                    .any(|name| !matches!(name.as_str(), "zero" | "one" | "other"))
                {
                    bail!("{key}: unexpected plural form");
                }
                let get = |form: &str| {
                    forms
                        .get(form)
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .with_context(|| format!("{key}: expected a string for plural form {form}"))
                };
                Message::Plural {
                    one: get("one")?,
                    other: get("other")?,
                    zero: forms
                        .contains_key("zero")
                        .then(|| get("zero"))
                        .transpose()?,
                }
            }
            Value::Object(_) => {
                flatten(&key, value.clone(), output)?;
                continue;
            }
            _ => bail!("{key}: expected a string or a namespace object"),
        };
        if output.insert(key.clone(), message).is_some() {
            bail!("duplicate translation {key}");
        }
    }
    Ok(())
}

impl Message {
    fn signature(&self) -> Result<Signature> {
        match self {
            Self::Text(text) => Ok(Signature {
                plural: false,
                parameters: parameters(text)?,
            }),
            Self::Plural { one, other, zero } => {
                let mut names = parameters(other)?;
                names.insert("count".to_owned());
                for text in std::iter::once(one).chain(zero) {
                    let mut branch = parameters(text)?;
                    branch.insert("count".to_owned());
                    if branch != names {
                        bail!("plural forms must use the same named parameters");
                    }
                }
                Ok(Signature {
                    plural: true,
                    parameters: names,
                })
            }
        }
    }
}

fn parameters(mut text: &str) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    while let Some(index) = text.find(['{', '}']) {
        let tail = text[index..]
            .strip_prefix('{')
            .context("unexpected closing brace")?;
        let (name, rest) = tail.split_once('}').context("unclosed placeholder")?;
        let mut bytes = name.bytes();
        if !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            bail!("invalid placeholder {name:?}");
        }
        names.insert(name.to_owned());
        text = rest;
    }
    Ok(names)
}

/// Validate the catalogs and generate a diagnostic-friendly TypeScript key map.
///
/// # Errors
/// Returns the catalog path and key when a catalog disagrees with the default locale.
pub fn declarations(root: &Path) -> Result<String> {
    Locales::load(root)?.types()
}

impl Locales {
    pub(crate) fn rewrite<'a>(
        &self,
        allocator: &'a oxc::allocator::Allocator,
        program: &mut oxc::ast::ast::Program<'a>,
        scoping: &oxc::semantic::Scoping,
        source: &str,
        release: bool,
    ) -> Result<(), oxc::diagnostics::OxcDiagnostic> {
        use oxc::{
            ast::ast::{ImportDeclarationSpecifier, Statement},
            ast_visit::VisitMut as _,
            diagnostics::OxcDiagnostic,
            parser::Parser,
            span::SourceType,
        };

        if self.locales.is_empty() || !source.contains("@impasto/i18n") {
            return Ok(());
        }
        let imports = program
            .body
            .iter()
            .filter_map(|statement| {
                let Statement::ImportDeclaration(import) = statement else {
                    return None;
                };
                (import.source.value == "@impasto/i18n" && !import.import_kind.is_type())
                    .then_some(import)
            })
            .flat_map(|import| import.specifiers.iter().flatten())
            .filter_map(|item| {
                let ImportDeclarationSpecifier::ImportSpecifier(item) = item else {
                    return None;
                };
                (item.imported.name() == "t" && !item.import_kind.is_type())
                    .then(|| item.local.symbol_id.get())
                    .flatten()
            })
            .collect::<BTreeSet<_>>();
        if imports.is_empty() {
            return Ok(());
        }
        let mut prefix = "__impasto_i18n".to_owned();
        while source.contains(&prefix) {
            prefix.push('_');
        }
        let ids = self
            .signatures
            .keys()
            .enumerate()
            .map(|(id, key)| (key.as_str(), id))
            .collect();
        let namespaces = self
            .signatures
            .keys()
            .filter_map(|key| key.split_once('.').map(|(namespace, _)| namespace))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(id, namespace)| (namespace, id))
            .collect();
        let mut rewrite = StaticTranslations {
            allocator,
            scoping,
            imports,
            ids,
            namespaces,
            prefix: &prefix,
            release,
            used: BTreeSet::new(),
            used_namespaces: BTreeSet::new(),
            error: None,
        };
        rewrite.visit_program(program);
        if let Some(error) = rewrite.error {
            return Err(error);
        }
        if !rewrite.used.is_empty() || !rewrite.used_namespaces.is_empty() {
            let mut head = String::new();
            if !rewrite.used.is_empty() {
                writeln!(
                    head,
                    "import {{translateMessage as {prefix}}} from '@impasto/i18n';"
                )
                .map_err(|error| OxcDiagnostic::error(error.to_string()))?;
            }
            if !rewrite.used_namespaces.is_empty() {
                writeln!(
                    head,
                    "import {{translateCatalog as {prefix}_lookup}} from '@impasto/i18n';"
                )
                .map_err(|error| OxcDiagnostic::error(error.to_string()))?;
            }
            let items = rewrite
                .used
                .iter()
                .map(|id| format!("message{id} as {prefix}_{id}"))
                .chain(
                    rewrite
                        .used_namespaces
                        .iter()
                        .map(|id| format!("namespace{id} as {prefix}_namespace{id}")),
                )
                .collect::<Vec<_>>()
                .join(",");
            write!(head, "import {{{items}}} from '{MODULE}';")
                .map_err(|error| OxcDiagnostic::error(error.to_string()))?;
            let parsed =
                Parser::new(allocator, allocator.alloc_str(&head), SourceType::mjs()).parse();
            if let Some(error) = parsed.diagnostics.into_iter().next() {
                return Err(error);
            }
            let body = std::mem::replace(&mut program.body, parsed.program.body);
            program.body.extend(body);
        }
        Ok(())
    }
}

struct StaticTranslations<'a, 's> {
    allocator: &'a oxc::allocator::Allocator,
    scoping: &'s oxc::semantic::Scoping,
    imports: BTreeSet<oxc::syntax::symbol::SymbolId>,
    ids: BTreeMap<&'s str, usize>,
    namespaces: BTreeMap<&'s str, usize>,
    prefix: &'s str,
    release: bool,
    used: BTreeSet<usize>,
    used_namespaces: BTreeSet<usize>,
    error: Option<oxc::diagnostics::OxcDiagnostic>,
}

impl<'a> oxc::ast_visit::VisitMut<'a> for StaticTranslations<'a, '_> {
    fn visit_call_expression(&mut self, call: &mut oxc::ast::ast::CallExpression<'a>) {
        use oxc::{
            ast::{
                ast::{Argument, Expression, IdentifierReference},
                builder::AstBuilder,
            },
            ast_visit::walk_mut,
            span::GetSpan as _,
        };
        walk_mut::walk_call_expression(self, call);
        if self.error.is_some() || call.optional {
            return;
        }
        let Expression::Identifier(callee) = call.callee.get_inner_expression() else {
            return;
        };
        let Some(symbol) = callee
            .reference_id
            .get()
            .and_then(|id| self.scoping.get_reference(id).symbol_id())
        else {
            return;
        };
        if !self.imports.contains(&symbol) {
            return;
        }
        let Some(arg) = call.arguments.first_mut() else {
            return;
        };
        if matches!(arg, Argument::SpreadElement(_)) {
            return;
        }
        let Some(Value::String(key)) = crate::styles::literal(arg.to_expression()) else {
            if self.release
                && let Some(namespace) = namespace_prefix(arg.to_expression())
                && let Some(&id) = self.namespaces.get(namespace.as_str())
            {
                self.used_namespaces.insert(id);
                let ast = AstBuilder::new(self.allocator);
                let span = arg.span();
                let name = self
                    .allocator
                    .alloc_str(&format!("{}_namespace{id}", self.prefix));
                call.arguments.insert(
                    0,
                    Argument::Identifier(IdentifierReference::boxed(span, name, &ast)),
                );
                let lookup = self.allocator.alloc_str(&format!("{}_lookup", self.prefix));
                call.callee =
                    Expression::Identifier(IdentifierReference::boxed(callee.span, lookup, &ast));
            }
            return;
        };
        let Some(&id) = self.ids.get(key.as_str()) else {
            self.error = Some(
                oxc::diagnostics::OxcDiagnostic::error(format!("unknown translation key {key:?}"))
                    .with_label(arg.span()),
            );
            return;
        };
        if self.release {
            self.used.insert(id);
            let ast = AstBuilder::new(self.allocator);
            let name = self.allocator.alloc_str(&format!("{}_{id}", self.prefix));
            *arg = Argument::Identifier(IdentifierReference::boxed(arg.span(), name, &ast));
            let name = self.allocator.alloc_str(self.prefix);
            call.callee =
                Expression::Identifier(IdentifierReference::boxed(callee.span, name, &ast));
        }
    }
}

fn namespace_prefix(expression: &oxc::ast::ast::Expression<'_>) -> Option<String> {
    use oxc::{ast::ast::Expression, syntax::operator::BinaryOperator};
    let head = match expression.get_inner_expression() {
        Expression::TemplateLiteral(template) => template.quasis.first()?.value.cooked?.to_string(),
        Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            if let Some(Value::String(value)) = crate::styles::literal(&binary.left) {
                value
            } else {
                return namespace_prefix(&binary.left);
            }
        }
        _ => return None,
    };
    let (namespace, _) = head.split_once('.')?;
    Some(namespace.to_owned())
}
