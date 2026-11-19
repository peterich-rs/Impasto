//! Owned values exchanged with a JavaScript host.
//!
//! The crate stays on `std`. UI sessions, trees, and renderers are not
//! dependencies. A session reexports these types under their existing paths.

mod io;
mod modules;
mod registry;
mod value;

pub use io::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
    IoEvent, IoJob,
};

pub use modules::{
    ImmediateExec, Invoke, Method, MethodExec, Module, PromiseFn, Settle, SyncFn, Work,
    module_not_linked,
};
pub use registry::ModuleRegistry;
pub use value::{PluginResult, PluginValue};
