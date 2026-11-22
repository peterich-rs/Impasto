//! List window, scroll clamp, and the driver state machines.
//!
//! Language bindings supply a key table and apply render ops. Measurement,
//! chrome, and the pull threshold stay here.

mod controller;
mod diff;
mod driver;
mod prefix;
mod set;
mod window;

pub use diff::ListOp;
pub use driver::{Chrome, ListItem, ListNodes};
pub use set::{DriverEvent, DriverId, DriverSet, ScrollConfig};
pub(crate) use window::pull_delta;

#[cfg(test)]
mod tests;
