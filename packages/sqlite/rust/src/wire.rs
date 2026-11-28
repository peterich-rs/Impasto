use impasto::abi::PluginValue;
use impasto::{FromPluginValue, ImpastoType, IntoPluginValue, TsTypes};
use rusqlite::{
    Statement,
    types::{Value, ValueRef},
};

use crate::SqlError;

#[derive(Clone, Copy, Default, ImpastoType)]
pub(crate) enum IntMode {
    #[default]
    Number,
    Bigint,
    String,
}

#[derive(Clone)]
pub(crate) struct Cell(pub(crate) Value);

impl FromPluginValue for Cell {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "integer conversion is bounded by the JavaScript safe integer range"
    )]
    fn from_value(value: PluginValue) -> Result<Self, String> {
        let safe = 9_007_199_254_740_991.0;
        Ok(Self(match value {
            PluginValue::Null => Value::Null,
            PluginValue::Bytes(value) => Value::Blob(value),
            PluginValue::Str(value) => Value::Text(value),
            PluginValue::F64(value) if value.fract() == 0.0 && value.abs() <= safe => {
                Value::Integer(value as i64)
            }
            PluginValue::F64(value) if value.is_finite() && value.fract() != 0.0 => {
                Value::Real(value)
            }
            PluginValue::Map(fields) if fields.len() == 1 => {
                let (name, value) = &fields[0];
                if name == "integer"
                    && let PluginValue::Str(value) = value
                {
                    Value::Integer(value.parse().map_err(|_| "SQL integer is out of range")?)
                } else {
                    return Err("invalid SQL value".into());
                }
            }
            _ => return Err("invalid SQL value".into()),
        }))
    }
}

impl IntoPluginValue for Cell {
    fn into_value(self) -> Result<PluginValue, String> {
        Ok(encode(self.0, IntMode::Bigint))
    }
}

impl ImpastoType for Cell {
    fn typescript(_: &mut TsTypes) -> String {
        "number | string | null | Uint8Array | ArrayBuffer | { integer: string }".into()
    }
}

fn encode(value: Value, mode: IntMode) -> PluginValue {
    match value {
        Value::Null => PluginValue::Null,
        Value::Blob(bytes) => PluginValue::Bytes(bytes),
        Value::Real(number) => PluginValue::F64(number),
        Value::Text(text) => PluginValue::Str(text),
        Value::Integer(integer) => match mode {
            IntMode::Bigint => PluginValue::Map(vec![(
                "integer".into(),
                PluginValue::Str(integer.to_string()),
            )]),
            IntMode::String => PluginValue::Str(integer.to_string()),
            IntMode::Number => {
                if (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&integer) {
                    PluginValue::F64(integer as f64)
                } else {
                    PluginValue::Str(integer.to_string())
                }
            }
        },
    }
}

pub(crate) struct Column(PluginValue);

impl Column {
    pub(crate) fn rowid(integer: i64, mode: IntMode) -> Self {
        let mode =
            if !matches!(mode, IntMode::Bigint) && integer.unsigned_abs() > 9_007_199_254_740_991 {
                IntMode::Bigint
            } else {
                mode
            };
        let mode = if matches!(mode, IntMode::String) {
            IntMode::Number
        } else {
            mode
        };
        Self(encode(Value::Integer(integer), mode))
    }
    pub(crate) fn from_sql(value: ValueRef<'_>, mode: IntMode) -> Result<Self, SqlError> {
        let value = match value {
            ValueRef::Text(value) => Value::Text(
                std::str::from_utf8(value)
                    .map_err(|_| SqlError::mismatch())?
                    .to_owned(),
            ),
            value => value.into(),
        };
        Ok(Self(encode(value, mode)))
    }
}

impl FromPluginValue for Column {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        Ok(Self(value))
    }
}
impl IntoPluginValue for Column {
    fn into_value(self) -> Result<PluginValue, String> {
        Ok(self.0)
    }
}
impl ImpastoType for Column {
    fn typescript(_: &mut TsTypes) -> String {
        Cell::typescript(&mut TsTypes::default())
    }
}

#[derive(ImpastoType, Default, Clone)]
pub(crate) struct Params {
    positional: Option<Vec<Cell>>,
    named: Option<Vec<(String, Cell)>>,
}

impl Params {
    pub(crate) fn bind(self, statement: &mut Statement<'_>) -> Result<(), SqlError> {
        let count = statement.parameter_count();
        match (self.positional, self.named) {
            (Some(values), None) if values.len() == count => {
                for (index, value) in values.into_iter().enumerate() {
                    statement.raw_bind_parameter(index + 1, value.0)?;
                }
            }
            (None, Some(values)) if values.len() == count => {
                let mut bound = std::collections::BTreeSet::new();
                for (name, value) in values {
                    let index = if name.starts_with([':', '@', '$', '?']) {
                        statement.parameter_index(&name)?
                    } else {
                        let mut index = None;
                        for prefix in [':', '@', '$'] {
                            if let Some(candidate) =
                                statement.parameter_index(&format!("{prefix}{name}"))?
                                && index.replace(candidate).is_some()
                            {
                                return Err(SqlError::mismatch());
                            }
                        }
                        index
                    }
                    .ok_or_else(SqlError::mismatch)?;
                    if !bound.insert(index) {
                        return Err(SqlError::mismatch());
                    }
                    statement.raw_bind_parameter(index, value.0)?;
                }
            }
            (None, None) if count == 0 => {}
            _ => return Err(SqlError::mismatch()),
        }
        Ok(())
    }
}
