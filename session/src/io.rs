//! Work that leaves the render thread and comes back as a [`PluginResult`].
//!
//! Language globals such as `fetch` and framework modules such as `Clipboard`
//! share this job list. The host decides which thread runs each job.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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

/// One HTTP call. `cancel` becomes true when the script aborts it.
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub token: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub cancel: Arc<AtomicBool>,
}

/// Bytes and headers from one completed response. The body is capped by the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A recoverable HTTP failure. The message is safe to reject a `fetch` promise with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpError {
    pub message: String,
}

impl HttpError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Blocking HTTP used by the host network thread and by session tests.
pub trait HttpClient: Send + Sync {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError>;
}

/// One unit of work posted from the script.
#[derive(Clone, Debug)]
pub enum IoJob {
    Http(HttpRequest),
    Abort {
        token: u64,
    },
    Plugin {
        token: u64,
        name: String,
        method: String,
        args: Vec<PluginValue>,
    },
    SocketOpen {
        id: u64,
        url: String,
    },
    SocketSend {
        id: u64,
        text: bool,
        data: Vec<u8>,
    },
    SocketClose {
        id: u64,
    },
}

/// Largest response body `fetch` will accept.
pub const HTTP_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// Where a host-object method runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPlace {
    /// Reads session memory on the render thread.
    Session,
    /// Leaves the render thread. Always completes through a promise.
    Platform,
}

/// Whether the script sees a return value or a promise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostWait {
    Sync,
    Promise,
}

/// One method on a host object. The script calls it by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostMethod {
    pub module: &'static str,
    pub name: &'static str,
    pub place: HostPlace,
    pub wait: HostWait,
}

const HOST_METHODS: &[HostMethod] = &[
    HostMethod {
        module: "AppState",
        name: "getCurrentState",
        place: HostPlace::Session,
        wait: HostWait::Sync,
    },
    HostMethod {
        module: "Linking",
        name: "getInitialURL",
        place: HostPlace::Session,
        wait: HostWait::Sync,
    },
    HostMethod {
        module: "Linking",
        name: "openURL",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "Clipboard",
        name: "getString",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "Clipboard",
        name: "setString",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "FileSystem",
        name: "documentDirectory",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "FileSystem",
        name: "read",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "FileSystem",
        name: "write",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "FileSystem",
        name: "delete",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "FileSystem",
        name: "pick",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "AsyncStorage",
        name: "multiGet",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "AsyncStorage",
        name: "multiSet",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "AsyncStorage",
        name: "multiRemove",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "AsyncStorage",
        name: "clear",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "Permissions",
        name: "checkPermissionStatus",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "Permissions",
        name: "requestPermissions",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "Permissions",
        name: "shouldShowRequestPermissionRationale",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "Permissions",
        name: "checkServiceStatus",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
    HostMethod {
        module: "Permissions",
        name: "openAppSettings",
        place: HostPlace::Platform,
        wait: HostWait::Promise,
    },
];

/// The method row, if this module exposes `name`.
pub fn host_method(module: &str, name: &str) -> Option<&'static HostMethod> {
    HOST_METHODS
        .iter()
        .find(|method| method.module == module && method.name == name)
}

/// True when the module has at least one method. Camera stays unregistered.
pub fn host_module_linked(module: &str) -> bool {
    HOST_METHODS.iter().any(|method| method.module == module)
}

/// Methods of one module, in table order.
pub fn host_module_methods(module: &str) -> impl Iterator<Item = &'static HostMethod> {
    HOST_METHODS
        .iter()
        .filter(move |method| method.module == module)
}

/// `module "<name>" is not linked`, or with the method when the name exists but the method does not.
pub fn module_not_linked(module: &str, method: Option<&str>) -> String {
    match method {
        Some(method) => format!("module \"{module}.{method}\" is not linked"),
        None => format!("module \"{module}\" is not linked"),
    }
}

/// A path relative to the document directory. Rejects `..`, absolute paths, and backslashes.
pub fn document_relative(path: &str) -> Result<String, String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains(':')
    {
        return Err("path is outside the document directory".to_owned());
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("path is outside the document directory".to_owned());
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}
