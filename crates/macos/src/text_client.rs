//! In-memory text mirror for `NSTextInputClient`.
//!
//! AppKit asks for the selection and the marked range on the main thread, while
//! the editable string lives in the session. This mirror is updated by the same
//! callbacks that post [`InputEvent`]s, so those queries have a length to return.
//! It does not decide editing policy.

use session::{InputEvent, Key};

#[derive(Clone, Debug, PartialEq)]
pub struct Marked {
    pub committed: String,
    pub composing: String,
    pub selection: std::ops::Range<u32>,
    pub caret: uicore::Rect,
}

impl Default for Marked {
    fn default() -> Self {
        Self {
            committed: String::new(),
            composing: String::new(),
            selection: 0..0,
            caret: uicore::Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        }
    }
}

impl Marked {
    pub fn has_marked(&self) -> bool {
        !self.composing.is_empty()
    }

    pub fn insert(&mut self, text: &str) -> Vec<InputEvent> {
        let mut events = Vec::new();
        if self.has_marked() {
            self.composing.clear();
            events.push(InputEvent::Composing(None));
        }
        if !text.is_empty() {
            self.committed.push_str(text);
            events.push(InputEvent::Text(text.to_owned()));
        }
        self.selection_at_end();
        events
    }

    pub fn mark(&mut self, text: &str) -> Vec<InputEvent> {
        if text.is_empty() {
            self.composing.clear();
            return vec![InputEvent::Composing(None)];
        }
        self.composing = text.to_owned();
        self.selection_at_end();
        vec![InputEvent::Composing(Some(self.composing.clone()))]
    }

    pub fn unmark(&mut self) -> Vec<InputEvent> {
        if self.composing.is_empty() {
            return Vec::new();
        }
        self.composing.clear();
        vec![InputEvent::Composing(None)]
    }

    pub fn backspace(&mut self) -> Vec<InputEvent> {
        if !self.composing.is_empty() {
            pop_char(&mut self.composing);
            return if self.composing.is_empty() {
                vec![InputEvent::Composing(None)]
            } else {
                vec![InputEvent::Composing(Some(self.composing.clone()))]
            };
        }
        if self.committed.is_empty() {
            return Vec::new();
        }
        pop_char(&mut self.committed);
        self.selection_at_end();
        vec![InputEvent::Key(session::KeyPress::new(Key::Backspace))]
    }

    fn selection_at_end(&mut self) {
        let end = (Self::utf16_len(&self.committed) + Self::utf16_len(&self.composing)) as u32;
        self.selection = end..end;
    }

    pub fn utf16_len(text: &str) -> usize {
        text.encode_utf16().count()
    }

    pub fn apply(&mut self, state: &session::ImeState) {
        self.committed = state.text.clone();
        self.selection = state.selection.clone();
        self.caret = state.caret;
        self.composing = match &state.composing {
            Some(range) => utf16_slice(&state.text, range.start, range.end),
            None => String::new(),
        };
    }

    pub fn caret_utf16(&self) -> usize {
        self.selection.end as usize
    }

    /// Marked range in UTF-16 units, or `None` when nothing is composing.
    pub fn marked_utf16(&self) -> Option<(usize, usize)> {
        if self.composing.is_empty() {
            None
        } else {
            Some((
                Self::utf16_len(&self.committed),
                Self::utf16_len(&self.composing),
            ))
        }
    }
}

fn utf16_slice(text: &str, start: u32, end: u32) -> String {
    let units: Vec<u16> = text
        .encode_utf16()
        .skip(start as usize)
        .take(end.saturating_sub(start) as usize)
        .collect();
    String::from_utf16_lossy(&units)
}

fn pop_char(text: &mut String) {
    let Some(ch) = text.chars().next_back() else {
        return;
    };
    let keep = text.len() - ch.len_utf8();
    text.truncate(keep);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_appends_and_a_commit_clears_the_composition() {
        let mut marked = Marked::default();
        assert_eq!(
            marked.mark("ni"),
            vec![InputEvent::Composing(Some("ni".to_string()))]
        );
        assert_eq!(
            marked.insert("你"),
            vec![
                InputEvent::Composing(None),
                InputEvent::Text("你".to_string())
            ]
        );
        assert_eq!(marked.committed, "你");
        assert!(!marked.has_marked());
        assert_eq!(marked.caret_utf16(), 1);
    }

    #[test]
    fn backspace_shortens_composition_before_committed_text() {
        let mut marked = Marked::default();
        marked.insert("你");
        marked.mark("ni");
        assert_eq!(
            marked.backspace(),
            vec![InputEvent::Composing(Some("n".to_string()))]
        );
        marked.unmark();
        assert_eq!(
            marked.backspace(),
            vec![InputEvent::Key(session::KeyPress::new(Key::Backspace))]
        );
        assert!(marked.committed.is_empty());
        assert!(marked.backspace().is_empty());
    }
}
