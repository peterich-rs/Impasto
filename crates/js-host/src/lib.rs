//! Owned values exchanged with a JavaScript host.
//!
//! The crate stays on `std`. UI sessions, trees, and renderers are not
//! dependencies. A session reexports these types under their existing paths.

mod declaration;
mod io;
mod modules;
mod registry;
mod value;

pub use io::{
    Credit, CreditPool, CreditUsage, Delivery, HTTP_BODY_LIMIT, HostEvent, HttpClient, HttpError,
    HttpFailure, HttpPiece, HttpRequest, HttpResponse, IoEvent, IoJob, MAX_SAFE_ID, RequestKey,
    Route, RuntimeKey, TERMINAL_PAYLOAD_LIMIT,
};

pub use declaration::DeclarationError;
pub use modules::{
    EventType, ImmediateExec, Invoke, LISTENER_EXPORTS, Method, MethodExec, MethodTypes, Module,
    ModuleExports, PromiseFn, Settle, SyncFn, Work, module_not_linked,
};
pub use registry::{ModuleRegistry, RegistryError, valid_specifier};
pub use value::{PluginResult, PluginValue};
