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

impl PluginValue {
    /// Estimate owned payload size with bounded depth and traversal.
    pub fn payload_bytes(&self) -> usize {
        fn visit(value: &PluginValue, depth: usize, left: &mut usize) -> usize {
            if depth > 256 || *left == 0 {
                return usize::MAX;
            }
            *left -= 1;
            match value {
                PluginValue::Null | PluginValue::Bool(_) | PluginValue::F64(_) => 8,
                PluginValue::Str(text) => text.len(),
                PluginValue::Bytes(bytes) => bytes.len(),
                PluginValue::List(items) => {
                    if items.len() > *left {
                        return usize::MAX;
                    }
                    items
                        .iter()
                        .try_fold(0usize, |sum, item| {
                            sum.checked_add(visit(item, depth + 1, left))
                        })
                        .unwrap_or(usize::MAX)
                }
                PluginValue::Map(items) => {
                    if items.len() > *left {
                        return usize::MAX;
                    }
                    items
                        .iter()
                        .try_fold(0usize, |sum, (key, item)| {
                            sum.checked_add(key.len())?
                                .checked_add(visit(item, depth + 1, left))
                        })
                        .unwrap_or(usize::MAX)
                }
            }
        }
        visit(self, 0, &mut 65_536)
    }
}
