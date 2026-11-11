//! Key expression parsing (#14), compatible with VS Code `keybindings.json`.
//!
//! A key expression is one or more chords separated by spaces (`ctrl+k ctrl+s`). Each chord is
//! modifiers followed by one key; modifiers are joined with `+` (VS Code also accepts `-`).
//! Matching is case-insensitive. Keys are normalized to the VS Code "general" names, so the US
//! punctuation aliases and the Windows virtual-key names compare equal (`/` == `oem_2`,
//! `=` == `oem_plus`). Scan-code names in brackets (`[Slash]`, `[KeyA]`) map to the key the US
//! layout produces at that position, which is what `keyboard.dispatch: code` (VS Code's default)
//! resolves on that layout. No layout detection is attempted (issue #14, risk notes).
use crate::KeyStroke;
use std::{fmt, sync::Arc};

/// Why a key expression was rejected. Diagnostic only: never shown to the user as raw text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyParseError {
    pub input: String,
    pub reason: KeyParseErrorKind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyParseErrorKind {
    /// Nothing but whitespace.
    Empty,
    /// A chord ended after a modifier (`ctrl+`).
    MissingKey,
    /// The key name is not a known VS Code key name.
    UnknownKey(String),
}
impl fmt::Display for KeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.reason {
            KeyParseErrorKind::Empty => write!(f, "empty key expression"),
            KeyParseErrorKind::MissingKey => write!(f, "missing key in `{}`", self.input),
            KeyParseErrorKind::UnknownKey(k) => {
                write!(f, "unknown key `{k}` in `{}`", self.input)
            }
        }
    }
}
impl std::error::Error for KeyParseError {}
impl From<KeyParseError> for rove_base::Error {
    fn from(_: KeyParseError) -> Self {
        rove_base::Error::InvalidInput
    }
}

/// Parse a full key expression (one or more chords).
pub fn parse_keybinding(input: &str) -> Result<Vec<KeyStroke>, KeyParseError> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() {
        return Err(error(input, KeyParseErrorKind::Empty));
    }
    parts
        .into_iter()
        .map(|chord| parse_chord_in(chord, input))
        .collect()
}

/// Parse a single chord such as `ctrl+shift+p`.
pub fn parse_chord(input: &str) -> Result<KeyStroke, KeyParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(error(input, KeyParseErrorKind::Empty));
    }
    parse_chord_in(trimmed, input)
}

fn parse_chord_in(chord: &str, whole: &str) -> Result<KeyStroke, KeyParseError> {
    let lower = chord.to_ascii_lowercase();
    let mut rest = lower.as_str();
    let (mut ctrl, mut shift, mut alt, mut meta) = (false, false, false, false);
    fn modifier(rest: &str) -> Option<(&'static str, &str)> {
        ["ctrl", "shift", "alt", "meta", "win", "cmd"]
            .iter()
            .find_map(|m| {
                let after = rest.strip_prefix(m)?;
                let after = after
                    .strip_prefix('+')
                    .or_else(|| after.strip_prefix('-'))?;
                Some((*m, after))
            })
    }
    while let Some((name, after)) = modifier(rest) {
        match name {
            "ctrl" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            _ => meta = true,
        }
        rest = after;
    }
    if rest.is_empty() {
        return Err(error(whole, KeyParseErrorKind::MissingKey));
    }
    let key = normalize_key(rest)
        .ok_or_else(|| error(whole, KeyParseErrorKind::UnknownKey(rest.to_owned())))?;
    Ok(KeyStroke {
        key: key.into(),
        ctrl,
        shift,
        alt,
        meta,
    })
}

fn error(input: &str, reason: KeyParseErrorKind) -> KeyParseError {
    KeyParseError {
        input: input.to_owned(),
        reason,
    }
}

/// Map any accepted spelling (case-insensitive) to the canonical VS Code general key name.
/// Returns None for unknown names. Modifier keys alone are reported as `ctrl`/`shift`/`alt`/`meta`.
pub fn normalize_key(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    let name = lower.as_str();
    if let Some(code) = name.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return scan_code(code);
    }
    if let Some(&(_, canonical)) = ALIASES.iter().find(|(alias, _)| *alias == name) {
        return Some(canonical);
    }
    KEYS.iter().copied().find(|k| *k == name)
}

/// Whether the key is a bare modifier (pressing it alone never resolves or aborts a chord).
pub fn is_modifier_key(key: &str) -> bool {
    matches!(key, "ctrl" | "shift" | "alt" | "meta")
}

/// Canonical text of a chord: `ctrl+shift+alt+meta+key` with only the present modifiers.
pub fn format_stroke(k: &KeyStroke) -> String {
    let mut s = String::new();
    for (on, name) in [
        (k.ctrl, "ctrl+"),
        (k.shift, "shift+"),
        (k.alt, "alt+"),
        (k.meta, "meta+"),
    ] {
        if on {
            s.push_str(name);
        }
    }
    s.push_str(&k.key);
    s
}

/// Canonical text of a chord sequence, chords separated by one space.
pub fn format_keybinding(keys: &[KeyStroke]) -> String {
    keys.iter().map(format_stroke).collect::<Vec<_>>().join(" ")
}

/// Strokes are equal when modifiers and the normalized key match.
pub fn same_stroke(a: &KeyStroke, b: &KeyStroke) -> bool {
    a.ctrl == b.ctrl
        && a.shift == b.shift
        && a.alt == b.alt
        && a.meta == b.meta
        && (a.key == b.key
            || normalize_key(&a.key).is_some_and(|k| Some(k) == normalize_key(&b.key)))
}

/// Build a stroke from a key name already reported by an input layer.
pub fn stroke(key: &str, ctrl: bool, shift: bool, alt: bool, meta: bool) -> Option<KeyStroke> {
    Some(KeyStroke {
        key: Arc::from(normalize_key(key)?),
        ctrl,
        shift,
        alt,
        meta,
    })
}

/// VS Code general key names (lower case), see `vs/base/common/keyCodes.ts`.
const KEYS: &[&str] = &[
    "ctrl",
    "shift",
    "alt",
    "meta",
    "backspace",
    "tab",
    "enter",
    "pausebreak",
    "capslock",
    "escape",
    "space",
    "pageup",
    "pagedown",
    "end",
    "home",
    "left",
    "up",
    "right",
    "down",
    "insert",
    "delete",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "contextmenu",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
    "f13",
    "f14",
    "f15",
    "f16",
    "f17",
    "f18",
    "f19",
    "f20",
    "f21",
    "f22",
    "f23",
    "f24",
    "numlock",
    "scrolllock",
    "oem_1",
    "oem_plus",
    "oem_comma",
    "oem_minus",
    "oem_period",
    "oem_2",
    "oem_3",
    "oem_4",
    "oem_5",
    "oem_6",
    "oem_7",
    "oem_8",
    "oem_102",
    "abnt_c1",
    "abnt_c2",
    "numpad0",
    "numpad1",
    "numpad2",
    "numpad3",
    "numpad4",
    "numpad5",
    "numpad6",
    "numpad7",
    "numpad8",
    "numpad9",
    "numpad_multiply",
    "numpad_add",
    "numpad_separator",
    "numpad_subtract",
    "numpad_decimal",
    "numpad_divide",
    "browsersearch",
    "browserhome",
    "browserback",
    "browserforward",
    "mediatracknext",
    "mediatrackprevious",
    "mediastop",
    "mediaplaypause",
    "launchmediaplayer",
    "launchmail",
    "launchapp2",
    "audiovolumemute",
    "audiovolumeup",
    "audiovolumedown",
];

/// US-layout characters and alternative spellings accepted by VS Code for the same key code.
const ALIASES: &[(&str, &str)] = &[
    (";", "oem_1"),
    ("=", "oem_plus"),
    (",", "oem_comma"),
    ("-", "oem_minus"),
    (".", "oem_period"),
    ("/", "oem_2"),
    ("`", "oem_3"),
    ("[", "oem_4"),
    ("\\", "oem_5"),
    ("]", "oem_6"),
    ("'", "oem_7"),
    ("esc", "escape"),
    ("return", "enter"),
    ("pause", "pausebreak"),
    ("del", "delete"),
    ("ins", "insert"),
    ("arrowleft", "left"),
    ("arrowright", "right"),
    ("arrowup", "up"),
    ("arrowdown", "down"),
    ("control", "ctrl"),
    ("cmd", "meta"),
    ("win", "meta"),
    ("numpad_enter", "enter"),
];

/// `[ScanCode]` names (KeyboardEvent.code) mapped to the US-layout key at that position.
fn scan_code(code: &str) -> Option<&'static str> {
    let single = |s: &str, ok: fn(u8) -> bool| {
        (s.len() == 1 && ok(s.as_bytes()[0]))
            .then(|| KEYS.iter().copied().find(|k| *k == s))
            .flatten()
    };
    if let Some(letter) = code.strip_prefix("key") {
        return single(letter, |b| b.is_ascii_lowercase());
    }
    if let Some(digit) = code.strip_prefix("digit") {
        return single(digit, |b| b.is_ascii_digit());
    }
    if let Some(n) = code.strip_prefix("numpad") {
        if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() {
            return KEYS
                .iter()
                .copied()
                .find(|k| k.strip_prefix("numpad") == Some(n));
        }
    }
    Some(match code {
        "semicolon" => "oem_1",
        "equal" => "oem_plus",
        "comma" => "oem_comma",
        "minus" => "oem_minus",
        "period" => "oem_period",
        "slash" => "oem_2",
        "backquote" => "oem_3",
        "bracketleft" => "oem_4",
        "backslash" => "oem_5",
        "bracketright" => "oem_6",
        "quote" => "oem_7",
        "intlbackslash" => "oem_102",
        "arrowleft" => "left",
        "arrowright" => "right",
        "arrowup" => "up",
        "arrowdown" => "down",
        "numpadmultiply" => "numpad_multiply",
        "numpadadd" => "numpad_add",
        "numpadcomma" => "numpad_separator",
        "numpadsubtract" => "numpad_subtract",
        "numpaddecimal" => "numpad_decimal",
        "numpaddivide" => "numpad_divide",
        "numpadenter" => "enter",
        "controlleft" | "controlright" => "ctrl",
        "shiftleft" | "shiftright" => "shift",
        "altleft" | "altright" => "alt",
        "metaleft" | "metaright" => "meta",
        // Named keys share their scan-code name: [Enter], [Escape], [F1], [PageUp], [Home]...
        other => {
            return KEYS
                .iter()
                .copied()
                .find(|k| *k == other && !k.starts_with("oem_"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn k(s: &str) -> String {
        format_keybinding(&parse_keybinding(s).unwrap())
    }
    #[test]
    fn modifiers_and_case() {
        assert_eq!(k("ctrl+shift+p"), "ctrl+shift+p");
        assert_eq!(k("Ctrl+Shift+P"), "ctrl+shift+p");
        assert_eq!(k("shift+ctrl+P"), "ctrl+shift+p");
        assert_eq!(k("alt+f12"), "alt+f12");
        assert_eq!(k("ctrl+shift+alt+b"), "ctrl+shift+alt+b");
        assert_eq!(k("win+e"), "meta+e");
        assert_eq!(k("cmd+e"), "meta+e");
        // VS Code also accepts `-` between modifiers.
        assert_eq!(k("ctrl-shift-p"), "ctrl+shift+p");
        assert_eq!(k("  f5  "), "f5");
    }
    #[test]
    fn chords() {
        let keys = parse_keybinding("ctrl+k ctrl+s").unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(k("ctrl+k   ctrl+s"), "ctrl+k ctrl+s");
        assert_eq!(k("escape escape"), "escape escape");
        assert_eq!(k("ctrl+k b"), "ctrl+k b");
    }
    #[test]
    fn windows_virtual_key_names() {
        assert_eq!(k("ctrl+oem_2"), "ctrl+oem_2");
        assert_eq!(k("ctrl+/"), "ctrl+oem_2");
        assert_eq!(k("ctrl+oem_5"), "ctrl+oem_5");
        assert_eq!(k("ctrl+\\"), "ctrl+oem_5");
        assert_eq!(k("ctrl+oem_comma"), "ctrl+oem_comma");
        assert_eq!(k("ctrl+,"), "ctrl+oem_comma");
        assert_eq!(k("ctrl+oem_period"), "ctrl+oem_period");
        assert_eq!(k("ctrl+oem_plus"), "ctrl+oem_plus");
        assert_eq!(k("ctrl+="), "ctrl+oem_plus");
        assert_eq!(k("ctrl+-"), "ctrl+oem_minus");
        assert_eq!(k("ctrl+shift+oem_minus"), "ctrl+shift+oem_minus");
        assert_eq!(k("ctrl+numpad_add"), "ctrl+numpad_add");
        assert_eq!(k("ctrl+numpad0"), "ctrl+numpad0");
        assert_eq!(k("ctrl+NUMPAD_SUBTRACT"), "ctrl+numpad_subtract");
        assert_eq!(k("ctrl+oem_102"), "ctrl+oem_102");
        assert_eq!(k("ctrl+[Slash]"), "ctrl+oem_2");
        assert_eq!(k("ctrl+[KeyA]"), "ctrl+a");
        assert_eq!(k("[Digit1]"), "1");
        assert!(same_stroke(
            &parse_chord("ctrl+/").unwrap(),
            &parse_chord("ctrl+oem_2").unwrap()
        ));
    }
    #[test]
    fn invalid_expressions_are_errors() {
        for (input, reason) in [
            ("", KeyParseErrorKind::Empty),
            ("   ", KeyParseErrorKind::Empty),
            ("ctrl+", KeyParseErrorKind::MissingKey),
            ("ctrl+shift+", KeyParseErrorKind::MissingKey),
            ("ctrl+foo", KeyParseErrorKind::UnknownKey("foo".into())),
            ("hyper+x", KeyParseErrorKind::UnknownKey("hyper+x".into())),
            ("ctrl+k ctrl+", KeyParseErrorKind::MissingKey),
            (
                "ctrl+[Nope]",
                KeyParseErrorKind::UnknownKey("[nope]".into()),
            ),
            ("f25", KeyParseErrorKind::UnknownKey("f25".into())),
        ] {
            assert_eq!(
                parse_keybinding(input).unwrap_err().reason,
                reason,
                "{input}"
            );
        }
        let e: rove_base::Error = parse_keybinding("ctrl+").unwrap_err().into();
        assert_eq!(e, rove_base::Error::InvalidInput);
    }
    #[test]
    fn every_default_table_key_parses() {
        let data = include_str!("../data/default-keybindings.json");
        let rows: Vec<serde_json::Value> = serde_json::from_str(data).unwrap();
        for row in rows {
            let key = row["key"].as_str().unwrap();
            assert!(parse_keybinding(key).is_ok(), "{key}");
        }
    }
}
