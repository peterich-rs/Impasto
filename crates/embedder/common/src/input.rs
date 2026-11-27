use crate::command::Command;
use std::collections::{HashMap, VecDeque};

use engine::input::{Button, Buttons, Phase, PointerEvent, PointerHistory, PointerId};
use engine::session::{InputEvent, Metrics};

fn cancelled(event: PointerEvent, time_ms: f64) -> PointerEvent {
    PointerEvent {
        phase: Phase::Cancel,
        time_ms,
        button: Button::None,
        buttons: Buttons::empty(),
        ..event
    }
}

/// Pointer filtering and "should another frame be requested" for one shell.
#[derive(Debug)]
pub struct Input {
    events: VecDeque<FrameInput>,
    /// Pointers frozen at `pause`, including one synthetic cancel per active id.
    frozen: Option<VecDeque<FrameInput>>,
    active: HashMap<PointerId, PointerEvent>,
    paused: bool,
    pending_metrics: Option<Metrics>,
    sent_metrics: Option<Metrics>,
}

#[derive(Debug, PartialEq)]
pub enum FrameInput {
    Pointer(PointerEvent),
    PointerMoves {
        event: PointerEvent,
        history: Box<PointerHistory>,
    },
    Session(InputEvent),
    DismissIme,
}

pub struct FrameIn {
    pub events: VecDeque<FrameInput>,
    pub metrics: Option<Metrics>,
    pub paused: bool,
}

impl Input {
    pub fn new() -> Self {
        Self {
            events: VecDeque::new(),
            frozen: None,
            active: HashMap::new(),
            paused: false,
            pending_metrics: None,
            sent_metrics: None,
        }
    }

    /// Returns whether the platform should arm the next vsync.
    pub fn push_pointer(&mut self, event: PointerEvent) -> bool {
        self.queue_pointer(event, None)
    }

    pub(crate) fn push_pointer_moves(
        &mut self,
        event: PointerEvent,
        history: Box<PointerHistory>,
    ) -> bool {
        self.queue_pointer(event, Some(history))
    }

    fn queue_pointer(
        &mut self,
        mut event: PointerEvent,
        history: Option<Box<PointerHistory>>,
    ) -> bool {
        match event.phase {
            Phase::Down => {
                if let Some(prior) = self.active.insert(event.pointer, event)
                    && prior
                        .buttons
                        .contains(Buttons::from_button(event.button).bits())
                {
                    self.events
                        .push_back(FrameInput::Pointer(cancelled(prior, event.time_ms)));
                }
            }
            Phase::Move => {
                if let Some(prior) = self.active.get_mut(&event.pointer) {
                    *prior = event;
                }
            }
            Phase::Up => {
                let released = Buttons::from_button(event.button).bits();
                if event.buttons.bits() & !released == 0 {
                    self.active.remove(&event.pointer);
                } else if let Some(prior) = self.active.get_mut(&event.pointer) {
                    *prior = event;
                }
            }
            Phase::Cancel => {
                if let Some(prior) = self.active.remove(&event.pointer) {
                    let time_ms = prior.time_ms.max(event.time_ms);
                    if !event.x.is_finite() || !event.y.is_finite() {
                        event = cancelled(prior, time_ms);
                    } else {
                        event = cancelled(event, time_ms);
                    }
                } else if !event.x.is_finite() || !event.y.is_finite() {
                    return false;
                }
            }
            Phase::Wheel | Phase::Zoom => {}
        }
        let merge = self.events.back().is_some_and(|previous| match previous {
            FrameInput::Pointer(previous)
            | FrameInput::PointerMoves {
                event: previous, ..
            } => {
                previous.phase == Phase::Move
                    && event.phase == Phase::Move
                    && previous.pointer == event.pointer
                    && previous.kind == event.kind
                    && previous.button == event.button
                    && previous.buttons == event.buttons
                    && previous.modifiers == event.modifiers
                    && previous.time_ms <= event.time_ms
            }
            _ => false,
        });
        if merge && let Some(previous) = self.events.pop_back() {
            let (previous, mut samples) = match previous {
                FrameInput::Pointer(previous) => (previous, Box::new(PointerHistory::default())),
                FrameInput::PointerMoves { event, history } => (event, history),
                _ => unreachable!("only moves can be merged"),
            };
            samples.push(previous.into());
            if let Some(history) = history {
                for &sample in history.as_slice() {
                    if sample.time_ms > previous.time_ms && sample.time_ms < event.time_ms {
                        samples.push(sample);
                    }
                }
            }
            self.events.push_back(FrameInput::PointerMoves {
                event,
                history: samples,
            });
        } else if let Some(history) = history {
            self.events
                .push_back(FrameInput::PointerMoves { event, history });
        } else {
            self.events.push_back(FrameInput::Pointer(event));
        }
        !self.paused
    }

    /// Queues a key or text event. Returns whether the platform should arm vsync.
    /// Paused input stays queued until resume, matching [`Input::push_pointer`].
    pub fn push_input(&mut self, event: InputEvent) -> bool {
        self.events.push_back(FrameInput::Session(event));
        !self.paused
    }

    /// The keyboard went away. Delivered once on the next frame.
    pub fn dismiss_ime(&mut self) -> bool {
        if !matches!(self.events.back(), Some(FrameInput::DismissIme)) {
            self.events.push_back(FrameInput::DismissIme);
        }
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
        let mut deferred = VecDeque::new();
        let mut frozen = VecDeque::new();
        for event in self.events.drain(..) {
            if matches!(
                event,
                FrameInput::Pointer(_) | FrameInput::PointerMoves { .. }
            ) {
                frozen.push_back(event);
            } else {
                deferred.push_back(event);
            }
        }
        self.events = deferred;
        frozen.extend(
            self.active
                .drain()
                .map(|(_, event)| FrameInput::Pointer(cancelled(event, event.time_ms))),
        );
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
        if self.pending_metrics.or(self.sent_metrics) == Some(metrics) {
            return false;
        }
        if self.sent_metrics == Some(metrics) {
            self.pending_metrics = None;
            return false;
        }
        self.pending_metrics = Some(metrics);
        !self.paused
    }

    pub fn reset_for_source_update(&mut self) {
        self.events.clear();
        self.frozen = None;
        self.active.clear();
    }

    pub fn begin_frame(&mut self) -> FrameIn {
        if let Some(events) = self.frozen.take() {
            return FrameIn {
                events,
                metrics: None,
                paused: true,
            };
        }
        if self.paused {
            return FrameIn {
                events: VecDeque::new(),
                metrics: None,
                paused: true,
            };
        }
        let metrics = self.pending_metrics.take();
        if let Some(metrics) = metrics {
            self.sent_metrics = Some(metrics);
        }
        FrameIn {
            events: std::mem::take(&mut self.events),
            metrics,
            paused: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::input::Phase;
    use engine::session::InputEvent;

    fn event(phase: Phase, x: f32) -> PointerEvent {
        PointerEvent::new(phase, x, 0.0, 0.0)
    }

    fn phases(input: &Input) -> Vec<Phase> {
        input
            .events
            .iter()
            .filter_map(pointer)
            .map(|event| event.phase)
            .collect()
    }

    fn pointer(event: &FrameInput) -> Option<&PointerEvent> {
        match event {
            FrameInput::Pointer(event) | FrameInput::PointerMoves { event, .. } => Some(event),
            _ => None,
        }
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
        assert!(input.begin_frame().events.is_empty());
    }

    #[test]
    fn a_tap_keeps_its_two_real_events() {
        let mut input = Input::new();
        input.push_pointer(event(Phase::Down, 4.0));
        input.push_pointer(event(Phase::Up, 8.0));
        assert_eq!(phases(&input), [Phase::Down, Phase::Up]);
        assert!(input.active.is_empty());
    }

    #[test]
    fn blank_cancel_uses_the_last_real_point_and_keeps_the_cancel_time() {
        let mut input = Input::new();
        input.push_pointer(event(Phase::Down, 4.0));
        input.push_pointer(event(Phase::Move, -6.0));
        let mut cancel = event(Phase::Cancel, f32::NAN);
        cancel.time_ms = 10.0;
        input.push_pointer(cancel);
        assert_eq!(phases(&input), [Phase::Down, Phase::Move, Phase::Cancel]);
        let last = input.events.back().and_then(pointer).expect("cancel");
        assert_eq!(last.x, -6.0);
        assert_eq!(last.time_ms, 10.0);
    }

    #[test]
    fn coalescing_preserves_real_history_but_dispatches_only_latest_position() {
        let mut input = Input::new();
        input.push_pointer(PointerEvent::new(Phase::Move, 6.0, 4.0, 10.0));
        let mut history = Box::new(PointerHistory::default());
        history.push(PointerEvent::new(Phase::Move, -1.0, 4.0, 14.0).into());
        input.push_pointer_moves(PointerEvent::new(Phase::Move, -3.0, 4.0, 18.0), history);
        input.push_pointer(PointerEvent::new(Phase::Move, -5.0, 4.0, 22.0));
        let mut frame = input.begin_frame();
        let Some(FrameInput::PointerMoves { event, history }) = frame.events.pop_front() else {
            panic!("move with history");
        };
        assert!(frame.events.is_empty());
        assert_eq!((event.x, event.time_ms), (-5.0, 22.0));
        let points: Vec<_> = history
            .as_slice()
            .iter()
            .map(|s| (s.x, s.time_ms))
            .collect();
        assert_eq!(points, [(6.0, 10.0), (-1.0, 14.0), (-3.0, 18.0)]);
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
                .events
                .iter()
                .filter_map(pointer)
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
        assert!(
            !input.push_input(InputEvent::Key(engine::session::KeyPress::new(
                engine::session::Key::Backspace
            )))
        );
        assert!(!input.dismiss_ime());
        let frame = input.begin_frame();
        assert!(frame.paused);
        assert!(frame.events.is_empty());
        assert!(input.resume());
        let next = input.begin_frame();
        assert_eq!(
            next.events,
            vec![
                FrameInput::Session(InputEvent::Text("a".to_string())),
                FrameInput::Session(InputEvent::Key(engine::session::KeyPress::new(
                    engine::session::Key::Backspace
                ))),
                FrameInput::DismissIme,
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
                .events
                .iter()
                .filter_map(pointer)
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![Phase::Down, Phase::Cancel]
        );
        assert!(input.resume());
        let next = input.begin_frame();
        assert_eq!(
            next.events
                .iter()
                .filter_map(pointer)
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![Phase::Move]
        );
    }

    #[test]
    fn a_text_edit_and_dismissal_precede_the_next_pointer_focus_change() {
        let mut input = Input::new();
        let text = InputEvent::Text("a".to_string());
        let next_focus = event(Phase::Down, 20.0);
        input.push_input(text.clone());
        input.dismiss_ime();
        input.push_pointer(next_focus);
        assert_eq!(
            input.begin_frame().events,
            [
                FrameInput::Session(text),
                FrameInput::DismissIme,
                FrameInput::Pointer(next_focus)
            ],
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

    #[test]
    fn returning_to_the_sent_metrics_before_a_frame_drops_the_intermediate_change() {
        let mut input = Input::new();
        let portrait = Metrics::new(180.0, 320.0, 2.0);
        let landscape = Metrics::new(320.0, 180.0, 2.0);
        assert!(input.set_metrics(portrait));
        assert_eq!(input.begin_frame().metrics, Some(portrait));
        assert!(input.set_metrics(landscape));
        assert!(!input.set_metrics(portrait));
        assert!(input.begin_frame().metrics.is_none());
    }
}

pub(crate) fn push_coalesced(commands: &mut VecDeque<Command>, command: Command) {
    if let Command::BackGesture(engine::session::BackGesture::Progress(next)) = command
        && let Some(Command::BackGesture(engine::session::BackGesture::Progress(previous))) =
            commands.back_mut()
    {
        *previous = next;
        return;
    }
    if let Command::WindowStateChanged(key) = &command
        && let Some(index) = commands.iter().rposition(
            |queued| matches!(queued, Command::WindowStateChanged(queued) if key == queued),
        )
    {
        commands.remove(index);
    }
    if let Command::ExternalTexture { source, .. } | Command::ExternalGpuTexture { source, .. } =
        &command
        && let Some(index) = commands.iter().rposition(|queued| {
            matches!(queued, Command::ExternalTexture { source: queued, .. }
                | Command::ExternalGpuTexture { source: queued, .. } if source == queued)
        })
    {
        commands.remove(index);
    }
    commands.push_back(command);
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use engine::input::{Phase, PointerEvent};

    fn move_event(x: f32, time_ms: f64) -> PointerEvent {
        PointerEvent::new(Phase::Move, x, 0.0, time_ms)
    }

    #[test]
    fn a_move_after_a_down_stays_in_order() {
        let mut commands = VecDeque::new();
        push_coalesced(
            &mut commands,
            Command::Pointer(PointerEvent::new(Phase::Down, 0.0, 0.0, 0.0)),
        );
        push_coalesced(&mut commands, Command::Pointer(move_event(3.0, 4.0)));
        assert_eq!(commands.len(), 2);
    }

    #[test]
    fn wheels_stay_in_order() {
        let mut commands = VecDeque::new();
        push_coalesced(
            &mut commands,
            Command::Pointer(PointerEvent::wheel(1.0, 2.0, 0.0, 0.0, 4.0)),
        );
        push_coalesced(
            &mut commands,
            Command::Pointer(PointerEvent::wheel(1.0, 2.0, 8.0, 0.0, 6.0)),
        );
        assert_eq!(commands.len(), 2);
    }
}
