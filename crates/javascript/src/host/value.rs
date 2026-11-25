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
#[derive(Debug)]
pub enum PluginResult {
    Done(PluginValue),
    Resource(ResourceFactory),
    Failed {
        message: String,
    },
    Rejected {
        message: String,
    },
    Scoped {
        cancel: super::CancelToken,
        result: Box<Self>,
    },
}

impl PluginResult {
    pub(super) fn payload_bytes(&self) -> usize {
        let mut result = self;
        loop {
            match result {
                Self::Done(value) => return value.payload_bytes(),
                Self::Resource(_) => return 0,
                Self::Failed { message } | Self::Rejected { message } => return message.len(),
                Self::Scoped { result: inner, .. } => result = inner,
            }
        }
    }
}

/// Owned state that is consumed to create a resource on the VM queue.
pub struct ResourceFactory(Box<dyn FnOnce() -> Box<dyn super::NativeResource> + Send>);

impl ResourceFactory {
    pub fn new(create: impl FnOnce() -> Box<dyn super::NativeResource> + Send + 'static) -> Self {
        Self(Box::new(create))
    }

    pub fn open(self) -> Box<dyn super::NativeResource> {
        (self.0)()
    }
}

impl std::fmt::Debug for ResourceFactory {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str("ResourceFactory")
    }
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
