//! When the next vsync is armed, and what a frame should do with its list.

use crate::command::FrameToken;
use crate::present::AfterPresent;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameView {
    pub surface_open: bool,
    pub paused: bool,
    pub fixture_open: bool,
    pub fixture_presented: bool,
    pub unchanged: bool,
    pub needs_present: bool,
    pub again: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameStep {
    Cancel,
    Skip { again: bool },
    Submit,
    PresentCached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PresentFollowup {
    pub needs_present: bool,
    pub mark_fixture_presented: bool,
    pub clear_surface: bool,
    pub vsync: VsyncOp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VsyncOp {
    Request,
    Cancel,
    Stop,
}

pub(crate) fn frame_step(view: &FrameView) -> FrameStep {
    if !view.surface_open || view.paused || (view.fixture_open && view.fixture_presented) {
        return FrameStep::Cancel;
    }
    if view.unchanged && !view.needs_present {
        return FrameStep::Skip { again: view.again };
    }
    if view.unchanged {
        FrameStep::PresentCached
    } else {
        FrameStep::Submit
    }
}

pub(crate) fn present_followup(after: AfterPresent, is_fixture: bool) -> PresentFollowup {
    match after {
        AfterPresent::Shown { again } => PresentFollowup {
            needs_present: false,
            mark_fixture_presented: is_fixture,
            clear_surface: false,
            vsync: if again {
                VsyncOp::Request
            } else {
                VsyncOp::Cancel
            },
        },
        AfterPresent::Stop => PresentFollowup {
            needs_present: false,
            mark_fixture_presented: false,
            clear_surface: false,
            vsync: VsyncOp::Stop,
        },
        AfterPresent::Retry => PresentFollowup {
            needs_present: true,
            mark_fixture_presented: false,
            clear_surface: false,
            vsync: VsyncOp::Request,
        },
        AfterPresent::Fatal => PresentFollowup {
            needs_present: false,
            mark_fixture_presented: false,
            clear_surface: false,
            vsync: VsyncOp::Stop,
        },
    }
}

/// Surface destruction clears the last list. Present failures do not.
pub(crate) fn surface_lost() -> PresentFollowup {
    PresentFollowup {
        needs_present: false,
        mark_fixture_presented: false,
        clear_surface: true,
        vsync: VsyncOp::Cancel,
    }
}

/// One armed vsync at a time. A request during an in-flight frame is remembered.
pub struct FrameScheduler {
    next: u64,
    armed: Option<FrameToken>,
    in_flight: bool,
    pending: bool,
}

impl FrameScheduler {
    pub fn new() -> Self {
        Self {
            next: 0,
            armed: None,
            in_flight: false,
            pending: false,
        }
    }

    /// Arms one token. Returns `None` when a token is already armed or a frame
    /// is running; that call sets [`Self::pending`] so [`Self::finish`] can arm
    /// the next one.
    pub fn request(&mut self) -> Option<FrameToken> {
        if self.armed.is_some() || self.in_flight {
            self.pending = true;
            return None;
        }
        self.pending = false;
        self.next = self.next.wrapping_add(1);
        let token = FrameToken(self.next);
        self.armed = Some(token);
        Some(token)
    }

    /// Accepts the token the platform echoed. A stale token does not open a frame.
    pub fn accept(&mut self, token: FrameToken) -> bool {
        if self.armed != Some(token) {
            return false;
        }
        self.armed = None;
        self.in_flight = true;
        true
    }

    /// Ends the in-flight frame. `again` is the session's request for one more.
    pub fn finish(&mut self, again: bool) -> Option<FrameToken> {
        self.in_flight = false;
        if again || self.pending {
            self.request()
        } else {
            None
        }
    }

    pub fn cancel(&mut self) {
        self.armed = None;
        self.pending = false;
    }

    pub(crate) fn busy(&self) -> bool {
        self.in_flight
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AfterPresent, FrameScheduler, FrameStep, FrameView, VsyncOp, frame_step, present_followup,
        surface_lost,
    };

    fn open_view() -> FrameView {
        FrameView {
            surface_open: true,
            paused: false,
            fixture_open: false,
            fixture_presented: false,
            unchanged: false,
            needs_present: false,
            again: false,
        }
    }

    #[test]
    fn a_closed_surface_pauses_and_a_finished_fixture_cancel() {
        assert_eq!(
            frame_step(&FrameView {
                surface_open: false,
                ..open_view()
            }),
            FrameStep::Cancel
        );
        assert_eq!(
            frame_step(&FrameView {
                paused: true,
                ..open_view()
            }),
            FrameStep::Cancel
        );
        assert_eq!(
            frame_step(&FrameView {
                fixture_open: true,
                fixture_presented: true,
                ..open_view()
            }),
            FrameStep::Cancel
        );
    }

    #[test]
    fn an_unchanged_list_skips_unless_present_must_retry() {
        assert_eq!(
            frame_step(&FrameView {
                unchanged: true,
                again: true,
                ..open_view()
            }),
            FrameStep::Skip { again: true }
        );
        assert_eq!(
            frame_step(&FrameView {
                unchanged: true,
                ..open_view()
            }),
            FrameStep::Skip { again: false }
        );
        assert_eq!(
            frame_step(&FrameView {
                unchanged: true,
                needs_present: true,
                ..open_view()
            }),
            FrameStep::PresentCached
        );
        assert_eq!(frame_step(&open_view()), FrameStep::Submit);
    }

    #[test]
    fn a_failed_present_retries_and_destruction_clears_the_surface() {
        let retry = present_followup(AfterPresent::Retry, false);
        assert!(retry.needs_present);
        assert_eq!(retry.vsync, VsyncOp::Request);
        assert!(!retry.clear_surface);
        let lost = surface_lost();
        assert!(lost.clear_surface);
        assert_eq!(lost.vsync, VsyncOp::Cancel);
        let shown = present_followup(AfterPresent::Shown { again: false }, true);
        assert!(shown.mark_fixture_presented);
        assert_eq!(shown.vsync, VsyncOp::Cancel);
    }

    #[test]
    fn a_request_during_the_frame_is_armed_when_the_frame_finishes() {
        let mut scheduler = FrameScheduler::new();
        let token = scheduler.request().expect("first token");
        assert!(scheduler.request().is_none());
        assert!(scheduler.accept(token));
        assert!(!scheduler.accept(token));
        let next = scheduler.finish(false).expect("pending request");
        assert_ne!(next, token);
    }

    #[test]
    fn cancel_drops_an_armed_token() {
        let mut scheduler = FrameScheduler::new();
        let token = scheduler.request().expect("token");
        scheduler.cancel();
        assert!(!scheduler.accept(token));
    }
}
