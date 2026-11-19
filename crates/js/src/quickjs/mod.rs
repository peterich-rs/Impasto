//! QuickJS context, job drain, and the shared timer selection rule.
//!
//! A [`Vm`] is created on the thread that runs JavaScript and is never sent
//! elsewhere. Interrupt and rejection hooks capture the address of that `Vm`
//! only after it sits in its final allocation.

mod vm;

#[allow(
    unused_imports,
    reason = "the UI adapter names Active even though this crate does not"
)]
pub use vm::{Active, JobStop, Timer, Vm, poll_timer, run_jobs};
