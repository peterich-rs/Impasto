use std::collections::{BTreeMap, HashMap};
use std::fmt;

use crate::{
    ImpastoError,
    abi::{PluginResult, PluginValue, StreamSink},
};

pub trait FromPluginValue: Sized {
    fn from_value(value: PluginValue) -> Result<Self, String>;
}

pub trait IntoPluginValue {
    fn into_value(self) -> Result<PluginValue, String>;
}

pub trait ImpastoType: FromPluginValue + IntoPluginValue {
    fn typescript(types: &mut TsTypes) -> String;

    fn input_typescript(types: &mut TsTypes) -> String {
        Self::typescript(types)
    }
}

#[derive(Default)]
pub struct TsTypes {
    definitions: BTreeMap<&'static str, (&'static str, String)>,
    collision: Option<String>,
}

impl TsTypes {
    pub fn define<T: 'static>(
        &mut self,
        name: &'static str,
        build: impl FnOnce(&mut Self) -> String,
    ) {
        let ident = std::any::type_name::<T>();
        if let Some((declared, _)) = self.definitions.get(name) {
            if *declared != ident {
                self.collision = Some(format!(
                    "TypeScript name {name} is used by {declared} and {ident}"
                ));
            }
            return;
        }
        self.definitions.insert(name, (ident, String::new()));
        let text = build(self);
        self.definitions.insert(name, (ident, text));
    }

    pub fn render(self) -> Result<String, crate::PluginInitError> {
        if let Some(error) = self.collision {
            return Err(crate::PluginInitError::new(error));
        }
        Ok(self
            .definitions
            .into_values()
            .map(|(_, text)| text)
            .collect::<Vec<_>>()
            .join("\n\n"))
    }
}

pub fn decode_arg<T: FromPluginValue>(
    args: &[PluginValue],
    index: usize,
    name: &str,
) -> Result<T, String> {
    T::from_value(args.get(index).cloned().unwrap_or(PluginValue::Null))
        .map_err(|error| format!("{name}: {error}"))
}

pub fn decode_field<T: FromPluginValue>(
    fields: &mut BTreeMap<String, PluginValue>,
    name: &str,
) -> Result<T, String> {
    T::from_value(fields.remove(name).unwrap_or(PluginValue::Null))
        .map_err(|error| format!("{name}: {error}"))
}

pub fn encode_error(error: impl ImpastoError) -> String {
    format!("[{}:{}] {}", error.scope(), error.code(), error.message())
}

pub fn finish_value(result: Result<PluginValue, String>) -> PluginResult {
    match result {
        Ok(value) => PluginResult::Done(value),
        Err(message) => PluginResult::Rejected { message },
    }
}

macro_rules! primitive {
    ($rust:ty, $variant:path, $ts:literal) => {
        impl FromPluginValue for $rust {
            fn from_value(value: PluginValue) -> Result<Self, String> {
                match value {
                    $variant(value) => Ok(value),
                    _ => Err(format!("expected {}", $ts)),
                }
            }
        }
        impl IntoPluginValue for $rust {
            fn into_value(self) -> Result<PluginValue, String> {
                Ok($variant(self))
            }
        }
        impl ImpastoType for $rust {
            fn typescript(_: &mut TsTypes) -> String {
                $ts.into()
            }
        }
    };
}

primitive!(bool, PluginValue::Bool, "boolean");
primitive!(String, PluginValue::Str, "string");
primitive!(f64, PluginValue::F64, "number");

impl FromPluginValue for () {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        if value == PluginValue::Null {
            Ok(())
        } else {
            Err("expected null".into())
        }
    }
}
impl IntoPluginValue for () {
    fn into_value(self) -> Result<PluginValue, String> {
        Ok(PluginValue::Null)
    }
}
impl ImpastoType for () {
    fn typescript(_: &mut TsTypes) -> String {
        "null".into()
    }
}
impl IntoPluginValue for &str {
    fn into_value(self) -> Result<PluginValue, String> {
        self.to_owned().into_value()
    }
}

macro_rules! integer {
    ($rust:ty, $min:expr, $max:expr, $check:expr) => {
        impl FromPluginValue for $rust {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "validated as an in-range integer"
            )]
            fn from_value(value: PluginValue) -> Result<Self, String> {
                let value = f64::from_value(value)?;
                if value.fract() == 0.0 && ($min..=$max).contains(&value) {
                    Ok(value as Self)
                } else {
                    Err(concat!("expected an in-range ", stringify!($rust)).into())
                }
            }
        }
        impl IntoPluginValue for $rust {
            fn into_value(self) -> Result<PluginValue, String> {
                $check(self)
            }
        }
        impl ImpastoType for $rust {
            fn typescript(_: &mut TsTypes) -> String {
                "number".into()
            }
        }
    };
}

fn small_number<T: Into<f64>>(number: T) -> Result<PluginValue, String> {
    Ok(PluginValue::F64(number.into()))
}

macro_rules! small_integer {
    ($($rust:ty),*) => {$(
        integer!($rust, f64::from(<$rust>::MIN), f64::from(<$rust>::MAX), small_number);
    )*};
}

small_integer!(i8, i16, i32, u8, u16, u32);

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

#[expect(
    clippy::cast_precision_loss,
    reason = "only integers within the safe JavaScript range are cast"
)]
fn unsigned_number(number: u64) -> Result<PluginValue, String> {
    if number > javascript::host::MAX_SAFE_ID {
        Err("integer exceeds JavaScript safe range".into())
    } else {
        Ok(PluginValue::F64(number as f64))
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "only integers within the safe JavaScript range are cast"
)]
fn signed_number(number: i64) -> Result<PluginValue, String> {
    if number.unsigned_abs() > javascript::host::MAX_SAFE_ID {
        Err("integer exceeds JavaScript safe range".into())
    } else {
        Ok(PluginValue::F64(number as f64))
    }
}

integer!(i64, -MAX_SAFE, MAX_SAFE, signed_number);
integer!(u64, 0.0, MAX_SAFE, unsigned_number);

impl FromPluginValue for f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "converts IEEE 754 precision to the requested width"
    )]
    fn from_value(value: PluginValue) -> Result<Self, String> {
        Ok(f64::from_value(value)? as Self)
    }
}
impl IntoPluginValue for f32 {
    fn into_value(self) -> Result<PluginValue, String> {
        small_number(self)
    }
}
impl ImpastoType for f32 {
    fn typescript(_: &mut TsTypes) -> String {
        "number".into()
    }
}

impl<T: FromPluginValue> FromPluginValue for Option<T> {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        match value {
            PluginValue::Null => Ok(None),
            value => T::from_value(value).map(Some),
        }
    }
}
impl<T: IntoPluginValue> IntoPluginValue for Option<T> {
    fn into_value(self) -> Result<PluginValue, String> {
        self.map_or(Ok(PluginValue::Null), T::into_value)
    }
}
impl<T: ImpastoType> ImpastoType for Option<T> {
    fn typescript(types: &mut TsTypes) -> String {
        format!("{} | null", T::typescript(types))
    }
    fn input_typescript(types: &mut TsTypes) -> String {
        format!("{} | null", T::input_typescript(types))
    }
}

impl<T: FromPluginValue> FromPluginValue for Vec<T> {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        let PluginValue::List(items) = value else {
            return Err("expected an array".into());
        };
        items
            .into_iter()
            .enumerate()
            .map(|(index, item)| T::from_value(item).map_err(|error| format!("[{index}]: {error}")))
            .collect()
    }
}
impl<T: IntoPluginValue> IntoPluginValue for Vec<T> {
    fn into_value(self) -> Result<PluginValue, String> {
        self.into_iter()
            .map(T::into_value)
            .collect::<Result<_, _>>()
            .map(PluginValue::List)
    }
}
impl<T: ImpastoType> ImpastoType for Vec<T> {
    fn typescript(types: &mut TsTypes) -> String {
        format!("Array<{}>", T::typescript(types))
    }
    fn input_typescript(types: &mut TsTypes) -> String {
        format!("Array<{}>", T::input_typescript(types))
    }
}

macro_rules! string_map {
    ($map:ident) => {
        impl<T: FromPluginValue> FromPluginValue for $map<String, T> {
            fn from_value(value: PluginValue) -> Result<Self, String> {
                let PluginValue::Map(items) = value else {
                    return Err("expected an object".into());
                };
                items
                    .into_iter()
                    .map(|(key, value)| {
                        let value =
                            T::from_value(value).map_err(|error| format!("{key}: {error}"))?;
                        Ok((key, value))
                    })
                    .collect()
            }
        }
        impl<T: IntoPluginValue> IntoPluginValue for $map<String, T> {
            fn into_value(self) -> Result<PluginValue, String> {
                self.into_iter()
                    .map(|(key, item)| Ok((key, item.into_value()?)))
                    .collect::<Result<_, _>>()
                    .map(PluginValue::Map)
            }
        }
        impl<T: ImpastoType> ImpastoType for $map<String, T> {
            fn typescript(types: &mut TsTypes) -> String {
                format!("Record<string, {}>", T::typescript(types))
            }
            fn input_typescript(types: &mut TsTypes) -> String {
                format!("Record<string, {}>", T::input_typescript(types))
            }
        }
    };
}

string_map!(BTreeMap);
string_map!(HashMap);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);
impl FromPluginValue for Bytes {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        match value {
            PluginValue::Bytes(bytes) => Ok(Self(bytes)),
            _ => Err("expected binary data".into()),
        }
    }
}
impl IntoPluginValue for Bytes {
    fn into_value(self) -> Result<PluginValue, String> {
        Ok(PluginValue::Bytes(self.0))
    }
}
impl ImpastoType for Bytes {
    fn typescript(_: &mut TsTypes) -> String {
        "Uint8Array | ArrayBuffer".into()
    }
}

pub struct TypedSink<T> {
    sink: StreamSink,
    item: std::marker::PhantomData<fn(T)>,
}

impl<T: IntoPluginValue> TypedSink<T> {
    #[doc(hidden)]
    pub fn new(sink: StreamSink) -> Self {
        Self {
            sink,
            item: std::marker::PhantomData,
        }
    }

    pub fn cancellation(&self) -> crate::CancelToken {
        self.sink.cancellation()
    }

    pub async fn send(&mut self, item: T) -> Result<(), TypedSendError> {
        let value = item.into_value().map_err(TypedSendError::Encode)?;
        self.sink
            .send_async(value)
            .await
            .map_err(TypedSendError::Transport)
    }

    #[doc(hidden)]
    pub fn finish(self, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => self.sink.finish(),
            Err(error) => self.sink.fail(error),
        }
    }
}

#[derive(Debug)]
pub enum TypedSendError {
    Encode(String),
    Transport(crate::abi::StreamSendError),
}

impl fmt::Display for TypedSendError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(message) => out.write_str(message),
            Self::Transport(error) => error.fmt(out),
        }
    }
}

impl std::error::Error for TypedSendError {}

impl ImpastoError for TypedSendError {
    fn scope(&self) -> &'static str {
        "impasto"
    }
    fn code(&self) -> &'static str {
        "stream"
    }
    fn message(&self) -> String {
        self.to_string()
    }
}
