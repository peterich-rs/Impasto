//! QuickJS context, job drain, and the shared timer selection rule.
//!
//! A [`Vm`] is created on the thread that runs JavaScript and is never sent
//! elsewhere. Interrupt and rejection hooks capture the address of that `Vm`
//! only after it sits in its final allocation.

pub mod binder;
pub(crate) mod buffer;
pub mod codec;
mod entry;
mod extension;
pub use entry::TrustedBytecode;
mod vm;
pub use extension::{ExtensionContext, exception_text};
pub use vm::JobReport;
#[cfg(test)]
pub(crate) use vm::acquires;
pub(crate) use vm::{AcquireStop, run_job_batch};

pub use vm::Budget;
pub(crate) use vm::{Active, Vm};
pub use vm::{JobStop, JsDiagnostic, Saved, Timer, poll_timer};
