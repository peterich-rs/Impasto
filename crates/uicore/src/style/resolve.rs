//! Merge source layers into the winning longhands.
//!
//! `Reset` stays as [`WinningDeclaration::Initial`]. It is not the same as an
//! omitted key: a reset hides lower sources until `clear` removes it.

use std::collections::BTreeMap;

use super::{Declaration, Key, Property, Style, flatten};

/// One key after every source layer has been applied, before inheritance.
#[derive(Clone, Debug, PartialEq)]
pub enum WinningDeclaration {
    Value(Property),
    /// `null` / reset. The node preset shows through and inheritance stops.
    Initial,
    /// Explicit inherit. A parent value is copied when one exists.
    Inherit,
}

/// The computed declarations after every layer has been applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    pub values: BTreeMap<Key, WinningDeclaration>,
}

pub fn merge_layers(layers: &[&Style]) -> Resolved {
    let mut values = BTreeMap::new();
    for layer in layers {
        for (key, declaration) in flatten(layer) {
            let winning = match declaration {
                Declaration::Reset(_) => WinningDeclaration::Initial,
                Declaration::Inherit(_) => WinningDeclaration::Inherit,
                Declaration::Set(property) => WinningDeclaration::Value(property),
            };
            values.insert(key, winning);
        }
    }
    Resolved { values }
}

impl Resolved {
    pub fn explicit_keys(&self) -> impl Iterator<Item = Key> + '_ {
        self.values.iter().filter_map(|(key, declaration)| {
            matches!(declaration, WinningDeclaration::Value(_)).then_some(*key)
        })
    }

    pub fn property(&self, key: Key) -> Option<&Property> {
        match self.values.get(&key) {
            Some(WinningDeclaration::Value(property)) => Some(property),
            _ => None,
        }
    }

    /// Copy inherited keys from `parent` when this node did not set or reset them.
    pub fn with_inheritance(&self, parent: Option<&Resolved>) -> Resolved {
        let mut values = self.values.clone();
        for key in
            KEYS_INHERITED
                .iter()
                .chain(self.values.iter().filter_map(|(key, value)| {
                    matches!(value, WinningDeclaration::Inherit).then_some(key)
                }))
        {
            match values.get(key) {
                Some(WinningDeclaration::Value(_) | WinningDeclaration::Initial) => {}
                Some(WinningDeclaration::Inherit) | None => {
                    if let Some(WinningDeclaration::Value(property)) =
                        parent.and_then(|parent| parent.values.get(key))
                    {
                        values.insert(*key, WinningDeclaration::Value(property.clone()));
                    } else if matches!(values.get(key), Some(WinningDeclaration::Inherit)) {
                        values.remove(key);
                    }
                }
            }
        }
        Resolved { values }
    }
}

const KEYS_INHERITED: &[Key] = &[
    Key::Color,
    Key::FontFamily,
    Key::FontSize,
    Key::FontWeight,
    Key::LineHeight,
    Key::TextAlign,
    Key::WritingDirection,
    Key::LayoutDirection,
];
