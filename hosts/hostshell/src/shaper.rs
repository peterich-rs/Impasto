use std::cell::RefCell;
use std::rc::Rc;

use protocol::TextStyle;
use ui_core::{Shaper, TextSize};

/// Measures with the same typography context the player draws with.
pub struct ImpellerShaper {
    engine: Rc<RefCell<engine::Engine>>,
}

impl ImpellerShaper {
    pub fn new(engine: Rc<RefCell<engine::Engine>>) -> Self {
        Self { engine }
    }
}

impl Shaper for ImpellerShaper {
    fn measure(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> TextSize {
        match self
            .engine
            .borrow_mut()
            .measure_paragraph(text, font_size, max_width, color, style)
        {
            Ok((width, height)) => TextSize { width, height },
            Err(_) => TextSize {
                width: 0.0,
                height: 0.0,
            },
        }
    }
}
