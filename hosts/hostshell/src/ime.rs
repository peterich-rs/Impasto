//! Publishes IME visibility. The string itself stays in the focused control.

use std::cell::RefCell;

use crate::Services;

pub fn publish_update(update: &session::ImeUpdate, services: &RefCell<Box<dyn Services>>) {
    services.borrow().apply_ime(update);
}
