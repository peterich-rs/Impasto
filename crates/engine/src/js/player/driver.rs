//! Reconcile list and scroll drivers once per frame and deliver their events.

use crate::tree::list::DriverEvent;

use super::Inner;

pub(super) fn collect_driver_tick(inner: &Inner) -> Vec<DriverEvent> {
    {
        let mut drivers = inner.drivers.borrow_mut();
        let mut tree = inner.tree.borrow_mut();
        drivers.tick(&mut tree);
    }
    let pending = inner.drivers.borrow().wants_frame();
    inner.driver_frame.set(pending);
    inner.drivers.borrow_mut().take_events()
}
