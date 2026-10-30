use std::collections::{HashSet, VecDeque};

use session::{InputEvent, Metrics};
use uicore::{Button, Buttons, Phase, PointerEvent, PointerId, PointerKind};

/// Pointer filtering and "should another frame be requested" for one shell.
#[derive(Debug)]
pub struct Input {
    pointers: VecDeque<PointerEvent>,
    inputs: VecDeque<InputEvent>,
    /// Pointers frozen at `pause`, including one synthetic cancel per active id.
    frozen: Option<VecDeque<PointerEvent>>,
    ime_dismissed: bool,
    active: HashSet<PointerId>,
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
            frozen: None,
            ime_dismissed: false,
            active: HashSet::new(),
            paused: false,
            pending_metrics: None,
            sent_metrics: None,
        }
    }

    /// Returns whether the platform should arm the next vsync.
    pub fn push_pointer(&mut self, event: PointerEvent) -> bool {
        match event.phase {
            Phase::Down => {
                self.active.insert(event.pointer);
            }
            Phase::Up | Phase::Cancel => {
                self.active.remove(&event.pointer);
            }
            Phase::Move | Phase::Wheel | Phase::Zoom => {}
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

    /// Pauses input. An open press is frozen with a synthetic cancel and delivered
    /// on the next frame. Pointers, keys, and metrics that arrive after this stay
    /// queued until [`Input::resume`].
    pub fn pause(&mut self) -> bool {
        self.paused = true;
        if self.frozen.is_some() || self.active.is_empty() {
            return false;
        }
        let mut frozen = std::mem::take(&mut self.pointers);
        let ids: Vec<PointerId> = self.active.drain().collect();
        for id in ids {
            frozen.push_back(cancel_event(id));
        }
        self.frozen = Some(frozen);
        true
    }

    pub fn resume(&mut self) -> bool {
        self.paused = false;
        true
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_metrics(&mut self, metrics: Metrics) -> bool {
        if self.sent_metrics == Some(metrics) || self.pending_metrics == Some(metrics) {
            return false;
        }
        self.pending_metrics = Some(metrics);
        !self.paused
    }

    pub fn begin_frame(&mut self) -> FrameIn {
        if let Some(pointers) = self.frozen.take() {
            return FrameIn {
                pointers,
                inputs: VecDeque::new(),
                metrics: None,
                paused: true,
                ime_dismissed: false,
            };
        }
        if self.paused {
            return FrameIn {
                pointers: VecDeque::new(),
                inputs: VecDeque::new(),
                metrics: None,
                paused: true,
                ime_dismissed: false,
            };
        }
        let metrics = self.pending_metrics.take();
        if let Some(metrics) = metrics {
            self.sent_metrics = Some(metrics);
        }
        FrameIn {
            pointers: std::mem::take(&mut self.pointers),
            inputs: std::mem::take(&mut self.inputs),
            metrics,
            paused: false,
            ime_dismissed: std::mem::take(&mut self.ime_dismissed),
        }
    }
}

fn cancel_event(pointer: PointerId) -> PointerEvent {
    PointerEvent {
        pointer,
        kind: PointerKind::Mouse,
        phase: Phase::Cancel,
        x: 0.0,
        y: 0.0,
        time_ms: 0.0,
        button: Button::None,
        buttons: Buttons::empty(),
        modifiers: uicore::Modifiers::empty(),
        dx: 0.0,
        dy: 0.0,
        zoom: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::InputEvent;
    use uicore::Phase;

    fn event(phase: Phase, x: f32) -> PointerEvent {
        PointerEvent::new(phase, x, 0.0, 0.0)
    }

    fn phases(input: &Input) -> Vec<Phase> {
        input.pointers.iter().map(|event| event.phase).collect()
    }

    #[test]
    fn moves_outside_a_press_are_queued_for_hover() {
        let mut input = Input::new();
        assert!(input.push_pointer(event(Phase::Move, 1.0)));
        assert!(input.push_pointer(event(Phase::Down, 1.0)));
        assert!(input.push_pointer(event(Phase::Move, 2.0)));
        assert!(input.push_pointer(event(Phase::Up, 2.0)));
        assert!(input.push_pointer(event(Phase::Move, 3.0)));
        assert!(input.push_pointer(PointerEvent::wheel(0.0, 0.0, 0.0, 0.0, 8.0)));
        assert!(input.push_pointer(event(Phase::Cancel, 0.0)));
        assert_eq!(
            phases(&input),
            vec![
                Phase::Move,
                Phase::Down,
                Phase::Move,
                Phase::Up,
                Phase::Move,
                Phase::Wheel,
                Phase::Cancel
            ]
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
        assert!(!input.push_input(InputEvent::Key(session::KeyPress::new(
            session::Key::Backspace
        ))));
        assert!(!input.dismiss_ime());
        let frame = input.begin_frame();
        assert!(frame.paused);
        assert!(!frame.ime_dismissed);
        assert!(frame.inputs.is_empty());
        assert!(input.resume());
        let next = input.begin_frame();
        assert!(next.ime_dismissed);
        assert_eq!(
            next.inputs,
            vec![
                InputEvent::Text("a".to_string()),
                InputEvent::Key(session::KeyPress::new(session::Key::Backspace))
            ]
        );
    }

    #[test]
    fn a_move_after_pause_waits_until_resume() {
        let mut input = Input::new();
        input.push_pointer(event(Phase::Down, 1.0));
        assert!(input.pause());
        assert!(!input.push_pointer(event(Phase::Move, 4.0)));
        let frame = input.begin_frame();
        assert_eq!(
            frame
                .pointers
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![Phase::Down, Phase::Cancel]
        );
        assert!(input.resume());
        let next = input.begin_frame();
        assert_eq!(
            next.pointers
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![Phase::Move]
        );
    }

    #[test]
    fn identical_metrics_do_not_request_another_frame() {
        let metrics = Metrics::new(320.0, 180.0, 2.0);
        let mut input = Input::new();
        assert!(input.set_metrics(metrics));
        let _ = input.begin_frame();
        assert!(!input.set_metrics(metrics));
    }
}
