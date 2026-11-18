/// JSON-like value exchanged with one native module method.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginValue {
    Null,
    Bool(bool),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<PluginValue>),
    Map(Vec<(String, PluginValue)>),
}

/// Completion of one token allocated for a promise.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginResult {
    Done(PluginValue),
    Failed { message: String },
}
