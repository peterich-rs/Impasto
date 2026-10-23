//! In-memory text mirror for `NSTextInputClient`.
//!
//! AppKit asks for the selection and the marked range on the main thread, while
//! the editable string lives in the session. This mirror is updated by the same
//! callbacks that post [`InputEvent`]s, so those queries have a length to return.
//! It does not decide editing policy.

use session::{InputEvent, Key};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marked {
    pub committed: String,
    pub composing: String,
}

impl Marked {
    pub fn has_marked(&self) -> bool {
        !self.composing.is_empty()
    }

    /// Replace the committed string and drop any composition. Used when a field
    /// takes focus and the session sends the string it already has.
    pub fn replace_committed(&mut self, text: String) {
        self.committed = text;
        self.composing.clear();
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
        events
    }

    pub fn mark(&mut self, text: &str) -> Vec<InputEvent> {
        if text.is_empty() {
            self.composing.clear();
            return vec![InputEvent::Composing(None)];
        }
        self.composing = text.to_owned();
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
        vec![InputEvent::Key(Key::Backspace)]
    }

    pub fn utf16_len(text: &str) -> usize {
        text.encode_utf16().count()
    }

    pub fn caret_utf16(&self) -> usize {
        Self::utf16_len(&self.committed) + Self::utf16_len(&self.composing)
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
        assert_eq!(marked.backspace(), vec![InputEvent::Key(Key::Backspace)]);
        assert!(marked.committed.is_empty());
        assert!(marked.backspace().is_empty());
    }
}
