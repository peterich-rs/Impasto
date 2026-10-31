//! Input connection edits. The Java `TextInput` reports each `InputConnection`
//! operation. [`Client`] applies it to a UTF-16 buffer and returns the
//! [`Edit`]s the session already understands. Indices are UTF-16 code units,
//! matching Java strings.

/// `AKEYCODE_*` values used by the host. Kept here so the mapping tests do not
/// need the Android NDK.
const KEY_BACK: u32 = 4;
const KEY_DPAD_UP: u32 = 19;
const KEY_DPAD_DOWN: u32 = 20;
const KEY_DPAD_LEFT: u32 = 21;
const KEY_DPAD_RIGHT: u32 = 22;
const KEY_TAB: u32 = 61;
const KEY_ESCAPE: u32 = 111;
const KEY_FORWARD_DEL: u32 = 112;
const KEY_MOVE_HOME: u32 = 122;
const KEY_MOVE_END: u32 = 123;
const KEY_ENTER: u32 = 66;
const KEY_DEL: u32 = 67;
const KEY_NUMPAD_ENTER: u32 = 160;
pub const KEY_CUT: u32 = 277;
pub const KEY_COPY: u32 = 278;
pub const KEY_PASTE: u32 = 279;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub text: String,
    /// Inclusive start, exclusive end, in UTF-16 code units.
    pub compose: Option<(usize, usize)>,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            compose: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Backspace,
    Enter,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Delete,
    Home,
    End,
    WordLeft,
    WordRight,
    DocumentStart,
    DocumentEnd,
    DeleteWord,
    DeleteWordForward,
    Escape,
    /// Select the whole buffer so the following [`Edit::Text`] replaces it.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "production callers are compiled only for Android")
    )]
    SelectAll,
    Text(String),
    /// Delete a UTF-16 range, then [`Edit::Insert`] the replacement.
    DeleteRange {
        start: u32,
        end: u32,
    },
    Insert(String),
    Composing(Option<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    Edit {
        edit: Edit,
        shift: bool,
    },
    /// The system back key while the keyboard is visible.
    Dismiss,
    /// The system back key while the keyboard is hidden.
    Back,
}

/// `action_down` is `KeyAction::Down`. `Up` is ignored so a key is not applied twice.
/// While the IME is visible, delete and character keys arrive as buffer changes.
///
/// `shift` extends the selection. `ctrl` turns arrows into word or document motions.
pub fn map_key(
    action_down: bool,
    keycode: u32,
    ime_visible: bool,
    shift: bool,
    ctrl: bool,
) -> Option<KeyOutcome> {
    if !action_down {
        return None;
    }
    if keycode == KEY_BACK {
        return Some(if ime_visible {
            KeyOutcome::Dismiss
        } else {
            KeyOutcome::Back
        });
    }
    if ime_visible && ime_delete(keycode).is_some() {
        return None;
    }
    let edit = match keycode {
        KEY_DEL if ctrl => Edit::DeleteWord,
        KEY_DEL => Edit::Backspace,
        KEY_FORWARD_DEL if ctrl => Edit::DeleteWordForward,
        KEY_FORWARD_DEL => Edit::Delete,
        KEY_ENTER | KEY_NUMPAD_ENTER => Edit::Enter,
        KEY_TAB => Edit::Tab,
        KEY_ESCAPE => Edit::Escape,
        KEY_DPAD_LEFT if ctrl => Edit::WordLeft,
        KEY_DPAD_LEFT => Edit::Left,
        KEY_DPAD_RIGHT if ctrl => Edit::WordRight,
        KEY_DPAD_RIGHT => Edit::Right,
        KEY_DPAD_UP => Edit::Up,
        KEY_DPAD_DOWN => Edit::Down,
        KEY_MOVE_HOME if ctrl => Edit::DocumentStart,
        KEY_MOVE_HOME => Edit::Home,
        KEY_MOVE_END if ctrl => Edit::DocumentEnd,
        KEY_MOVE_END => Edit::End,
        _ => return None,
    };
    Some(KeyOutcome::Edit { edit, shift })
}

/// Delete key the IME may deliver in addition to a buffer change.
/// The host posts it only when that batch did not already change the buffer.
pub fn ime_delete(keycode: u32) -> Option<Edit> {
    match keycode {
        KEY_DEL => Some(Edit::Backspace),
        KEY_FORWARD_DEL => Some(Edit::Delete),
        _ => None,
    }
}

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Buffer change since `previous`. Composition is reported before committed edits
/// so a confirm clears the marked text and then appends the committed character.
pub fn diff(previous: &Snapshot, next: &Snapshot) -> Vec<Edit> {
    let (old_committed, old_marked) = parts(previous);
    let (new_committed, new_marked) = parts(next);
    let mut events = Vec::new();
    if old_marked != new_marked {
        events.push(Edit::Composing(new_marked));
    }
    events.extend(committed_edits(&old_committed, &new_committed));
    events
}

fn parts(snapshot: &Snapshot) -> (String, Option<String>) {
    let Some((start, end)) = snapshot.compose else {
        return (snapshot.text.clone(), None);
    };
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    if start == end {
        return (snapshot.text.clone(), None);
    }
    let start_byte = utf16_to_byte(&snapshot.text, start);
    let end_byte = utf16_to_byte(&snapshot.text, end);
    if start_byte >= end_byte
        || !snapshot.text.is_char_boundary(start_byte)
        || !snapshot.text.is_char_boundary(end_byte)
    {
        return (snapshot.text.clone(), None);
    }
    let marked = snapshot.text[start_byte..end_byte].to_owned();
    let mut committed = String::with_capacity(snapshot.text.len() - marked.len());
    committed.push_str(&snapshot.text[..start_byte]);
    committed.push_str(&snapshot.text[end_byte..]);
    (committed, Some(marked))
}

fn committed_edits(old: &str, new: &str) -> Vec<Edit> {
    if old == new {
        return Vec::new();
    }
    if let Some(suffix) = new.strip_prefix(old)
        && !suffix.is_empty()
    {
        return vec![Edit::Text(suffix.to_owned())];
    }
    let prefix = common_prefix(old, new);
    let mut edits = Vec::new();
    if prefix < old.len() {
        let start = u32::try_from(utf16_len(&old[..prefix])).unwrap_or(u32::MAX);
        let end = u32::try_from(utf16_len(old)).unwrap_or(u32::MAX);
        edits.push(Edit::DeleteRange { start, end });
    }
    if prefix < new.len() {
        edits.push(Edit::Insert(new[prefix..].to_owned()));
    }
    edits
}

fn common_prefix(left: &str, right: &str) -> usize {
    let mut bytes = 0;
    for (a, b) in left.chars().zip(right.chars()) {
        if a != b {
            break;
        }
        bytes += a.len_utf8();
    }
    bytes
}

/// One field's buffer, as the input connection last saw it.
///
/// `selection` and [`Buffer::compose`] are UTF-16 code units. The composing
/// range lies inside `text`. A batch nests `begin` / `end` and emits one diff.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Buffer {
    text: String,
    selection: std::ops::Range<usize>,
    compose: Option<(usize, usize)>,
}

impl Buffer {
    fn empty() -> Self {
        Self {
            text: String::new(),
            selection: 0..0,
            compose: None,
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            compose: self.compose.filter(|(start, end)| start < end),
        }
    }
}

pub struct Client {
    buffer: Buffer,
    depth: u32,
    dirty: bool,
    base: Snapshot,
}

impl Client {
    pub fn new() -> Self {
        Self {
            buffer: Buffer::empty(),
            depth: 0,
            dirty: false,
            base: Snapshot::empty(),
        }
    }

    /// Replace the mirror with the session's editing state. Does not emit edits.
    pub fn apply_state(
        &mut self,
        text: &str,
        selection: std::ops::Range<u32>,
        compose: Option<std::ops::Range<u32>>,
    ) {
        let len = utf16_len(text);
        let start = (selection.start as usize).min(len);
        let end = (selection.end as usize).min(len).max(start);
        let compose = compose.and_then(|range| {
            let start = range.start as usize;
            let end = range.end as usize;
            (start < end && end <= len).then_some((start, end))
        });
        self.buffer = Buffer {
            text: text.to_owned(),
            selection: start..end,
            compose,
        };
        self.depth = 0;
        self.dirty = false;
        self.base = self.buffer.snapshot();
    }

    pub fn text(&self) -> String {
        self.buffer.text.clone()
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "production callers are compiled only for Android")
    )]
    pub fn begin(&mut self) {
        if self.depth == 0 {
            self.base = self.buffer.snapshot();
            self.dirty = false;
        }
        self.depth += 1;
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "production callers are compiled only for Android")
    )]
    pub fn end(&mut self) -> Vec<Edit> {
        if self.depth == 0 {
            return Vec::new();
        }
        self.depth -= 1;
        if self.depth == 0 && self.dirty {
            self.dirty = false;
            return diff(&self.base, &self.buffer.snapshot());
        }
        Vec::new()
    }

    /// `commitText`. Replaces the composing range, or the selection.
    pub fn commit(&mut self, text: &str) -> Vec<Edit> {
        self.mutate(|buffer| {
            let (start, end) = buffer
                .compose
                .unwrap_or((buffer.selection.start, buffer.selection.end));
            splice(buffer, start, end, text, false);
        })
    }

    /// `setComposingText`. An empty string clears the composition.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "production callers are compiled only for Android")
    )]
    pub fn composing(&mut self, text: &str) -> Vec<Edit> {
        self.mutate(|buffer| {
            let (start, end) = buffer
                .compose
                .unwrap_or((buffer.selection.start, buffer.selection.end));
            splice(buffer, start, end, text, !text.is_empty());
        })
    }

    /// `finishComposingText`. The marked characters stay and become committed.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "production callers are compiled only for Android")
    )]
    pub fn finish(&mut self) -> Vec<Edit> {
        if self.buffer.compose.is_none() {
            return Vec::new();
        }
        self.mutate(|buffer| {
            buffer.compose = None;
        })
    }

    /// `deleteSurroundingText`. Lengths are UTF-16 units outside the selection.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "production callers are compiled only for Android")
    )]
    pub fn delete(&mut self, before: usize, after: usize) -> Vec<Edit> {
        if before == 0 && after == 0 {
            return Vec::new();
        }
        self.mutate(|buffer| delete_surrounding(buffer, before, after))
    }

    fn mutate(&mut self, edit: impl FnOnce(&mut Buffer)) -> Vec<Edit> {
        let base = self.buffer.snapshot();
        edit(&mut self.buffer);
        if self.depth > 0 {
            self.dirty = true;
            return Vec::new();
        }
        diff(&base, &self.buffer.snapshot())
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

fn splice(buffer: &mut Buffer, start: usize, end: usize, insert: &str, composing: bool) {
    let mut units: Vec<u16> = buffer.text.encode_utf16().collect();
    let start = start.min(units.len());
    let end = end.min(units.len()).max(start);
    let extra: Vec<u16> = insert.encode_utf16().collect();
    let extra_len = extra.len();
    units.splice(start..end, extra);
    buffer.text = String::from_utf16_lossy(&units);
    let caret = start + extra_len;
    buffer.selection = caret..caret;
    buffer.compose = (composing && extra_len > 0).then_some((start, caret));
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "production callers are compiled only for Android")
)]
fn delete_surrounding(buffer: &mut Buffer, before: usize, after: usize) {
    let mut units: Vec<u16> = buffer.text.encode_utf16().collect();
    let sel_lo = buffer
        .selection
        .start
        .min(buffer.selection.end)
        .min(units.len());
    let sel_hi = buffer
        .selection
        .start
        .max(buffer.selection.end)
        .min(units.len());
    let from = sel_lo.saturating_sub(before);
    let to = (sel_hi + after).min(units.len());
    let compose = buffer.compose;
    if to > sel_hi {
        units.drain(sel_hi..to);
    }
    if from < sel_lo {
        units.drain(from..sel_lo);
    }
    buffer.text = String::from_utf16_lossy(&units);
    let removed_before = sel_lo - from;
    let span = sel_hi - sel_lo;
    buffer.selection = from..(from + span);
    buffer.compose = compose.and_then(|(start, end)| {
        let start = shift_index(start, from, sel_lo, sel_hi, to, removed_before);
        let end = shift_index(end, from, sel_lo, sel_hi, to, removed_before);
        (start < end && end <= units.len()).then_some((start, end))
    });
}

/// Map a UTF-16 index across a deletion of `[from, sel_lo)` and `[sel_hi, to)`.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "production callers are compiled only for Android")
)]
fn shift_index(
    index: usize,
    from: usize,
    sel_lo: usize,
    sel_hi: usize,
    to: usize,
    removed_before: usize,
) -> usize {
    let removed_after = to - sel_hi;
    if index <= from {
        index
    } else if index <= sel_lo {
        from
    } else if index <= sel_hi {
        index - removed_before
    } else if index <= to {
        sel_hi - removed_before
    } else {
        index - removed_before - removed_after
    }
}

fn utf16_to_byte(text: &str, utf16_index: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= utf16_index {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(text: &str, compose: Option<(usize, usize)>) -> Snapshot {
        Snapshot {
            text: text.to_owned(),
            compose,
        }
    }

    #[test]
    fn first_character_is_text() {
        let events = diff(&Snapshot::empty(), &marked("a", None));
        assert_eq!(events, vec![Edit::Text("a".to_string())]);
    }

    #[test]
    fn composition_starts_confirms_and_backspaces() {
        let empty = Snapshot::empty();
        let composing = marked("ni", Some((0, 2)));
        assert_eq!(
            diff(&empty, &composing),
            vec![Edit::Composing(Some("ni".to_string()))]
        );
        let shorter = marked("n", Some((0, 1)));
        assert_eq!(
            diff(&composing, &shorter),
            vec![Edit::Composing(Some("n".to_string()))]
        );
        let confirmed = marked("你", None);
        assert_eq!(
            diff(&composing, &confirmed),
            vec![Edit::Composing(None), Edit::Text("你".to_string())]
        );
    }

    #[test]
    fn committed_backspace_deletes_the_last_unit() {
        let events = diff(&marked("ab", None), &marked("a", None));
        assert_eq!(events, vec![Edit::DeleteRange { start: 1, end: 2 }]);
    }

    #[test]
    fn deletion_in_the_middle_deletes_that_range_then_inserts() {
        let events = diff(&marked("hello", None), &marked("hllo", None));
        assert_eq!(
            events,
            vec![
                Edit::DeleteRange { start: 1, end: 5 },
                Edit::Insert("llo".to_string())
            ]
        );
    }

    #[test]
    fn cjk_compose_range_is_utf16_not_bytes() {
        let previous = marked("你", None);
        let next = marked("你好", Some((1, 2)));
        assert_eq!(
            diff(&previous, &next),
            vec![Edit::Composing(Some("好".to_string()))]
        );
    }

    #[test]
    fn commit_replaces_a_composition_and_finish_keeps_it() {
        let mut client = Client::new();
        assert_eq!(
            client.composing("ni"),
            vec![Edit::Composing(Some("ni".to_string()))]
        );
        assert_eq!(
            client.commit("你"),
            vec![Edit::Composing(None), Edit::Text("你".to_string())]
        );
        client.composing("ni");
        assert_eq!(
            client.finish(),
            vec![Edit::Composing(None), Edit::Text("ni".to_string())]
        );
    }

    #[test]
    fn backspace_shortens_the_composition() {
        let mut client = Client::new();
        client.composing("ni");
        assert_eq!(
            client.delete(1, 0),
            vec![Edit::Composing(Some("n".to_string()))]
        );
        assert_eq!(client.delete(1, 0), vec![Edit::Composing(None)]);
    }

    #[test]
    fn a_batch_emits_one_diff() {
        let mut client = Client::new();
        client.begin();
        assert!(client.composing("n").is_empty());
        assert!(client.composing("ni").is_empty());
        assert_eq!(client.end(), vec![Edit::Composing(Some("ni".to_string()))]);
        assert!(client.end().is_empty());
    }

    #[test]
    fn key_up_is_ignored_and_back_dismisses_only_while_the_keyboard_is_up() {
        assert_eq!(map_key(false, KEY_DEL, false, false, false), None);
        assert_eq!(
            map_key(true, KEY_DEL, false, false, false),
            Some(KeyOutcome::Edit {
                edit: Edit::Backspace,
                shift: false,
            })
        );
        assert_eq!(map_key(true, KEY_DEL, true, false, false), None);
        assert_eq!(
            map_key(true, KEY_BACK, true, false, false),
            Some(KeyOutcome::Dismiss)
        );
        assert_eq!(
            map_key(true, KEY_BACK, false, false, false),
            Some(KeyOutcome::Back)
        );
        assert_eq!(
            map_key(true, KEY_ENTER, true, false, false),
            Some(KeyOutcome::Edit {
                edit: Edit::Enter,
                shift: false,
            })
        );
        assert_eq!(
            map_key(true, KEY_DPAD_LEFT, false, true, true),
            Some(KeyOutcome::Edit {
                edit: Edit::WordLeft,
                shift: true,
            })
        );
    }
}
