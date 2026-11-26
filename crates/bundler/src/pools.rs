//! Content-addressed, lazily decoded scene constants shared by linked modules.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    hash::{DefaultHasher, Hash, Hasher},
};

use engine::scene::template::{self, Template, TemplateImage, TemplateKind, TemplateRow};
use oxc::diagnostics::OxcDiagnostic;

pub(crate) const PREFIX: &str = "impasto:pool/";

#[derive(Clone, Debug, Default)]
pub(crate) struct PoolModules(BTreeMap<String, String>);

impl PoolModules {
    pub(crate) fn intern(&mut self, kind: &str, source: String) -> Result<String, OxcDiagnostic> {
        let mut digest = DefaultHasher::new();
        source.hash(&mut digest);
        let id = format!("{PREFIX}{kind}/{:016x}", digest.finish());
        self.insert(id, source)
    }

    pub(crate) fn scalar(
        &mut self,
        kind: &str,
        define: &str,
        encoded: &str,
    ) -> Result<String, OxcDiagnostic> {
        let encoded = quote(encoded);
        let source = if kind == "style" {
            format!("export default [{encoded}];")
        } else {
            format!(
                "import {{ {define} }} from 'scene'; let id; export default () => id ??= {define}({encoded});"
            )
        };
        self.intern(kind, source)
    }

    pub(crate) fn template(&mut self, source: &str) -> Result<String, OxcDiagnostic> {
        let mut styles = Vec::new();
        let mut classes = Vec::new();
        let mut images = Vec::new();
        let json: Vec<Vec<serde_json::Value>> = serde_json::from_str(source)
            .map_err(|error| OxcDiagnostic::error(error.to_string()))?;
        let invalid = || OxcDiagnostic::error("could not encode compiler template");
        let rows = json
            .iter()
            .map(|row| {
                Some(TemplateRow {
                    kind: TemplateKind::parse(row.first()?.as_str()?)?,
                    parent: u8::try_from(row.get(1)?.as_u64()?).ok()?,
                    text: row
                        .get(2)
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    style: import_index(&mut styles, row.get(3))?,
                    class: import_index(&mut classes, row.get(4))?,
                    image: if let Some(record) = row.get(5).filter(|value| !value.is_null()) {
                        let image: TemplateImage = serde_json::from_value(record.clone()).ok()?;
                        let index = images
                            .iter()
                            .position(|existing| existing == &image)
                            .unwrap_or_else(|| {
                                images.push(image);
                                images.len() - 1
                            });
                        Some(u8::try_from(index).ok()?)
                    } else {
                        None
                    },
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
        let mut module = String::new();
        for (kind, define, entries) in [
            ("style", "defineStyle", styles.as_slice()),
            ("class", "defineClass", classes.as_slice()),
        ] {
            for (index, encoded) in entries.iter().enumerate() {
                let dependency = self.scalar(kind, define, encoded)?;
                let _ = write!(module, "import {kind}{index} from {};", quote(&dependency));
            }
        }
        let arguments = |kind: &str, len| {
            (0..len)
                .map(|index| format!("{kind}{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let encoded = template::encode(&Template { rows, images }).map_err(OxcDiagnostic::error)?;
        let _ = write!(
            module,
            "export default [{}, [{}], [{}]];",
            quote(&encoded),
            arguments("style", styles.len()),
            arguments("class", classes.len()),
        );
        self.intern("template", module)
    }

    fn insert(&mut self, id: String, source: String) -> Result<String, OxcDiagnostic> {
        match self.0.entry(id.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) if entry.get() != &source => {
                Err(OxcDiagnostic::error("scene pool identifier collision"))
            }
            std::collections::btree_map::Entry::Occupied(_) => Ok(id),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(source);
                Ok(id)
            }
        }
    }

    #[cfg(feature = "bundle")]
    pub(crate) fn extend(&mut self, other: &Self) -> Result<(), OxcDiagnostic> {
        for (id, body) in &other.0 {
            self.insert(id.clone(), body.clone())?;
        }
        Ok(())
    }

    #[cfg(feature = "bundle")]
    pub(crate) fn get(&self, id: &str) -> Option<&str> {
        self.0.get(id).map(String::as_str)
    }

    #[cfg(feature = "bundle")]
    pub(crate) fn ids(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }
}

fn quote(text: &str) -> String {
    serde_json::Value::String(text.to_owned()).to_string()
}

fn import_index<'a>(
    entries: &mut Vec<&'a str>,
    value: Option<&'a serde_json::Value>,
) -> Option<Option<u8>> {
    let Some(value) = value.and_then(serde_json::Value::as_str) else {
        return Some(None);
    };
    let index = entries
        .iter()
        .position(|entry| *entry == value)
        .unwrap_or_else(|| {
            let index = entries.len();
            entries.push(value);
            index
        });
    u8::try_from(index).ok().map(Some)
}
