//! Settings › Shortcuts (`Shortcuts.swift`, `ShortcutSettingsView.swift`, `StoredShortcut.swift`): the app's commands
//! whose keyboard shortcut can be changed, their defaults, recording a new one and what refuses it.
//!
//! The macOS shortcuts map to this platform's keys: ⌘ is Ctrl, ⌥ is Alt, ⌘⌫ is Ctrl+Backspace. The Mac's Control key
//! has no counterpart of its own, so a shortcut here takes Ctrl, Alt and Shift.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::preferences::{PreferencesError, PreferencesStore};

/// The app's commands whose shortcut can be changed. Keys that work only in one place (arrows, Space, Escape, Enter
/// in a dialog) and the system's own stay fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutAction {
    Create,
    Queue,
    Library,
    Open,
    Generate,
    Find,
    RefreshHistory,
    Star,
    DeleteResult,
    CancelJob,
    RetryJob,
    Keep,
    Reject,
    ClearCulling,
}

impl ShortcutAction {
    pub const ALL: [Self; 14] = [
        Self::Create,
        Self::Queue,
        Self::Library,
        Self::Open,
        Self::Generate,
        Self::Find,
        Self::RefreshHistory,
        Self::Star,
        Self::DeleteResult,
        Self::CancelJob,
        Self::RetryJob,
        Self::Keep,
        Self::Reject,
        Self::ClearCulling,
    ];

    /// The stored value of an action whose shortcut was removed.
    pub const NONE: &'static str = "none";

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Queue => "queue",
            Self::Library => "library",
            Self::Open => "open",
            Self::Generate => "generate",
            Self::Find => "find",
            Self::RefreshHistory => "refreshHistory",
            Self::Star => "star",
            Self::DeleteResult => "deleteResult",
            Self::CancelJob => "cancelJob",
            Self::RetryJob => "retryJob",
            Self::Keep => "keep",
            Self::Reject => "reject",
            Self::ClearCulling => "clearCulling",
        }
    }

    /// Culling keys work only while the Library's grid or large viewer has the keyboard, never in a text field, so
    /// they take a single key, the way photo culling does.
    pub fn is_library_key(self) -> bool {
        matches!(self, Self::Keep | Self::Reject | Self::ClearCulling)
    }

    /// Whether the key goes to a text field being edited instead (`TextAwareShortcut`): the culling keys, and
    /// Delete from Library, whose Ctrl+Backspace deletes text there.
    pub fn yields_to_text(self) -> bool {
        self.is_library_key() || self == Self::DeleteResult
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Create => "Show Create",
            Self::Queue => "Show Queue",
            Self::Library => "Show Library",
            Self::Open => "Open a video or images",
            Self::Generate => "Generate",
            Self::Find => "Search a gallery",
            Self::RefreshHistory => "Refresh Higgsfield history",
            Self::Star => "Star or unstar results",
            Self::DeleteResult => "Delete results from Library",
            Self::CancelJob => "Cancel the selected job",
            Self::RetryJob => "Retry the selected job",
            Self::Keep => "Keep results in Library",
            Self::Reject => "Reject results in Library",
            Self::ClearCulling => "Clear keep or reject",
        }
    }

    pub fn default_shortcut(self) -> Shortcut {
        let ctrl = Modifiers::CTRL;
        match self {
            Self::Create => Shortcut::character('1', ctrl),
            Self::Queue => Shortcut::character('2', ctrl),
            Self::Library => Shortcut::character('3', ctrl),
            Self::Open => Shortcut::character('o', ctrl),
            Self::Generate => Shortcut::new(Key::Enter, ctrl),
            Self::Find => Shortcut::character('f', ctrl),
            Self::RefreshHistory => Shortcut::character('r', ctrl),
            Self::Star => Shortcut::character('l', ctrl),
            Self::DeleteResult => Shortcut::new(Key::Backspace, ctrl),
            // ⌘. is the Mac's "stop what's running".
            Self::CancelJob => Shortcut::character('.', ctrl),
            Self::RetryJob => Shortcut::character('r', Modifiers::CTRL_SHIFT),
            // P for pick, X for reject, U for unflag: the keys photo culling apps use.
            Self::Keep => Shortcut::character('p', Modifiers::NONE),
            Self::Reject => Shortcut::character('x', Modifiers::NONE),
            Self::ClearCulling => Shortcut::character('u', Modifiers::NONE),
        }
    }

    /// The preferences key. Absent or empty means the default; `NONE` means no shortcut.
    pub fn storage_key(self) -> String {
        format!("shortcuts.{}", self.raw_value())
    }

    /// The shortcut `stored` (the `storage_key` value) sets; `None` when it was removed.
    pub fn shortcut(self, stored: &str) -> Option<Shortcut> {
        if stored.is_empty() {
            return Some(self.default_shortcut());
        }
        if stored == Self::NONE {
            return None;
        }
        Some(Shortcut::parse(stored).unwrap_or_else(|| self.default_shortcut()))
    }
}

/// A key without its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A lowercased letter, digit or punctuation character.
    Character(char),
    Enter,
    Space,
    Tab,
    Backspace,
    Left,
    Right,
    Up,
    Down,
}

/// The named keys: the stored name, the name shown, and the `KeyboardEvent.code`s that type them.
const KEY_NAMES: [(Key, &str, &str, &[&str]); 8] = [
    (Key::Enter, "enter", "Enter", &["Enter", "NumpadEnter"]),
    (Key::Space, "space", "Space", &["Space"]),
    (Key::Tab, "tab", "Tab", &["Tab"]),
    (Key::Backspace, "backspace", "Backspace", &["Backspace"]),
    (Key::Left, "left", "Left", &["ArrowLeft"]),
    (Key::Right, "right", "Right", &["ArrowRight"]),
    (Key::Up, "up", "Up", &["ArrowUp"]),
    (Key::Down, "down", "Down", &["ArrowDown"]),
];

/// The punctuation keys of the US layout by `KeyboardEvent.code`, unshifted: a layout-independent key, as the macOS app
/// reads ⌘ shortcuts from the Latin layout.
const PUNCTUATION_CODES: [(&str, char); 11] = [
    ("Minus", '-'),
    ("Equal", '='),
    ("BracketLeft", '['),
    ("BracketRight", ']'),
    ("Backslash", '\\'),
    ("Semicolon", ';'),
    ("Quote", '\''),
    ("Comma", ','),
    ("Period", '.'),
    ("Slash", '/'),
    ("Backquote", '`'),
];

impl Key {
    fn stored_name(self) -> String {
        match self {
            Self::Character(character) => character.to_string(),
            named => KEY_NAMES
                .iter()
                .find(|(key, ..)| *key == named)
                .map_or_else(String::new, |(_, stored, ..)| (*stored).to_owned()),
        }
    }

    fn display_name(self) -> String {
        match self {
            Self::Character(character) => character.to_uppercase().collect(),
            named => KEY_NAMES
                .iter()
                .find(|(key, ..)| *key == named)
                .map_or_else(String::new, |(_, _, shown, _)| (*shown).to_owned()),
        }
    }

    /// The `KeyboardEvent.code`s of the physical keys that type it; empty for a character no US key types.
    fn codes(self) -> Vec<String> {
        match self {
            Self::Character(character @ 'a'..='z') => {
                vec![format!("Key{}", character.to_ascii_uppercase())]
            }
            Self::Character(digit @ '0'..='9') => vec![format!("Digit{digit}"), format!("Numpad{digit}")],
            Self::Character(character) => PUNCTUATION_CODES
                .iter()
                .filter(|(_, typed)| *typed == character)
                .map(|(code, _)| (*code).to_owned())
                .collect(),
            named => KEY_NAMES
                .iter()
                .find(|(key, ..)| *key == named)
                .map(|(.., codes)| codes.iter().map(|code| (*code).to_owned()).collect())
                .unwrap_or_default(),
        }
    }
}

/// Ctrl, Alt and Shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        alt: false,
        shift: false,
    };
    pub const CTRL_SHIFT: Self = Self {
        ctrl: true,
        alt: false,
        shift: true,
    };
    pub const CTRL_ALT: Self = Self {
        ctrl: true,
        alt: true,
        shift: false,
    };
    pub const ALT: Self = Self {
        ctrl: false,
        alt: true,
        shift: false,
    };

    /// In the order they are stored and shown.
    fn names(self) -> Vec<(&'static str, &'static str)> {
        [
            (self.ctrl, "ctrl", "Ctrl"),
            (self.alt, "alt", "Alt"),
            (self.shift, "shift", "Shift"),
        ]
        .into_iter()
        .filter(|(on, ..)| *on)
        .map(|(_, stored, shown)| (stored, shown))
        .collect()
    }
}

/// A key with its modifiers, stored as text such as `ctrl+shift+k` or `ctrl+enter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl Shortcut {
    /// A character key is kept lowercased.
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        let key = match key {
            Key::Character(character) => Key::Character(character.to_lowercase().next().unwrap_or(character)),
            named => named,
        };
        Self { key, modifiers }
    }

    pub fn character(character: char, modifiers: Modifiers) -> Self {
        Self::new(Key::Character(character), modifiers)
    }

    /// Parses the stored form; `None` for text that isn't one.
    pub fn parse(stored: &str) -> Option<Self> {
        // The key comes last and may itself be "+".
        let (prefix, last) = if let Some(prefix) = stored.strip_suffix("++") {
            (prefix, "+")
        } else if let Some(separator) = stored.rfind('+') {
            (&stored[..separator], &stored[separator + 1..])
        } else {
            ("", stored)
        };
        if last.is_empty() {
            return None;
        }
        let mut modifiers = Modifiers::NONE;
        if !prefix.is_empty() {
            for part in prefix.split('+') {
                match part {
                    "ctrl" => modifiers.ctrl = true,
                    "alt" => modifiers.alt = true,
                    "shift" => modifiers.shift = true,
                    _ => return None,
                }
            }
        }
        if let Some((key, ..)) = KEY_NAMES.iter().find(|(_, stored, ..)| *stored == last) {
            return Some(Self::new(*key, modifiers));
        }
        let mut characters = last.chars();
        match (characters.next(), characters.next()) {
            (Some(character), None) => Some(Self::character(character, modifiers)),
            _ => None,
        }
    }

    pub fn stored(&self) -> String {
        let mut parts: Vec<String> = self
            .modifiers
            .names()
            .into_iter()
            .map(|(stored, _)| stored.to_owned())
            .collect();
        parts.push(self.key.stored_name());
        parts.join("+")
    }

    /// As menus here show it, e.g. `Ctrl+Shift+K`.
    pub fn display_text(&self) -> String {
        let mut parts: Vec<String> = self
            .modifiers
            .names()
            .into_iter()
            .map(|(_, shown)| shown.to_owned())
            .collect();
        parts.push(self.key.display_name());
        parts.join("+")
    }

    /// The menu item's accelerator, as the window menu reads it (`Ctrl+Shift+R`); `None` for a character no US key
    /// types, which the menu can't bind.
    pub fn accelerator(&self) -> Option<String> {
        (!self.key.codes().is_empty()).then(|| self.display_text())
    }

    /// Shortcuts of the system and of the app's fixed commands: the macOS app's ⌘ ones with Ctrl, Ctrl+Shift+Z,
    /// Ctrl+Alt+H and Ctrl+Alt+W, View › Toggle Sidebar's Ctrl+Shift+S (⌃⌘S there), and the window and input
    /// switches (⌘Tab, ⌘Space, ⌃Space there).
    fn is_reserved(self) -> bool {
        const CTRL_KEYS: [char; 15] = [
            'q', 'w', 'h', 'm', 'n', ',', 'a', 'c', 'v', 'x', 'z', 's', 'p', '?', '`',
        ];
        if let Key::Character(character) = self.key {
            if self.modifiers == Modifiers::CTRL && CTRL_KEYS.contains(&character) {
                return true;
            }
            if (self.modifiers == Modifiers::CTRL_SHIFT && matches!(character, 'z' | 's'))
                || (self.modifiers == Modifiers::CTRL_ALT && matches!(character, 'h' | 'w'))
            {
                return true;
            }
        }
        matches!(self.key, Key::Tab | Key::Space)
            && (self.modifiers == Modifiers::CTRL || self.modifiers == Modifiers::ALT)
    }

    /// Keys the Library's grid and large viewer already use without a modifier: moving, the preview, opening.
    fn is_library_reserved(key: Key) -> bool {
        !matches!(key, Key::Character(_))
    }

    /// Whether `action` can take this shortcut while the others keep `assigned`; `None` when it can.
    pub fn refusal(
        self,
        action: ShortcutAction,
        assigned: &HashMap<ShortcutAction, Shortcut>,
    ) -> Option<ShortcutRefusal> {
        let is_plain = !self.modifiers.ctrl && !self.modifiers.alt;
        if is_plain && !action.is_library_key() {
            return Some(ShortcutRefusal::NeedsModifier);
        }
        if is_plain && Self::is_library_reserved(self.key) {
            return Some(ShortcutRefusal::Reserved);
        }
        if self.is_reserved() {
            return Some(ShortcutRefusal::Reserved);
        }
        ShortcutAction::ALL
            .into_iter()
            .find(|other| *other != action && assigned.get(other) == Some(&self))
            .map(ShortcutRefusal::UsedBy)
    }
}

/// Why a recorded shortcut can't be used, as the recorder shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutRefusal {
    /// Without Ctrl or Alt it would fire while typing.
    NeedsModifier,
    Reserved,
    UsedBy(ShortcutAction),
}

impl ShortcutRefusal {
    pub fn message(self) -> String {
        match self {
            Self::NeedsModifier => "Add Ctrl or Alt so it doesn't fire while typing".to_owned(),
            Self::Reserved => "The system or the app already uses it".to_owned(),
            Self::UsedBy(action) => format!("Already used by {}", action.title()),
        }
    }
}

/// What a key press does while the shortcut of an action is being recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutKeyPress {
    /// Escape: stop recording, keep the shortcut.
    Cancel,
    /// Store this value (`storage_key`) and stop: a shortcut, `ShortcutAction::NONE`, or "" for the default.
    Save(String),
    Refuse(ShortcutRefusal),
    /// A key that makes no shortcut, such as F5 or a modifier alone; keep recording.
    Ignore,
}

impl ShortcutAction {
    /// `shortcut` is the key press, `None` for a key without a character. Escape and Delete only count without
    /// modifiers; with them they are shortcuts like any other.
    pub fn key_press(
        self,
        shortcut: Option<Shortcut>,
        is_escape: bool,
        is_delete: bool,
        assigned: &HashMap<ShortcutAction, Shortcut>,
    ) -> ShortcutKeyPress {
        if is_escape {
            return ShortcutKeyPress::Cancel;
        }
        if is_delete {
            return ShortcutKeyPress::Save(Self::NONE.to_owned());
        }
        let Some(shortcut) = shortcut else {
            return ShortcutKeyPress::Ignore;
        };
        if let Some(refusal) = shortcut.refusal(self, assigned) {
            return ShortcutKeyPress::Refuse(refusal);
        }
        ShortcutKeyPress::Save(if shortcut == self.default_shortcut() {
            String::new()
        } else {
            shortcut.stored()
        })
    }
}

/// A key press as the window reports it (`KeyboardEvent`'s `key`, `code` and modifier flags).
#[allow(
    clippy::struct_excessive_bools,
    reason = "the modifier keys, as `KeyboardEvent` has them"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyStroke {
    pub key: String,
    pub code: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Windows or Super key; no shortcut here uses it.
    pub meta: bool,
}

impl KeyStroke {
    fn is_plain(&self) -> bool {
        !self.ctrl && !self.alt && !self.shift && !self.meta
    }

    pub fn is_escape(&self) -> bool {
        self.is_plain() && self.key == "Escape"
    }

    /// Backspace or Delete alone.
    pub fn is_delete(&self) -> bool {
        self.is_plain() && (self.key == "Backspace" || self.key == "Delete")
    }

    /// The shortcut the press makes; `None` for a key with no character (F5, a modifier alone) or with the Windows
    /// key. Letters, digits and punctuation are read by their place on the US layout, so another layout and Shift
    /// still give the key's own character.
    pub fn shortcut(&self) -> Option<Shortcut> {
        if self.meta {
            return None;
        }
        let modifiers = Modifiers {
            ctrl: self.ctrl,
            alt: self.alt,
            shift: self.shift,
        };
        if let Some((key, ..)) = KEY_NAMES
            .iter()
            .find(|(.., codes)| codes.contains(&self.code.as_str()))
        {
            return Some(Shortcut::new(*key, modifiers));
        }
        let from_code = if let Some(letter) = self.code.strip_prefix("Key") {
            single_char(letter)
        } else if let Some(digit) = self
            .code
            .strip_prefix("Digit")
            .or_else(|| self.code.strip_prefix("Numpad"))
        {
            single_char(digit).filter(char::is_ascii_digit)
        } else {
            PUNCTUATION_CODES
                .iter()
                .find(|(code, _)| *code == self.code)
                .map(|(_, character)| *character)
        };
        let character = from_code.or_else(|| single_char(&self.key))?;
        if character.is_whitespace() || character.is_control() {
            return None;
        }
        Some(Shortcut::character(character, modifiers))
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut characters = text.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) => Some(character),
        _ => None,
    }
}

/// A shortcut as the window matches key presses against it.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the modifier keys, as `KeyboardEvent` has them"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    pub action: ShortcutAction,
    /// `KeyboardEvent.code`s of the key; empty when the key is matched by `key`.
    pub codes: Vec<String>,
    /// The lowercased character, for a key no US key types.
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The key goes to a text field being edited instead.
    pub yields_to_text: bool,
}

impl ShortcutBinding {
    fn new(action: ShortcutAction, shortcut: Shortcut) -> Self {
        Self {
            action,
            codes: shortcut.key.codes(),
            key: shortcut.key.stored_name(),
            ctrl: shortcut.modifiers.ctrl,
            alt: shortcut.modifiers.alt,
            shift: shortcut.modifiers.shift,
            yields_to_text: action.yields_to_text(),
        }
    }
}

/// One row of the Shortcuts tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutRow {
    pub action: ShortcutAction,
    pub title: String,
    /// `None` once the shortcut was removed.
    pub shortcut: Option<String>,
    /// Set in Settings, so Reset is offered.
    pub is_changed: bool,
    /// "Back to Ctrl+1".
    pub reset_hint: String,
}

/// What the Shortcuts tab shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutSettings {
    pub rows: Vec<ShortcutRow>,
    pub can_restore_defaults: bool,
}

/// What a key press did while recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ShortcutRecording {
    /// Recording stops; nothing changed.
    Cancel,
    /// Recording stops; the shortcut was stored.
    Saved,
    /// Recording goes on; the row says why.
    Refused { message: String },
    /// Recording goes on.
    Ignored,
}

/// The shortcuts in the preferences.
pub struct ShortcutStore<'a>(pub &'a PreferencesStore);

impl ShortcutStore<'_> {
    /// The value stored for `action`; `None` while it keeps its default.
    pub fn stored(&self, action: ShortcutAction) -> Option<String> {
        match self.0.value(&action.storage_key()) {
            Some(Value::String(value)) if !value.is_empty() => Some(value),
            _ => None,
        }
    }

    pub fn shortcut(&self, action: ShortcutAction) -> Option<Shortcut> {
        action.shortcut(&self.stored(action).unwrap_or_default())
    }

    pub fn assigned(&self) -> HashMap<ShortcutAction, Shortcut> {
        ShortcutAction::ALL
            .into_iter()
            .filter_map(|action| Some((action, self.shortcut(action)?)))
            .collect()
    }

    /// Stores `value` for `action`; "" goes back to the default.
    pub fn save(&self, action: ShortcutAction, value: &str) -> Result<(), PreferencesError> {
        let value = (!value.is_empty()).then(|| Value::String(value.to_owned()));
        self.0.set_value(&action.storage_key(), value)
    }

    pub fn restore_defaults(&self) -> Result<(), PreferencesError> {
        for action in ShortcutAction::ALL {
            if self.stored(action).is_some() {
                self.save(action, "")?;
            }
        }
        Ok(())
    }

    pub fn settings(&self) -> ShortcutSettings {
        let rows: Vec<ShortcutRow> = ShortcutAction::ALL
            .into_iter()
            .map(|action| ShortcutRow {
                action,
                title: action.title().to_owned(),
                shortcut: self.shortcut(action).map(|shortcut| shortcut.display_text()),
                is_changed: self.stored(action).is_some(),
                reset_hint: format!("Back to {}", action.default_shortcut().display_text()),
            })
            .collect();
        ShortcutSettings {
            can_restore_defaults: rows.iter().any(|row| row.is_changed),
            rows,
        }
    }

    pub fn bindings(&self) -> Vec<ShortcutBinding> {
        ShortcutAction::ALL
            .into_iter()
            .filter_map(|action| Some(ShortcutBinding::new(action, self.shortcut(action)?)))
            .collect()
    }

    /// Records `stroke` as `action`'s shortcut, storing it when it can be used.
    pub fn record(
        &self,
        action: ShortcutAction,
        stroke: &KeyStroke,
    ) -> Result<ShortcutRecording, PreferencesError> {
        let press = action.key_press(
            stroke.shortcut(),
            stroke.is_escape(),
            stroke.is_delete(),
            &self.assigned(),
        );
        Ok(match press {
            ShortcutKeyPress::Cancel => ShortcutRecording::Cancel,
            ShortcutKeyPress::Save(value) => {
                self.save(action, &value)?;
                ShortcutRecording::Saved
            }
            ShortcutKeyPress::Refuse(refusal) => ShortcutRecording::Refused {
                message: refusal.message(),
            },
            ShortcutKeyPress::Ignore => ShortcutRecording::Ignored,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> HashMap<ShortcutAction, Shortcut> {
        ShortcutAction::ALL
            .into_iter()
            .map(|action| (action, action.default_shortcut()))
            .collect()
    }

    fn queue_only() -> HashMap<ShortcutAction, Shortcut> {
        HashMap::from([(ShortcutAction::Queue, ShortcutAction::Queue.default_shortcut())])
    }

    fn stroke(key: &str, code: &str, modifiers: Modifiers) -> KeyStroke {
        KeyStroke {
            key: key.to_owned(),
            code: code.to_owned(),
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            meta: false,
        }
    }

    #[test]
    fn a_shortcut_round_trips_through_its_stored_text() {
        let shortcuts = [
            Shortcut::character('K', Modifiers::CTRL_SHIFT),
            Shortcut::new(Key::Enter, Modifiers::CTRL),
            Shortcut::character('+', Modifiers::CTRL_ALT),
            Shortcut::new(Key::Left, Modifiers::CTRL_ALT),
        ];
        for shortcut in shortcuts {
            assert_eq!(Shortcut::parse(&shortcut.stored()), Some(shortcut));
        }
        assert_eq!(shortcuts[0].stored(), "ctrl+shift+k");
        assert_eq!(shortcuts[2].stored(), "ctrl+alt++");
    }

    #[test]
    fn the_display_follows_the_menu_order() {
        let all = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
        };
        assert_eq!(Shortcut::character('k', all).display_text(), "Ctrl+Alt+Shift+K");
        assert_eq!(
            Shortcut::new(Key::Enter, Modifiers::CTRL).display_text(),
            "Ctrl+Enter"
        );
        assert_eq!(ShortcutAction::Create.default_shortcut().display_text(), "Ctrl+1");
        assert_eq!(
            ShortcutAction::DeleteResult.default_shortcut().display_text(),
            "Ctrl+Backspace"
        );
    }

    #[test]
    fn text_that_isnt_a_shortcut_doesnt_parse() {
        for text in ["", "ctrl+", "hyper+k", "ctrl+kk", "+", "cmd+k"] {
            assert_eq!(Shortcut::parse(text), None, "{text}");
        }
    }

    #[test]
    fn stored_values_resolve_to_defaults_and_removals() {
        let queue = ShortcutAction::Queue;
        assert_eq!(queue.shortcut(""), Some(queue.default_shortcut()));
        assert_eq!(queue.shortcut(ShortcutAction::NONE), None);
        assert_eq!(queue.shortcut("garbage+x"), Some(queue.default_shortcut()));
        assert_eq!(
            queue.shortcut("ctrl+alt+q"),
            Some(Shortcut::character('q', Modifiers::CTRL_ALT))
        );
    }

    #[test]
    fn the_defaults_dont_clash_or_hit_reserved_keys() {
        let defaults = defaults();
        for action in ShortcutAction::ALL {
            assert_eq!(
                action.default_shortcut().refusal(action, &defaults),
                None,
                "{action:?}"
            );
        }
    }

    #[test]
    fn a_recorded_shortcut_is_refused_when_it_cant_work() {
        let assigned = queue_only();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        let create = ShortcutAction::Create;
        assert_eq!(
            Shortcut::character('k', shift).refusal(create, &assigned),
            Some(ShortcutRefusal::NeedsModifier)
        );
        assert_eq!(
            Shortcut::character('q', Modifiers::CTRL).refusal(create, &assigned),
            Some(ShortcutRefusal::Reserved)
        );
        assert_eq!(
            ShortcutAction::Queue
                .default_shortcut()
                .refusal(create, &assigned),
            Some(ShortcutRefusal::UsedBy(ShortcutAction::Queue))
        );
        assert_eq!(
            ShortcutAction::Queue
                .default_shortcut()
                .refusal(ShortcutAction::Queue, &assigned),
            None
        );
        assert_eq!(
            Shortcut::character('k', Modifiers::ALT).refusal(create, &assigned),
            None
        );
        assert_eq!(
            Shortcut::new(Key::Space, Modifiers::CTRL).refusal(create, &assigned),
            Some(ShortcutRefusal::Reserved)
        );
        // View › Toggle Sidebar.
        assert_eq!(
            Shortcut::character('s', Modifiers::CTRL_SHIFT).refusal(create, &assigned),
            Some(ShortcutRefusal::Reserved)
        );
    }

    #[test]
    fn the_menu_binds_keys_the_us_layout_types() {
        assert_eq!(
            ShortcutAction::RetryJob
                .default_shortcut()
                .accelerator()
                .as_deref(),
            Some("Ctrl+Shift+R")
        );
        assert_eq!(
            ShortcutAction::CancelJob
                .default_shortcut()
                .accelerator()
                .as_deref(),
            Some("Ctrl+.")
        );
        assert_eq!(
            ShortcutAction::Generate
                .default_shortcut()
                .accelerator()
                .as_deref(),
            Some("Ctrl+Enter")
        );
        assert_eq!(Shortcut::character('é', Modifiers::CTRL).accelerator(), None);
    }

    #[test]
    fn culling_keys_take_a_single_key_but_not_one_the_grid_uses() {
        let defaults = defaults();
        let k = Shortcut::character('k', Modifiers::NONE);
        assert_eq!(k.refusal(ShortcutAction::Keep, &defaults), None);
        assert_eq!(
            k.refusal(ShortcutAction::Star, &defaults),
            Some(ShortcutRefusal::NeedsModifier)
        );
        assert_eq!(
            Shortcut::new(Key::Space, Modifiers::NONE).refusal(ShortcutAction::Keep, &defaults),
            Some(ShortcutRefusal::Reserved)
        );
        assert_eq!(
            Shortcut::new(Key::Right, Modifiers::NONE).refusal(ShortcutAction::Reject, &defaults),
            Some(ShortcutRefusal::Reserved)
        );
        assert_eq!(
            Shortcut::character('z', Modifiers::CTRL).refusal(ShortcutAction::Keep, &defaults),
            Some(ShortcutRefusal::Reserved)
        );
        // Each culling key against the existing table: the others' keys and the app's Ctrl shortcuts.
        assert_eq!(
            ShortcutAction::Reject
                .default_shortcut()
                .refusal(ShortcutAction::Keep, &defaults),
            Some(ShortcutRefusal::UsedBy(ShortcutAction::Reject))
        );
        assert_eq!(
            ShortcutAction::Star
                .default_shortcut()
                .refusal(ShortcutAction::ClearCulling, &defaults),
            Some(ShortcutRefusal::UsedBy(ShortcutAction::Star))
        );
        assert_eq!(
            ShortcutAction::Keep
                .default_shortcut()
                .refusal(ShortcutAction::Create, &defaults),
            Some(ShortcutRefusal::NeedsModifier)
        );
        assert_eq!(
            ShortcutAction::Keep.key_press(Some(k), false, false, &defaults),
            ShortcutKeyPress::Save("k".to_owned())
        );
    }

    #[test]
    fn a_key_press_while_recording_saves_removes_cancels_or_refuses() {
        let assigned = queue_only();
        let create = ShortcutAction::Create;
        let new = Shortcut::character('k', Modifiers::CTRL_SHIFT);
        assert_eq!(
            create.key_press(Some(new), true, false, &assigned),
            ShortcutKeyPress::Cancel
        );
        assert_eq!(
            create.key_press(None, false, true, &assigned),
            ShortcutKeyPress::Save(ShortcutAction::NONE.to_owned())
        );
        assert_eq!(
            create.key_press(None, false, false, &assigned),
            ShortcutKeyPress::Ignore
        );
        assert_eq!(
            create.key_press(Some(new), false, false, &assigned),
            ShortcutKeyPress::Save("ctrl+shift+k".to_owned())
        );
        assert_eq!(
            create.key_press(Some(create.default_shortcut()), false, false, &assigned),
            ShortcutKeyPress::Save(String::new())
        );
        assert_eq!(
            create.key_press(
                Some(ShortcutAction::Queue.default_shortcut()),
                false,
                false,
                &assigned
            ),
            ShortcutKeyPress::Refuse(ShortcutRefusal::UsedBy(ShortcutAction::Queue))
        );
    }

    #[test]
    fn key_strokes_are_read_by_their_place_on_the_us_layout() {
        // Shift+1 types "!", and a Russian layout types "щ" on the O key; the shortcut keeps the key's own character.
        assert_eq!(
            stroke("!", "Digit1", Modifiers::CTRL_SHIFT).shortcut(),
            Some(Shortcut::character('1', Modifiers::CTRL_SHIFT))
        );
        assert_eq!(
            stroke("щ", "KeyO", Modifiers::CTRL).shortcut(),
            Some(Shortcut::character('o', Modifiers::CTRL))
        );
        assert_eq!(
            stroke("<", "Comma", Modifiers::CTRL_SHIFT).shortcut(),
            Some(Shortcut::character(',', Modifiers::CTRL_SHIFT))
        );
        assert_eq!(
            stroke("Enter", "NumpadEnter", Modifiers::CTRL).shortcut(),
            Some(Shortcut::new(Key::Enter, Modifiers::CTRL))
        );
        assert_eq!(stroke("F5", "F5", Modifiers::CTRL).shortcut(), None);
        assert_eq!(stroke("Control", "ControlLeft", Modifiers::CTRL).shortcut(), None);
        let mut windows_key = stroke("k", "KeyK", Modifiers::NONE);
        windows_key.meta = true;
        assert_eq!(windows_key.shortcut(), None);
        assert!(stroke("Escape", "Escape", Modifiers::NONE).is_escape());
        assert!(!stroke("Escape", "Escape", Modifiers::CTRL).is_escape());
        assert!(stroke("Delete", "Delete", Modifiers::NONE).is_delete());
        assert!(stroke("Backspace", "Backspace", Modifiers::NONE).is_delete());
        assert!(!stroke("Backspace", "Backspace", Modifiers::CTRL).is_delete());
    }

    #[test]
    fn bindings_name_the_physical_keys() {
        let binding = ShortcutBinding::new(ShortcutAction::Create, ShortcutAction::Create.default_shortcut());
        assert_eq!(binding.codes, ["Digit1", "Numpad1"]);
        assert!(binding.ctrl && !binding.alt && !binding.shift && !binding.yields_to_text);
        let delete = ShortcutBinding::new(
            ShortcutAction::DeleteResult,
            ShortcutAction::DeleteResult.default_shortcut(),
        );
        assert_eq!(delete.codes, ["Backspace"]);
        assert!(delete.yields_to_text);
        let foreign = ShortcutBinding::new(ShortcutAction::Find, Shortcut::character('é', Modifiers::CTRL));
        assert_eq!(foreign.codes, Vec::<String>::new());
        assert_eq!(foreign.key, "é");
    }

    #[test]
    fn the_store_records_resets_and_restores() {
        let folder = tempfile::tempdir().unwrap();
        let preferences = PreferencesStore::open(folder.path());
        let store = ShortcutStore(&preferences);
        let settings = store.settings();
        assert_eq!(settings.rows.len(), 14);
        assert_eq!(settings.rows[0].shortcut.as_deref(), Some("Ctrl+1"));
        assert_eq!(settings.rows[0].reset_hint, "Back to Ctrl+1");
        assert!(!settings.can_restore_defaults);
        assert_eq!(store.bindings().len(), 14);

        let create = ShortcutAction::Create;
        assert_eq!(
            store
                .record(create, &stroke("K", "KeyK", Modifiers::CTRL_SHIFT))
                .unwrap(),
            ShortcutRecording::Saved
        );
        assert_eq!(store.stored(create).as_deref(), Some("ctrl+shift+k"));
        assert_eq!(
            store
                .record(ShortcutAction::Queue, &stroke("K", "KeyK", Modifiers::CTRL_SHIFT))
                .unwrap(),
            ShortcutRecording::Refused {
                message: "Already used by Show Create".to_owned()
            }
        );
        assert_eq!(
            store
                .record(create, &stroke("Shift", "ShiftLeft", Modifiers::NONE))
                .unwrap(),
            ShortcutRecording::Ignored
        );
        assert_eq!(
            store
                .record(create, &stroke("Escape", "Escape", Modifiers::NONE))
                .unwrap(),
            ShortcutRecording::Cancel
        );
        store
            .record(ShortcutAction::Find, &stroke("Delete", "Delete", Modifiers::NONE))
            .unwrap();
        let settings = store.settings();
        assert_eq!(settings.rows[0].shortcut.as_deref(), Some("Ctrl+Shift+K"));
        assert!(settings.rows[0].is_changed && settings.can_restore_defaults);
        assert_eq!(settings.rows[5].shortcut, None);
        assert_eq!(store.bindings().len(), 13);

        let reopened = PreferencesStore::open(folder.path());
        assert_eq!(
            ShortcutStore(&reopened).stored(create).as_deref(),
            Some("ctrl+shift+k")
        );

        store.save(create, "").unwrap();
        assert_eq!(store.stored(create), None);
        store.restore_defaults().unwrap();
        assert!(!store.settings().can_restore_defaults);
    }
}
