//! GameActivity gives the whole IME buffer. The session wants committed text,
//! an optional composition, and keys. Indices in [`Snapshot::compose`] are
//! UTF-16 code units, matching Java strings.

/// `AKEYCODE_*` values used by the host. Kept here so the mapping tests do not
/// need the Android NDK.
const KEY_BACK: u32 = 4;
const KEY_DPAD_UP: u32 = 19;
const KEY_DPAD_DOWN: u32 = 20;
const KEY_DPAD_LEFT: u32 = 21;
const KEY_DPAD_RIGHT: u32 = 22;
const KEY_TAB: u32 = 61;
const KEY_ENTER: u32 = 66;
const KEY_DEL: u32 = 67;
const KEY_NUMPAD_ENTER: u32 = 160;

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
    Text(String),
    Composing(Option<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    Edit(Edit),
    /// The system back key while the keyboard is visible.
    Dismiss,
}

/// `action_down` is `KeyAction::Down`. `Up` is ignored so a key is not applied twice.
/// While the IME is visible, delete and character keys arrive as buffer changes.
pub fn map_key(action_down: bool, keycode: u32, ime_visible: bool) -> Option<KeyOutcome> {
    if !action_down {
        return None;
    }
    if keycode == KEY_BACK {
        return ime_visible.then_some(KeyOutcome::Dismiss);
    }
    if ime_visible && keycode == KEY_DEL {
        return None;
    }
    let edit = match keycode {
        KEY_DEL => Edit::Backspace,
        KEY_ENTER | KEY_NUMPAD_ENTER => Edit::Enter,
        KEY_TAB => Edit::Tab,
        KEY_DPAD_LEFT => Edit::Left,
        KEY_DPAD_RIGHT => Edit::Right,
        KEY_DPAD_UP => Edit::Up,
        KEY_DPAD_DOWN => Edit::Down,
        _ => return None,
    };
    Some(KeyOutcome::Edit(edit))
}

pub fn seed(text: &str) -> Snapshot {
    Snapshot {
        text: text.to_owned(),
        compose: None,
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
    if let Some(suffix) = new.strip_prefix(old) {
        return if suffix.is_empty() {
            Vec::new()
        } else {
            vec![Edit::Text(suffix.to_owned())]
        };
    }
    if let Some(removed) = old.strip_prefix(new) {
        return vec![Edit::Backspace; removed.chars().count()];
    }
    let mut events = vec![Edit::Backspace; old.chars().count()];
    if !new.is_empty() {
        events.push(Edit::Text(new.to_owned()));
    }
    events
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
    fn committed_backspace_is_one_key_per_character() {
        let events = diff(&marked("ab", None), &marked("a", None));
        assert_eq!(events, vec![Edit::Backspace]);
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
    fn key_up_is_ignored_and_back_dismisses_only_while_the_keyboard_is_up() {
        assert_eq!(map_key(false, KEY_DEL, false), None);
        assert_eq!(
            map_key(true, KEY_DEL, false),
            Some(KeyOutcome::Edit(Edit::Backspace))
        );
        assert_eq!(map_key(true, KEY_DEL, true), None);
        assert_eq!(map_key(true, KEY_BACK, true), Some(KeyOutcome::Dismiss));
        assert_eq!(map_key(true, KEY_BACK, false), None);
        assert_eq!(
            map_key(true, KEY_ENTER, true),
            Some(KeyOutcome::Edit(Edit::Enter))
        );
    }
}
