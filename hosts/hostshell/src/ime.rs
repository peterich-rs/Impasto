//! Publishes IME visibility. The string itself stays in the focused control.

use std::cell::RefCell;

use crate::Services;

pub fn publish(session: &mut dyn session::Session, services: &RefCell<Box<dyn Services>>) {
    if let Some(update) = session.take_ime() {
        services.borrow().apply_ime(&update);
    }
}
