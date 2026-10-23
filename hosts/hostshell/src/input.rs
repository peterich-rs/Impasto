use std::collections::VecDeque;

use session::{InputEvent, Metrics};
use ui_core::{Phase, PointerEvent};

/// Pointer filtering and "should another frame be requested" for one shell.
#[derive(Debug)]
pub struct Input {
    pointers: VecDeque<PointerEvent>,
    inputs: VecDeque<InputEvent>,
    ime_dismissed: bool,
    down: bool,
    paused: bool,
    pending_metrics: Option<Metrics>,
    sent_metrics: Option<Metrics>,
}

pub struct FrameIn {
    pub pointers: VecDeque<PointerEvent>,
    pub inputs: VecDeque<InputEvent>,
    pub metrics: Option<Metrics>,
    pub paused: bool,
    pub ime_dismissed: bool,
}

impl Input {
    pub fn new() -> Self {
        Self {
            pointers: VecDeque::new(),
            inputs: VecDeque::new(),
            ime_dismissed: false,
            down: false,
            paused: false,
            pending_metrics: None,
            sent_metrics: None,
        }
    }

    /// Returns whether the platform should arm the next vsync.
    pub fn push_pointer(&mut self, event: PointerEvent) -> bool {
        if event.phase == Phase::Move && !self.down {
            return false;
        }
        if event.phase == Phase::Down {
            self.down = true;
        }
        if matches!(event.phase, Phase::Up | Phase::Cancel) {
            self.down = false;
        }
        self.pointers.push_back(event);
        !self.paused
    }

    /// Queues a key or text event. Returns whether the platform should arm vsync.
    /// Paused input stays queued until resume, matching [`Input::push_pointer`].
    pub fn push_input(&mut self, event: InputEvent) -> bool {
        self.inputs.push_back(event);
        !self.paused
    }

    /// The keyboard went away. Delivered once on the next frame.
    pub fn dismiss_ime(&mut self) -> bool {
        self.ime_dismissed = true;
        !self.paused
    }

    /// Pauses input. Returns whether a frame is needed to deliver a synthetic cancel.
    pub fn pause(&mut self) -> bool {
        self.paused = true;
        if self.down {
            self.pointers.push_back(PointerEvent {
                phase: Phase::Cancel,
                x: 0.0,
                y: 0.0,
                time_ms: 0.0,
            });
            self.down = false;
            return true;
        }
        false
    }

    pub fn resume(&mut self) -> bool {
        self.paused = false;
        true
    }

    pub fn set_metrics(&mut self, metrics: Metrics) -> bool {
        if self.sent_metrics == Some(metrics) || self.pending_metrics == Some(metrics) {
            return false;
        }
        self.pending_metrics = Some(metrics);
        !self.paused
    }

    pub fn begin_frame(&mut self) -> FrameIn {
        let metrics = self.pending_metrics.take();
        if let Some(metrics) = metrics {
            self.sent_metrics = Some(metrics);
        }
        FrameIn {
            pointers: std::mem::take(&mut self.pointers),
            inputs: std::mem::take(&mut self.inputs),
            metrics,
            paused: self.paused,
            ime_dismissed: std::mem::take(&mut self.ime_dismissed),
        }
    }
}

pub fn same_draw_list(previous: Option<&protocol::DrawList>, next: &protocol::DrawList) -> bool {
    previous == Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::InputEvent;
    use ui_core::Phase;

    fn event(phase: Phase, x: f32) -> PointerEvent {
        PointerEvent {
            phase,
            x,
            y: 0.0,
            time_ms: 0.0,
        }
    }

    fn phases(input: &Input) -> Vec<Phase> {
        input.pointers.iter().map(|event| event.phase).collect()
    }

    #[test]
    fn moves_outside_a_press_are_dropped_and_cancel_passes() {
        let mut input = Input::new();
        assert!(!input.push_pointer(event(Phase::Move, 1.0)));
        assert!(input.pointers.is_empty());
        assert!(input.push_pointer(event(Phase::Down, 1.0)));
        assert!(input.push_pointer(event(Phase::Move, 2.0)));
        assert!(input.push_pointer(event(Phase::Up, 2.0)));
        assert!(!input.push_pointer(event(Phase::Move, 3.0)));
        assert!(input.push_pointer(event(Phase::Cancel, 0.0)));
        assert_eq!(
            phases(&input),
            vec![Phase::Down, Phase::Move, Phase::Up, Phase::Cancel]
        );
    }

    #[test]
    fn pause_without_a_press_does_not_request_a_frame() {
        let mut input = Input::new();
        assert!(!input.pause());
        assert!(input.begin_frame().paused);
        assert!(input.begin_frame().pointers.is_empty());
    }

    #[test]
    fn pause_cancels_an_open_press() {
        let mut input = Input::new();
        input.push_pointer(event(Phase::Down, 1.0));
        assert!(input.pause());
        let frame = input.begin_frame();
        assert!(frame.paused);
        assert_eq!(
            frame
                .pointers
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![Phase::Down, Phase::Cancel]
        );
    }

    #[test]
    fn input_is_queued_and_pause_does_not_arm_a_frame() {
        let mut input = Input::new();
        assert!(input.push_input(InputEvent::Text("a".to_string())));
        input.pause();
        assert!(!input.push_input(InputEvent::Key(session::Key::Backspace)));
        assert!(!input.dismiss_ime());
        let frame = input.begin_frame();
        assert!(frame.paused);
        assert!(frame.ime_dismissed);
        assert_eq!(
            frame.inputs,
            vec![
                InputEvent::Text("a".to_string()),
                InputEvent::Key(session::Key::Backspace)
            ]
        );
        let next = input.begin_frame();
        assert!(!next.ime_dismissed);
        assert!(next.inputs.is_empty());
    }

    #[test]
    fn identical_metrics_do_not_request_another_frame() {
        let metrics = Metrics {
            width: 320.0,
            height: 180.0,
            device_pixel_ratio: 2.0,
            inset_left: 0.0,
            inset_top: 0.0,
            inset_right: 0.0,
            inset_bottom: 0.0,
        };
        let mut input = Input::new();
        assert!(input.set_metrics(metrics));
        let _ = input.begin_frame();
        assert!(!input.set_metrics(metrics));
    }

    #[test]
    fn an_unchanged_draw_list_is_not_submitted_again() {
        let list = protocol::DrawList {
            commands: Vec::new(),
        };
        assert!(same_draw_list(Some(&list), &list));
        assert!(!same_draw_list(None, &list));
    }
}
