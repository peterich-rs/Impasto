//! Editing value for one field. Offsets are UTF-16 code units.
//!
//! Grapheme boundaries come from `unicode-segmentation`. A family emoji or a
//! combining accent moves and deletes as one cluster. The editor does not talk
//! to the platform keyboard; the session copies [`TextEditingValue`] into
//! `ImeState`.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::{Affinity, Caret, TextLine};

const UNDO_LIMIT: usize = 100;

/// A selection. `base` stays put while `extent` moves.
///
/// `affinity` matters only while the selection is collapsed on a soft line break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSelection {
    pub base: u32,
    pub extent: u32,
    pub affinity: Affinity,
}

impl TextSelection {
    pub fn collapsed(unit: u32) -> Self {
        Self {
            base: unit,
            extent: unit,
            affinity: Affinity::Downstream,
        }
    }

    pub fn is_collapsed(self) -> bool {
        self.base == self.extent
    }

    /// Inclusive start and exclusive end, in order.
    pub fn ordered(self) -> (u32, u32) {
        if self.base <= self.extent {
            (self.base, self.extent)
        } else {
            (self.extent, self.base)
        }
    }
}

/// One field's text, caret or selection, and the IME composing range.
///
/// `selection` and `composing` count UTF-16 code units into `text`. `text`
/// includes the composing characters while a composition is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEditingValue {
    pub text: String,
    pub selection: TextSelection,
    pub composing: Option<Range<u32>>,
}

impl TextEditingValue {
    pub fn at_end(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = utf16_len(&text);
        Self {
            text,
            selection: TextSelection::collapsed(end),
            composing: None,
        }
    }
}

/// What a key, the IME, or the clipboard asked the editor to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Left,
    Right,
    ExtendLeft,
    ExtendRight,
    Backspace,
    /// Delete the cluster after the caret.
    ForwardDelete,
    Insert(String),
    /// Insert a hard line break.
    Newline,
    /// `None` commits the open composition. `Some` replaces it.
    Composing(Option<String>),
    Paste(String),
    Copy,
    Cut,
    Undo,
    Redo,
    SelectAll,
    /// Replace the value. Used when a control binds a new string.
    Load(String),
    /// Delete the composing range, then insert `String`. Does not clear composing
    /// by the insert path, so a following insert cannot delete the same range again.
    Commit(String),
    /// Move the caret to a UTF-16 offset, clamped to the text.
    SetCaret(u32),
    /// Keep `base` and move `extent` to a UTF-16 offset.
    ExtendCaret(u32),
    WordLeft,
    WordRight,
    ExtendWordLeft,
    ExtendWordRight,
    LineStart,
    LineEnd,
    ExtendLineStart,
    ExtendLineEnd,
    DocumentStart,
    DocumentEnd,
    ExtendDocumentStart,
    ExtendDocumentEnd,
    Up,
    Down,
    ExtendUp,
    ExtendDown,
    DeleteWordBackward,
    DeleteWordForward,
    DeleteToLineStart,
    DeleteToLineEnd,
    /// Delete a UTF-16 range. The tree uses this after it resolves a word.
    DeleteRange {
        start: u32,
        end: u32,
    },
}

/// Grapheme-aware edits and a fixed-depth undo stack.
#[derive(Clone, Debug)]
pub struct TextEditor {
    value: TextEditingValue,
    undo: Vec<TextEditingValue>,
    redo: Vec<TextEditingValue>,
    /// Text copied by the last `Copy` or `Cut`. The host writes the pasteboard.
    clipboard: Option<String>,
    /// Horizontal anchor for a run of vertical motions.
    column_x: Option<f32>,
    revision: u64,
}

impl TextEditor {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            value: TextEditingValue::at_end(text),
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: None,
            column_x: None,
            revision: 0,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replace the string and clamp the selection and composing range into it.
    ///
    /// A composing range that collapses stays `Some`. The undo stack is left
    /// alone. `revision` increments only when the text changes.
    pub fn replace_keeping_composition(&mut self, next: String) -> bool {
        if self.value.text == next {
            return false;
        }
        let limit = utf16_len(&next);
        self.value.text = next;
        self.value.selection.base = self.value.selection.base.min(limit);
        self.value.selection.extent = self.value.selection.extent.min(limit);
        if let Some(range) = self.value.composing.as_mut() {
            let start = range.start.min(limit);
            let end = range.end.min(limit);
            *range = start..end;
        }
        self.revision = self.revision.saturating_add(1);
        true
    }

    pub fn value(&self) -> &TextEditingValue {
        &self.value
    }

    pub fn clipboard(&self) -> Option<&str> {
        self.clipboard.as_deref()
    }

    pub fn column_x(&self) -> Option<f32> {
        self.column_x
    }

    pub fn set_column_x(&mut self, x: Option<f32>) {
        self.column_x = x;
    }

    pub fn apply(&mut self, edit: Edit) {
        let lines: &[TextLine] = &[];
        self.apply_in(
            edit,
            lines,
            crate::shaper::whitespace_word,
            |_, _| empty_caret(),
            |_, _| empty_caret(),
        );
    }

    /// `lines` is the paragraph the text currently occupies. An empty slice
    /// falls back to grapheme steps, which is what a single unwrapped line does.
    pub fn apply_in<W, C, H>(
        &mut self,
        edit: Edit,
        lines: &[TextLine],
        word_at: W,
        caret_at: C,
        hit_at: H,
    ) where
        W: Fn(&str, u32) -> (u32, u32),
        C: Fn(u32, Affinity) -> Caret,
        H: Fn(f32, f32) -> Caret,
    {
        match edit {
            Edit::Left => self.move_cluster(lines, -1, false),
            Edit::Right => self.move_cluster(lines, 1, false),
            Edit::ExtendLeft => self.move_cluster(lines, -1, true),
            Edit::ExtendRight => self.move_cluster(lines, 1, true),
            Edit::Backspace => {
                self.checkpoint();
                self.delete_backward();
            }
            Edit::ForwardDelete => {
                self.checkpoint();
                self.delete_forward();
            }
            Edit::Insert(text) => {
                self.commit_composing();
                self.checkpoint();
                self.insert(&text);
            }
            Edit::Newline => {
                self.commit_composing();
                self.checkpoint();
                self.insert("\n");
            }
            Edit::Paste(text) => {
                self.commit_composing();
                self.checkpoint();
                self.insert(&text);
            }
            Edit::Composing(next) => self.set_composing(next),
            Edit::Copy => self.copy(),
            Edit::Cut => {
                self.copy();
                self.checkpoint();
                self.delete_selection();
            }
            Edit::Undo => self.undo(),
            Edit::Redo => self.redo(),
            Edit::SelectAll => {
                let end = utf16_len(&self.value.text);
                self.value.selection = TextSelection {
                    base: 0,
                    extent: end,
                    affinity: Affinity::Downstream,
                };
                self.value.composing = None;
                self.column_x = None;
            }
            Edit::Load(text) => {
                self.undo.clear();
                self.redo.clear();
                self.value = TextEditingValue::at_end(text);
                self.clipboard = None;
                self.column_x = None;
            }
            Edit::SetCaret(unit) => self.place(unit, Affinity::Downstream, false),
            Edit::ExtendCaret(unit) => self.place(unit, Affinity::Downstream, true),
            Edit::WordLeft => self.move_word(&word_at, -1, false),
            Edit::WordRight => self.move_word(&word_at, 1, false),
            Edit::ExtendWordLeft => self.move_word(&word_at, -1, true),
            Edit::ExtendWordRight => self.move_word(&word_at, 1, true),
            Edit::LineStart => self.move_line_edge(lines, -1, false),
            Edit::LineEnd => self.move_line_edge(lines, 1, false),
            Edit::ExtendLineStart => self.move_line_edge(lines, -1, true),
            Edit::ExtendLineEnd => self.move_line_edge(lines, 1, true),
            Edit::DocumentStart => self.place(0, Affinity::Downstream, false),
            Edit::DocumentEnd => {
                let end = utf16_len(&self.value.text);
                self.place(end, Affinity::Downstream, false);
            }
            Edit::ExtendDocumentStart => self.place(0, Affinity::Downstream, true),
            Edit::ExtendDocumentEnd => {
                let end = utf16_len(&self.value.text);
                self.place(end, Affinity::Downstream, true);
            }
            Edit::Up => self.move_vertical(lines, -1, false, &caret_at, &hit_at),
            Edit::Down => self.move_vertical(lines, 1, false, &caret_at, &hit_at),
            Edit::ExtendUp => self.move_vertical(lines, -1, true, &caret_at, &hit_at),
            Edit::ExtendDown => self.move_vertical(lines, 1, true, &caret_at, &hit_at),
            Edit::DeleteWordBackward => {
                self.checkpoint();
                self.delete_word(&word_at, -1);
            }
            Edit::DeleteWordForward => {
                self.checkpoint();
                self.delete_word(&word_at, 1);
            }
            Edit::DeleteToLineStart => {
                self.checkpoint();
                self.delete_to_line_edge(lines, -1);
            }
            Edit::DeleteToLineEnd => {
                self.checkpoint();
                self.delete_to_line_edge(lines, 1);
            }
            Edit::DeleteRange { start, end } => {
                self.checkpoint();
                self.value.selection = TextSelection {
                    base: start,
                    extent: end,
                    affinity: Affinity::Downstream,
                };
                self.delete_selection();
            }
            Edit::Commit(text) => {
                self.checkpoint();
                self.delete_composing_text();
                self.insert(&text);
            }
        }
        self.revision = self.revision.saturating_add(1);
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.value.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn step_history(
        value: &mut TextEditingValue,
        column_x: &mut Option<f32>,
        from: &mut Vec<TextEditingValue>,
        to: &mut Vec<TextEditingValue>,
    ) {
        let Some(previous) = from.pop() else {
            return;
        };
        to.push(std::mem::replace(value, previous));
        *column_x = None;
    }

    fn undo(&mut self) {
        Self::step_history(
            &mut self.value,
            &mut self.column_x,
            &mut self.undo,
            &mut self.redo,
        );
    }

    fn redo(&mut self) {
        Self::step_history(
            &mut self.value,
            &mut self.column_x,
            &mut self.redo,
            &mut self.undo,
        );
    }

    fn copy(&mut self) {
        let (start, end) = self.value.selection.ordered();
        if start == end {
            self.clipboard = None;
            return;
        }
        self.clipboard = Some(slice_utf16(&self.value.text, start, end));
    }

    fn place(&mut self, unit: u32, affinity: Affinity, extend: bool) {
        self.commit_composing();
        let end = utf16_len(&self.value.text);
        let unit = unit.min(end);
        if extend {
            self.value.selection.extent = unit;
            self.value.selection.affinity = affinity;
        } else {
            self.value.selection = TextSelection {
                base: unit,
                extent: unit,
                affinity,
            };
        }
        self.column_x = None;
    }

    fn move_cluster(&mut self, lines: &[TextLine], delta: i32, extend: bool) {
        self.commit_composing();
        let selection = self.value.selection;
        if !extend && !selection.is_collapsed() {
            let (start, end) = selection.ordered();
            let edge = if delta < 0 { start } else { end };
            self.place(edge, Affinity::Downstream, false);
            return;
        }
        let unit = selection.extent;
        let affinity = selection.affinity;
        if delta < 0 && affinity == Affinity::Downstream && soft_line_start(lines, unit) {
            self.place(unit, Affinity::Upstream, extend);
            return;
        }
        if delta > 0 && affinity == Affinity::Upstream && soft_line_start(lines, unit) {
            self.place(unit, Affinity::Downstream, extend);
            return;
        }
        let bounds = grapheme_bounds(&self.value.text);
        let next = step_bound(&bounds, unit, delta);
        self.place(next, Affinity::Downstream, extend);
    }

    fn move_word<W>(&mut self, word_at: &W, delta: i32, extend: bool)
    where
        W: Fn(&str, u32) -> (u32, u32),
    {
        self.commit_composing();
        let unit = self.value.selection.extent;
        let end = utf16_len(&self.value.text);
        let (start, stop) = word_at(&self.value.text, unit.min(end));
        let next = if delta < 0 {
            if unit > start {
                start
            } else if unit == 0 {
                0
            } else {
                word_at(&self.value.text, unit - 1).0
            }
        } else if unit < stop {
            stop
        } else if unit >= end {
            end
        } else {
            word_at(&self.value.text, unit + 1).1
        };
        self.place(next, Affinity::Downstream, extend);
    }

    fn move_line_edge(&mut self, lines: &[TextLine], delta: i32, extend: bool) {
        self.commit_composing();
        let unit = self.value.selection.extent;
        let affinity = self.value.selection.affinity;
        let Some(line) = line_for(lines, unit, affinity) else {
            let edge = if delta < 0 {
                0
            } else {
                utf16_len(&self.value.text)
            };
            self.place(edge, Affinity::Downstream, extend);
            return;
        };
        if delta < 0 {
            self.place(line.start, Affinity::Downstream, extend);
        } else {
            self.place(line.end, Affinity::Upstream, extend);
        }
    }

    fn move_vertical<C, H>(
        &mut self,
        lines: &[TextLine],
        delta: i32,
        extend: bool,
        caret_at: &C,
        hit_at: &H,
    ) where
        C: Fn(u32, Affinity) -> Caret,
        H: Fn(f32, f32) -> Caret,
    {
        if lines.is_empty() {
            return;
        }
        self.commit_composing();
        let unit = self.value.selection.extent;
        let affinity = self.value.selection.affinity;
        let caret = caret_at(unit, affinity);
        let x = self.column_x.unwrap_or(caret.x);
        let index = line_index(lines, unit, affinity);
        let next = (index as i32 + delta).clamp(0, lines.len() as i32 - 1) as usize;
        let line = &lines[next];
        let hit = hit_at(x, line.y + line.h * 0.5);
        self.place(hit.unit, hit.affinity, extend);
        self.column_x = Some(x);
    }

    fn delete_backward(&mut self) {
        self.commit_composing();
        if !self.value.selection.is_collapsed() {
            self.delete_selection();
            return;
        }
        let bounds = grapheme_bounds(&self.value.text);
        let caret = self.value.selection.extent;
        let previous = step_bound(&bounds, caret, -1);
        if previous == caret {
            return;
        }
        self.value.text = delete_utf16(&self.value.text, previous, caret);
        self.value.selection = TextSelection::collapsed(previous);
    }

    fn delete_forward(&mut self) {
        self.commit_composing();
        if !self.value.selection.is_collapsed() {
            self.delete_selection();
            return;
        }
        let bounds = grapheme_bounds(&self.value.text);
        let caret = self.value.selection.extent;
        let next = step_bound(&bounds, caret, 1);
        if next == caret {
            return;
        }
        self.value.text = delete_utf16(&self.value.text, caret, next);
        self.value.selection = TextSelection::collapsed(caret);
    }

    fn delete_word<W>(&mut self, word_at: &W, delta: i32)
    where
        W: Fn(&str, u32) -> (u32, u32),
    {
        self.commit_composing();
        if !self.value.selection.is_collapsed() {
            self.delete_selection();
            return;
        }
        let unit = self.value.selection.extent;
        let end = utf16_len(&self.value.text);
        let (start, stop) = word_at(&self.value.text, unit.min(end));
        let (from, to) = if delta < 0 {
            if unit > start {
                (start, unit)
            } else if unit == 0 {
                return;
            } else {
                (word_at(&self.value.text, unit - 1).0, unit)
            }
        } else if unit < stop {
            (unit, stop)
        } else if unit >= end {
            return;
        } else {
            (unit, word_at(&self.value.text, unit + 1).1)
        };
        if from == to {
            return;
        }
        self.value.text = delete_utf16(&self.value.text, from, to);
        self.value.selection = TextSelection::collapsed(from);
    }

    fn delete_to_line_edge(&mut self, lines: &[TextLine], delta: i32) {
        self.commit_composing();
        if !self.value.selection.is_collapsed() {
            self.delete_selection();
            return;
        }
        let unit = self.value.selection.extent;
        let affinity = self.value.selection.affinity;
        let Some(line) = line_for(lines, unit, affinity) else {
            return;
        };
        let (from, to) = if delta < 0 {
            (line.start, unit)
        } else {
            (unit, line.end)
        };
        if from == to {
            return;
        }
        self.value.text = delete_utf16(&self.value.text, from, to);
        self.value.selection = TextSelection::collapsed(from);
    }

    fn delete_selection(&mut self) {
        let (start, end) = self.value.selection.ordered();
        if start == end {
            return;
        }
        self.value.text = delete_utf16(&self.value.text, start, end);
        self.value.selection = TextSelection::collapsed(start);
        self.value.composing = None;
    }

    fn insert(&mut self, extra: &str) {
        self.delete_selection();
        let at = self.value.selection.extent;
        self.value.text = insert_utf16(&self.value.text, at, extra);
        let end = at.saturating_add(utf16_len(extra));
        self.value.selection = TextSelection::collapsed(end);
        self.value.composing = None;
        self.column_x = None;
    }

    fn commit_composing(&mut self) {
        self.value.composing = None;
    }

    /// Remove the marked characters. The composing flag is cleared with them.
    fn delete_composing_text(&mut self) {
        let Some(range) = self.value.composing.take() else {
            return;
        };
        let (start, end) = if range.start <= range.end {
            (range.start, range.end)
        } else {
            (range.end, range.start)
        };
        if start == end {
            return;
        }
        self.value.text = delete_utf16(&self.value.text, start, end);
        self.value.selection = TextSelection::collapsed(start);
    }

    fn set_composing(&mut self, next: Option<String>) {
        if self.value.composing.is_none() && next.is_some() {
            self.checkpoint();
            if !self.value.selection.is_collapsed() {
                self.delete_selection();
            }
        }
        if let Some(range) = self.value.composing.take() {
            let (start, end) = if range.start <= range.end {
                (range.start, range.end)
            } else {
                (range.end, range.start)
            };
            self.value.text = delete_utf16(&self.value.text, start, end);
            self.value.selection = TextSelection::collapsed(start);
        }
        let Some(extra) = next else {
            return;
        };
        let at = self.value.selection.extent;
        self.value.text = insert_utf16(&self.value.text, at, &extra);
        let end = at.saturating_add(utf16_len(&extra));
        self.value.composing = Some(at..end);
        self.value.selection = TextSelection::collapsed(end);
        self.column_x = None;
    }
}

fn empty_caret() -> Caret {
    Caret {
        unit: 0,
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 0.0,
        affinity: Affinity::Downstream,
    }
}

fn soft_line_start(lines: &[TextLine], unit: u32) -> bool {
    lines
        .iter()
        .enumerate()
        .any(|(index, line)| index > 0 && line.start == unit && !lines[index - 1].hard_break)
}

fn line_index(lines: &[TextLine], unit: u32, affinity: Affinity) -> usize {
    if affinity == Affinity::Upstream
        && let Some(index) = lines.iter().position(|line| line.start == unit)
        && index > 0
        && !lines[index - 1].hard_break
    {
        return index - 1;
    }
    lines
        .iter()
        .position(|line| unit < line.end_with_break)
        .unwrap_or(lines.len().saturating_sub(1))
}

fn line_for(lines: &[TextLine], unit: u32, affinity: Affinity) -> Option<&TextLine> {
    if lines.is_empty() {
        return None;
    }
    Some(&lines[line_index(lines, unit, affinity)])
}

pub fn utf16_len(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}

/// UTF-16 offset of the grapheme cluster that contains `utf16`.
///
/// An offset past the end clamps to the final boundary, which is [`grapheme_count`].
pub fn grapheme_index(text: &str, utf16: u32) -> u32 {
    let bounds = grapheme_bounds(text);
    let mut index = 0u32;
    for (slot, bound) in bounds.iter().enumerate() {
        if *bound <= utf16 {
            index = slot as u32;
        } else {
            break;
        }
    }
    index
}

/// Number of grapheme clusters in `text`.
pub fn grapheme_count(text: &str) -> u32 {
    grapheme_bounds(text).len().saturating_sub(1) as u32
}

/// UTF-16 start of grapheme `index`. Past the end clamps to [`utf16_len`].
pub fn grapheme_unit(text: &str, index: u32) -> u32 {
    let bounds = grapheme_bounds(text);
    let slot = (index as usize).min(bounds.len().saturating_sub(1));
    bounds[slot]
}

fn grapheme_bounds(text: &str) -> Vec<u32> {
    let mut bounds = vec![0];
    let mut unit = 0u32;
    for grapheme in text.graphemes(true) {
        unit = unit.saturating_add(grapheme.encode_utf16().count() as u32);
        bounds.push(unit);
    }
    bounds
}

fn step_bound(bounds: &[u32], unit: u32, delta: i32) -> u32 {
    if bounds.is_empty() {
        return 0;
    }
    let index = bounds
        .iter()
        .position(|bound| *bound >= unit)
        .unwrap_or(bounds.len() - 1);
    let index = if bounds.get(index).copied() != Some(unit) && index > 0 {
        index - 1
    } else {
        index
    };
    let next = if delta < 0 {
        index.saturating_sub(1)
    } else {
        (index + 1).min(bounds.len() - 1)
    };
    bounds[next]
}

fn byte_index(text: &str, unit: u32) -> usize {
    let mut seen = 0u32;
    for (byte, ch) in text.char_indices() {
        if seen >= unit {
            return byte;
        }
        seen = seen.saturating_add(ch.len_utf16() as u32);
    }
    text.len()
}

fn slice_utf16(text: &str, start: u32, end: u32) -> String {
    let from = byte_index(text, start);
    let to = byte_index(text, end);
    text.get(from..to).unwrap_or("").to_owned()
}

fn delete_utf16(text: &str, start: u32, end: u32) -> String {
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let from = byte_index(text, start);
    let to = byte_index(text, end);
    let mut out = String::with_capacity(text.len());
    out.push_str(text.get(..from).unwrap_or(""));
    out.push_str(text.get(to..).unwrap_or(""));
    out
}

fn insert_utf16(text: &str, at: u32, extra: &str) -> String {
    let byte = byte_index(text, at);
    let mut out = String::with_capacity(text.len() + extra.len());
    out.push_str(text.get(..byte).unwrap_or(""));
    out.push_str(extra);
    out.push_str(text.get(byte..).unwrap_or(""));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grapheme_index_splits_an_emoji_from_ascii() {
        let text = "😀a";
        assert_eq!(grapheme_count(text), 2);
        assert_eq!(grapheme_index(text, 0), 0);
        assert_eq!(grapheme_index(text, 1), 0);
        assert_eq!(grapheme_index(text, 2), 1);
        assert_eq!(grapheme_unit(text, 1), 2);
    }

    #[test]
    fn emoji_and_combining_marks_move_as_one_cluster() {
        let mut editor = TextEditor::new("a👨‍👩‍👧‍👦e\u{0301}");
        editor.apply(Edit::Left);
        assert_eq!(editor.value().selection.extent, utf16_len("a👨‍👩‍👧‍👦"));
        editor.apply(Edit::Left);
        assert_eq!(editor.value().selection.extent, utf16_len("a"));
        editor.apply(Edit::Left);
        assert_eq!(editor.value().selection, TextSelection::collapsed(0));
        editor.apply(Edit::Right);
        assert_eq!(editor.value().selection.extent, utf16_len("a"));
    }

    #[test]
    fn backspace_removes_one_grapheme() {
        let mut editor = TextEditor::new("a👨‍👩‍👧‍👦");
        editor.apply(Edit::Backspace);
        assert_eq!(editor.value().text, "a");
        editor.apply(Edit::Backspace);
        assert_eq!(editor.value().text, "");
    }

    #[test]
    fn undo_restores_the_deleted_cluster() {
        let mut editor = TextEditor::new("a👨‍👩‍👧‍👦");
        editor.apply(Edit::Backspace);
        editor.apply(Edit::Undo);
        assert_eq!(editor.value().text, "a👨‍👩‍👧‍👦");
        editor.apply(Edit::Redo);
        assert_eq!(editor.value().text, "a");
    }

    #[test]
    fn forward_delete_and_newline_and_extend() {
        let mut editor = TextEditor::new("ab");
        editor.apply(Edit::Left);
        editor.apply(Edit::ForwardDelete);
        assert_eq!(editor.value().text, "a");
        editor.apply(Edit::Newline);
        assert_eq!(editor.value().text, "a\n");
        editor.apply(Edit::DocumentStart);
        editor.apply(Edit::ExtendDocumentEnd);
        assert_eq!(editor.value().selection.base, 0);
        assert_eq!(editor.value().selection.extent, 2);
        editor.apply(Edit::Backspace);
        assert_eq!(editor.value().text, "");
    }

    #[test]
    fn shift_left_extends_and_word_backspace_removes_the_word() {
        let mut editor = TextEditor::new("hello world");
        editor.apply(Edit::ExtendWordLeft);
        assert_eq!(editor.value().selection.extent, utf16_len("hello "));
        editor.apply(Edit::DeleteWordBackward);
        assert_eq!(editor.value().text, "hello ");
    }
}
