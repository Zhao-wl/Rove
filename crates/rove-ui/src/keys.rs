//! GPUI keyboard events → #14 command layer.
//!
//! The core (`rove-commands`) owns parsing, `when` evaluation, rule resolution and chord state;
//! this module only converts GPUI keystrokes and keeps the per-window chord state.
//!
//! GPUI reports the printed character of the key. Rove does no layout inference (issue #14):
//! characters map to VS Code key names as on the US layout, which is what VS Code's default
//! `keyboard.dispatch: code` produces there. GPUI folds Shift into the character for punctuation
//! and digits (`ctrl+shift+-` arrives as `ctrl+_`), so those are mapped back to key + Shift.
//! Known limitation: GPUI does not distinguish numpad keys from the main-keyboard characters.
use gpui::Keystroke;
use rove_base::Result;
use rove_commands::{
    chord::{ChordState, KeyOutcome},
    context::ContextKeyService,
    keybindings::KeybindingService,
    keys::stroke,
    CommandRegistry, KeyStroke,
};
use std::{path::PathBuf, sync::Arc, time::Instant};

/// Shifted US-layout characters → (unshifted key name).
const SHIFTED: &[(&str, &str)] = &[
    ("~", "oem_3"),
    ("_", "oem_minus"),
    ("+", "oem_plus"),
    ("{", "oem_4"),
    ("}", "oem_6"),
    ("|", "oem_5"),
    (":", "oem_1"),
    ("\"", "oem_7"),
    ("<", "oem_comma"),
    (">", "oem_period"),
    ("?", "oem_2"),
    ("!", "1"),
    ("@", "2"),
    ("#", "3"),
    ("$", "4"),
    ("%", "5"),
    ("^", "6"),
    ("&", "7"),
    ("*", "8"),
    ("(", "9"),
    (")", "0"),
];

/// Convert a GPUI keystroke. Returns None for keys VS Code has no name for.
pub fn keystroke_from_gpui(k: &Keystroke) -> Option<KeyStroke> {
    let m = &k.modifiers;
    let key = k.key.as_str();
    if let Some((_, base)) = SHIFTED.iter().find(|(c, _)| *c == key) {
        return stroke(base, m.control, true, m.alt, m.platform);
    }
    let key = match key {
        "back" => "browserback",
        "forward" => "browserforward",
        "menu" => "contextmenu",
        "control" => "ctrl",
        "platform" | "super" | "win" => "meta",
        other => other,
    };
    stroke(key, m.control, m.shift, m.alt, m.platform)
}

/// Per-window keyboard input: shared keybinding service + context keys, own chord state.
pub struct KeyInput {
    pub bindings: Arc<KeybindingService>,
    pub context: Arc<ContextKeyService>,
    chord: ChordState,
}
impl KeyInput {
    pub fn new(bindings: Arc<KeybindingService>, context: Arc<ContextKeyService>) -> Self {
        Self {
            bindings,
            context,
            chord: ChordState::default(),
        }
    }
    /// Handle one key-down. `Unhandled` means the key should continue to the focused control.
    pub fn key_down(
        &mut self,
        keystroke: &Keystroke,
        commands: &CommandRegistry,
        now: Instant,
    ) -> Result<KeyOutcome> {
        let Some(key) = keystroke_from_gpui(keystroke) else {
            // An unnamed key still ends chord mode, like any unbound key.
            if self.chord.is_pending() {
                let keys = self.chord.pending().to_vec();
                self.chord.cancel();
                return Ok(KeyOutcome::Aborted(keys));
            }
            return Ok(KeyOutcome::Unhandled);
        };
        self.chord.press(
            key,
            now,
            self.bindings.as_ref(),
            self.context.as_ref(),
            commands,
        )
    }
    /// Timeout check; returns whether chord mode just ended.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.chord.tick(now)
    }
    /// Window lost focus: leave chord mode.
    pub fn cancel_chord(&mut self) -> bool {
        self.chord.cancel()
    }
    pub fn pending(&self) -> &[KeyStroke] {
        self.chord.pending()
    }
}

/// VS Code user directory holding `settings.json` / `keybindings.json` (FEATURES §13 rule 1):
/// `--vscode-user-dir <dir>` (or `--vscode-user-dir=<dir>`), then `ROVE_VSCODE_USER_DIR`, then
/// `%APPDATA%\Code\User`. Minimal stand-in until #13 provides the shared resolver.
pub fn vscode_user_dir(args: &[std::ffi::OsString]) -> Option<PathBuf> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let s = a.to_string_lossy();
        if s == "--vscode-user-dir" {
            return it.next().map(PathBuf::from);
        }
        if let Some(v) = s.strip_prefix("--vscode-user-dir=") {
            return Some(PathBuf::from(v));
        }
    }
    if let Some(dir) = std::env::var_os("ROVE_VSCODE_USER_DIR").filter(|d| !d.is_empty()) {
        return Some(dir.into());
    }
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Code").join("User"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_commands::keys::format_stroke;
    fn conv(s: &str) -> String {
        format_stroke(&keystroke_from_gpui(&Keystroke::parse(s).unwrap()).unwrap())
    }
    #[test]
    fn gpui_keystrokes_map_to_vscode_names() {
        assert_eq!(conv("ctrl-p"), "ctrl+p");
        assert_eq!(conv("ctrl-shift-p"), "ctrl+shift+p");
        assert_eq!(conv("alt-f12"), "alt+f12");
        assert_eq!(conv("ctrl-/"), "ctrl+oem_2");
        assert_eq!(conv("ctrl-\\"), "ctrl+oem_5");
        assert_eq!(conv("ctrl-,"), "ctrl+oem_comma");
        assert_eq!(conv("ctrl-_"), "ctrl+shift+oem_minus");
        assert_eq!(conv("ctrl-?"), "ctrl+shift+oem_2");
        assert_eq!(conv("ctrl-!"), "ctrl+shift+1");
        assert_eq!(conv("escape"), "escape");
        assert_eq!(conv("pageup"), "pageup");
        assert_eq!(conv("back"), "browserback");
        assert_eq!(conv("win-e"), "meta+e");
    }
    #[test]
    fn user_dir_resolution_prefers_argument() {
        let args: Vec<std::ffi::OsString> =
            vec!["rove".into(), "--vscode-user-dir".into(), "D:/x".into()];
        assert_eq!(vscode_user_dir(&args), Some(PathBuf::from("D:/x")));
        let args: Vec<std::ffi::OsString> = vec!["--vscode-user-dir=D:/y".into()];
        assert_eq!(vscode_user_dir(&args), Some(PathBuf::from("D:/y")));
    }
}
