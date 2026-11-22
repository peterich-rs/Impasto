use std::collections::{HashSet, VecDeque};

use engine::input::{Button, Buttons, Phase, PointerEvent, PointerId, PointerKind};
use engine::session::{InputEvent, Metrics};

/// Last sample for one pointer. Used to legalize the packet, not to resample it.
#[derive(Clone, Copy, Debug)]
struct Tracked {
    kind: PointerKind,
    x: f32,
    y: f32,
    time_ms: f64,
    button: Button,
    buttons: Buttons,
    modifiers: engine::input::Modifiers,
    pen: Option<engine::input::PenSample>,
    down: bool,
    moved: bool,
}

/// Fills missing samples. Does not invent positions between two real samples.
#[derive(Debug, Default)]
struct PointerConverter {
    tracked: std::collections::HashMap<PointerId, Tracked>,
}

impl PointerConverter {
    fn accept(&mut self, event: PointerEvent) -> Vec<PointerEvent> {
        let mut out = Vec::new();
        match event.phase {
            Phase::Move | Phase::Up | Phase::Cancel => {
                if let Some(tracked) = self.tracked.get(&event.pointer).copied() {
                    let leaving = event.x < 0.0 || event.y < 0.0;
                    let missing_move = event.phase == Phase::Up && tracked.down && !tracked.moved;
                    if leaving || missing_move {
                        let synthesized = move_at(event.pointer, &tracked, event.time_ms);
                        self.note(&synthesized);
                        out.push(synthesized);
                    }
                } else if event.phase == Phase::Move && event.buttons.bits() != 0 {
                    out.push(PointerEvent {
                        phase: Phase::Down,
                        ..event
                    });
                }
            }
            Phase::Down | Phase::Wheel | Phase::Zoom => {}
        }
        let event = if event.phase == Phase::Cancel {
            self.cancel_from(event)
        } else {
            event
        };
        self.note(&event);
        out.push(event);
        out
    }

    fn cancel_from(&self, event: PointerEvent) -> PointerEvent {
        let Some(tracked) = self.tracked.get(&event.pointer) else {
            return event;
        };
        PointerEvent {
            pointer: event.pointer,
            kind: tracked.kind,
            phase: Phase::Cancel,
            x: tracked.x,
            y: tracked.y,
            time_ms: tracked.time_ms,
            button: Button::None,
            buttons: Buttons::empty(),
            modifiers: tracked.modifiers,
            dx: 0.0,
            dy: 0.0,
            zoom: 0.0,
            pen: tracked.pen,
        }
    }

    fn note(&mut self, event: &PointerEvent) {
        if matches!(event.phase, Phase::Wheel | Phase::Zoom) {
            return;
        }
        // Zero is the conventional persistent desktop cursor. Contact IDs
        // can increase on every press, including UIKit's indirect pointers.
        if event.phase == Phase::Cancel
            || (event.phase == Phase::Up
                && (event.pointer != PointerId(0)
                    || matches!(event.kind, PointerKind::Touch | PointerKind::Pen)))
        {
            self.tracked.remove(&event.pointer);
            return;
        }
        let tracked = self.tracked.entry(event.pointer).or_insert(Tracked {
            kind: event.kind,
            x: event.x,
            y: event.y,
            time_ms: event.time_ms,
            button: event.button,
            buttons: event.buttons,
            modifiers: event.modifiers,
            pen: event.pen,
            down: false,
            moved: false,
        });
        tracked.kind = event.kind;
        if event.x >= 0.0 && event.y >= 0.0 {
            tracked.x = event.x;
            tracked.y = event.y;
        }
        tracked.time_ms = event.time_ms;
        tracked.button = event.button;
        tracked.buttons = event.buttons;
        tracked.modifiers = event.modifiers;
        tracked.pen = event.pen;
        match event.phase {
            Phase::Down => {
                tracked.down = true;
                tracked.moved = false;
            }
            Phase::Move => tracked.moved = true,
            Phase::Up => tracked.down = false,
            Phase::Cancel | Phase::Wheel | Phase::Zoom => {}
        }
    }

    fn cancel_event(&self, id: PointerId) -> PointerEvent {
        self.tracked.get(&id).map_or_else(
            || PointerEvent {
                pointer: id,
                kind: PointerKind::Mouse,
                phase: Phase::Cancel,
                x: 0.0,
                y: 0.0,
                time_ms: 0.0,
                button: Button::None,
                buttons: Buttons::empty(),
                modifiers: engine::input::Modifiers::empty(),
                dx: 0.0,
                dy: 0.0,
                zoom: 0.0,
                pen: None,
            },
            |tracked| PointerEvent {
                pointer: id,
                kind: tracked.kind,
                phase: Phase::Cancel,
                x: tracked.x,
                y: tracked.y,
                time_ms: tracked.time_ms,
                button: Button::None,
                buttons: Buttons::empty(),
                modifiers: tracked.modifiers,
                dx: 0.0,
                dy: 0.0,
                zoom: 0.0,
                pen: tracked.pen,
            },
        )
    }
}

fn move_at(pointer: PointerId, tracked: &Tracked, time_ms: f64) -> PointerEvent {
    PointerEvent {
        pointer,
        kind: tracked.kind,
        phase: Phase::Move,
        x: tracked.x,
        y: tracked.y,
        time_ms,
        button: tracked.button,
        buttons: tracked.buttons,
        modifiers: tracked.modifiers,
        dx: 0.0,
        dy: 0.0,
        zoom: 0.0,
        pen: tracked.pen,
    }
}

/// Pointer filtering and "should another frame be requested" for one shell.
#[derive(Debug)]
pub struct Input {
    events: VecDeque<FrameInput>,
    /// Pointers frozen at `pause`, including one synthetic cancel per active id.
    frozen: Option<VecDeque<FrameInput>>,
    active: HashSet<PointerId>,
    converter: PointerConverter,
    paused: bool,
    pending_metrics: Option<Metrics>,
    sent_metrics: Option<Metrics>,
}

#[derive(Debug, PartialEq)]
pub enum FrameInput {
    Pointer(PointerEvent),
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
            active: HashSet::new(),
            converter: PointerConverter::default(),
            paused: false,
            pending_metrics: None,
            sent_metrics: None,
        }
    }

    /// Returns whether the platform should arm the next vsync.
    pub fn push_pointer(&mut self, event: PointerEvent) -> bool {
        for event in self.converter.accept(event) {
            match event.phase {
                Phase::Down => {
                    self.active.insert(event.pointer);
                }
                Phase::Up | Phase::Cancel => {
                    self.active.remove(&event.pointer);
                }
                Phase::Move | Phase::Wheel | Phase::Zoom => {}
            }
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
            if matches!(event, FrameInput::Pointer(_)) {
                frozen.push_back(event);
            } else {
                deferred.push_back(event);
            }
        }
        self.events = deferred;
        frozen.extend(
            self.active
                .drain()
                .map(|id| FrameInput::Pointer(self.converter.cancel_event(id))),
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
        self.converter = PointerConverter::default();
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
            FrameInput::Pointer(event) => Some(event),
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
                Phase::Down,
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
    fn an_up_without_a_move_is_preceded_by_one_and_cancel_keeps_the_sample() {
        let mut input = Input::new();
        let mut down = event(Phase::Down, 4.0);
        down.kind = PointerKind::Touch;
        down.time_ms = 8.0;
        input.push_pointer(down);
        let mut up = event(Phase::Up, 9.0);
        up.kind = PointerKind::Touch;
        up.time_ms = 12.0;
        input.push_pointer(up);
        assert_eq!(phases(&input), vec![Phase::Down, Phase::Move, Phase::Up]);
        assert!(!input.converter.tracked.contains_key(&up.pointer));
        down.x = 9.0;
        down.time_ms = 12.0;
        input.push_pointer(down);
        let mut cancel = event(Phase::Cancel, 0.0);
        cancel.kind = PointerKind::Mouse;
        input.push_pointer(cancel);
        let last = input.events.back().and_then(pointer).expect("cancel");
        assert_eq!(last.phase, Phase::Cancel);
        assert_eq!(last.kind, PointerKind::Touch);
        assert_eq!((last.x, last.y), (9.0, 0.0));
        assert_eq!(last.time_ms, 12.0);
    }

    #[test]
    fn a_leave_is_preceded_by_the_last_in_view_move() {
        let mut input = Input::new();
        let mut hover = event(Phase::Move, 6.0);
        hover.button = Button::None;
        hover.buttons = Buttons::empty();
        input.push_pointer(hover);
        let mut leave = event(Phase::Move, -1.0);
        leave.button = Button::None;
        leave.buttons = Buttons::empty();
        input.push_pointer(leave);
        assert_eq!(phases(&input), vec![Phase::Move, Phase::Move, Phase::Move]);
        let xs: Vec<f32> = input
            .events
            .iter()
            .filter_map(pointer)
            .map(|event| event.x)
            .collect();
        assert_eq!(xs, vec![6.0, 6.0, -1.0]);
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
