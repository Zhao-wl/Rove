//! GPUI keyboard → #14 command layer: the one place where keys become commands.
//!
//! Every window shares one [`Keys`] global: the keybinding service (generated VS Code defaults,
//! module extras and the user's `keybindings.json`), the context key service and the command
//! table. A single keystroke interceptor ([`init`]) runs before GPUI's own keymap:
//!
//! 1. the keystroke is converted to a VS Code key ([`keystroke_from_gpui`]);
//! 2. `when` clauses are evaluated against the focused element path — every GPUI key context on
//!    that path publishes VS Code context key names (`editorTextFocus`, `listFocus`,
//!    `statusBarFocused`, `key=value` entries...) — layered over the global context keys;
//! 3. the resolved command is dispatched as the GPUI action of the module that implements it
//!    ([`action_for`]); commands without a module action go to the window as [`RunCommand`].
//!
//! If the focused path has no handler for the resolved command, the key falls through to native
//! handling (text editing in input boxes), like VS Code's fallback for `ctrl+c` in an input box.
//! While a modal dialog is open it owns the keyboard and keybindings are not evaluated.
//! Modules never call `cx.bind_keys`: default keys come from the generated table, so the user's
//! `keybindings.json` can override or remove every one of them.
//!
//! GPUI reports the printed character of the key. Rove does no layout inference (issue #14):
//! characters map to VS Code key names as on the US layout, which is what VS Code's default
//! `keyboard.dispatch: code` produces there. GPUI folds Shift into the character for punctuation
//! and digits (`ctrl+shift+-` arrives as `ctrl+_`), so those are mapped back to key + Shift.
//! Known limitation: GPUI does not distinguish numpad keys from the main-keyboard characters.
use gpui::{
    Action, App, Global, KeyContext, Keystroke, KeystrokeEvent, SharedString, Window, WindowId,
};
use rove_base::{Error, Events, NoEvents, Result, Value};
use rove_commands::{
    center::CommandCenter,
    chord::{ChordState, KeyOutcome, CHORD_TIMEOUT},
    context::ContextKeyService,
    keybindings::{default_rules, KeybindingService, RawRule},
    keys::{format_keybinding, parse_keybinding, stroke},
    ContextChange, ContextKeys, KeyStroke,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// Run a registered command that has no module-specific GPUI action (the window decides).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = rove_keys, no_json)]
pub struct RunCommand {
    pub id: SharedString,
    pub args: Value,
}

/// Chord feedback for the window's status bar.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = rove_keys, no_json)]
pub enum ChordStatus {
    /// First chord(s) pressed; waiting for the next key. Carries the keys so far.
    Pending(SharedString),
    /// The sequence pressed in chord mode is not bound.
    Aborted(SharedString),
    /// Chord mode ended without a command (timeout).
    Cleared,
}

/// The key context a modal dialog publishes; keybindings are suspended while it is focused.
pub const MODAL_CONTEXT: &str = crate::shell::catalog::CONTEXT_MODAL;

/// Context keys that are constant for the workbench (VS Code sets them for its built-in views).
const STATIC_CONTEXT: &[&str] = &["viewContainer.workbench.view.explorer.enabled"];

/// App-wide keyboard state (GPUI global).
pub struct Keys {
    pub bindings: Arc<KeybindingService>,
    pub context: Arc<ContextKeyService>,
    pub commands: Arc<CommandCenter>,
    chords: RefCell<HashMap<WindowId, (ChordState, u64)>>,
}
impl Global for Keys {}

/// Every command a window can run: VS Code IDs registered by each module, plus the Rove-only
/// commands. Registration is per module (append-friendly); a module that implements a command
/// also provides its GPUI action through [`action_for`].
pub fn default_commands() -> Result<CommandCenter> {
    let mut registry = rove_commands::CommandRegistry::default();
    crate::shell::catalog::register_commands(&mut registry)?;
    crate::code_view::commands::register(&mut registry)?;
    crate::editors::actions::register(&mut registry)?;
    crate::list_widget::registry::register_commands(&mut registry)?;
    crate::outline::registry::register_commands(&mut registry)?;
    // #24：快速打开、命令面板、转到行。
    crate::quick_access::registry::register_commands(&mut registry)?;
    let mut center = CommandCenter::default();
    for (id, command) in registry.entries() {
        center.register(id, command.clone())?;
    }
    rove_commands::builtin::register_rove_commands(&mut center)?;
    Ok(center)
}

/// Default rules contributed by modules on top of the generated VS Code table, in VS Code
/// keybindings.json syntax. Users override or remove them like any default.
fn module_defaults() -> Vec<RawRule> {
    crate::list_widget::keys::EXTRA_DEFAULTS
        .iter()
        .map(|(key, command, when)| RawRule {
            key: Some((*key).into()),
            command: (*command).into(),
            when: (!when.is_empty()).then(|| (*when).into()),
            args: Value::Null,
        })
        .collect()
}

/// The module that implements `id` provides its GPUI action; `None` for commands that only the
/// window (or nobody yet) handles.
pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    crate::shell::view::action_for(id)
        .or_else(|| crate::code_view::commands::action_for(id))
        .or_else(|| crate::editors::actions::action_for(id))
        .or_else(|| crate::list_widget::keys::action_for(id))
        .or_else(|| crate::index_status::action_for(id))
        .or_else(|| crate::outline::registry::action_for(id))
        .or_else(|| crate::quick_access::registry::action_for(id))
}

/// Whether a command can run (for menus and the command palette): it has a handler in the
/// command table or a module action. Registered-but-unimplemented commands are excluded.
pub fn is_implemented(id: &str, cx: &App) -> bool {
    action_for(id).is_some()
        || cx
            .try_global::<Keys>()
            .is_some_and(|k| k.commands.is_implemented(id))
}

/// Install the shared keyboard state and the keystroke interceptor (idempotent).
pub fn init(cx: &mut App) {
    if cx.has_global::<Keys>() {
        return;
    }
    let mut defaults: Vec<RawRule> = default_rules().into_iter().map(|d| d.rule).collect();
    defaults.extend(module_defaults());
    let context = ContextKeyService::with_platform_defaults();
    let _ = context.set_many(
        STATIC_CONTEXT
            .iter()
            .map(|k| (Arc::from(*k), Some(Value::Bool(true)))),
    );
    cx.set_global(Keys {
        bindings: Arc::new(KeybindingService::new(defaults)),
        context: Arc::new(context),
        commands: Arc::new(default_commands().expect("command registration")),
        chords: RefCell::new(HashMap::new()),
    });
    cx.intercept_keystrokes(on_keystroke).detach();
}

pub fn keys(cx: &App) -> &Keys {
    cx.global::<Keys>()
}

/// Use the `keybindings.json` shared with VS Code: loaded now, then reloaded in the background
/// when its timestamp or size changes. A broken file keeps the previous table.
pub fn watch_user_file(path: PathBuf, cx: &mut App) {
    init(cx);
    let service = keys(cx).bindings.clone();
    let executor = cx.background_executor().clone();
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = service.watch_user_file(path) {
                eprintln!("keybindings.json: {error}");
            }
            loop {
                executor.timer(Duration::from_secs(1)).await;
                if let Err(error) = service.reload_if_changed() {
                    eprintln!("keybindings.json: {error}");
                }
            }
        })
        .detach();
}

/// `keybindings.json` in the VS Code user directory (`rove_config::paths`, #13).
pub fn user_keybindings_path(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Option<PathBuf> {
    let (arg, _) = rove_config::paths::split_user_dir_arg(args);
    rove_config::paths::resolve_user_dir(arg, |k| std::env::var_os(k))
        .map(|dir| dir.join(rove_config::paths::KEYBINDINGS_FILE))
}

/// `when` context for one keystroke: the focused path's GPUI key contexts (innermost wins),
/// then the global context keys. A bare identifier reads as `true`, `key=value` as the string.
pub struct FocusContext<'a> {
    pub stack: &'a [KeyContext],
    pub base: &'a dyn ContextKeys,
}
impl ContextKeys for FocusContext<'_> {
    fn get(&self, key: &str) -> Option<Value> {
        for context in self.stack.iter().rev() {
            if let Some(value) = context.get(key) {
                return Some(Value::String(value.as_ref().into()));
            }
            if context.contains(key) {
                return Some(Value::Bool(true));
            }
        }
        self.base.get(key)
    }
    fn set(&self, _: Arc<str>, _: Option<Value>) -> Result<()> {
        Err(Error::Unsupported)
    }
    fn changes(&self) -> &dyn Events<ContextChange> {
        &NoEvents
    }
}

fn on_keystroke(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) {
    let Some(keys) = cx.try_global::<Keys>() else {
        return;
    };
    if event
        .context_stack
        .iter()
        .any(|c| c.contains(MODAL_CONTEXT))
    {
        return;
    }
    let window_id = window.window_handle().window_id();
    let (bindings, base, commands) = (
        keys.bindings.clone(),
        keys.context.clone(),
        keys.commands.clone(),
    );
    let context = FocusContext {
        stack: &event.context_stack,
        base: base.as_ref(),
    };
    let (outcome, generation, in_chord) = {
        let mut chords = keys.chords.borrow_mut();
        let (state, generation) = chords.entry(window_id).or_default();
        let in_chord = state.is_pending();
        let outcome = match keystroke_from_gpui(&event.keystroke) {
            Some(key) => state.press(
                key,
                Instant::now(),
                bindings.as_ref(),
                &context,
                commands.registry(),
            ),
            // A key VS Code has no name for still ends chord mode, like any unbound key.
            None if state.is_pending() => {
                let pending = state.pending().to_vec();
                state.cancel();
                Ok(KeyOutcome::Aborted(pending))
            }
            None => Ok(KeyOutcome::Unhandled),
        };
        *generation += 1;
        (outcome, *generation, in_chord)
    };
    match outcome {
        Ok(KeyOutcome::Unhandled) | Err(_) => {}
        Ok(KeyOutcome::Command(invocation)) => {
            let action = action_for(&invocation.id).unwrap_or_else(|| {
                Box::new(RunCommand {
                    id: invocation.id.to_string().into(),
                    args: invocation.args.clone(),
                })
            });
            if !in_chord && !window.is_action_available(action.as_ref(), cx) {
                // Nothing on the focused path implements it: native handling. (The last key
                // of a chord is always consumed.)
                return;
            }
            window.dispatch_action(action, cx);
            cx.stop_propagation();
        }
        Ok(KeyOutcome::Pending(sequence)) => {
            window.dispatch_action(
                Box::new(ChordStatus::Pending(format_keybinding(&sequence).into())),
                cx,
            );
            cx.stop_propagation();
            // VS Code leaves chord mode after 5 seconds without another key.
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(CHORD_TIMEOUT).await;
                    let _ = cx.update(|window, cx| {
                        let ended = cx.try_global::<Keys>().is_some_and(|keys| {
                            let mut chords = keys.chords.borrow_mut();
                            match chords.get_mut(&window_id) {
                                Some((state, current)) if *current == generation => state.cancel(),
                                _ => false,
                            }
                        });
                        if ended {
                            window.dispatch_action(Box::new(ChordStatus::Cleared), cx);
                        }
                    });
                })
                .detach();
        }
        Ok(KeyOutcome::Aborted(sequence)) => {
            window.dispatch_action(
                Box::new(ChordStatus::Aborted(format_keybinding(&sequence).into())),
                cx,
            );
            cx.stop_propagation();
        }
    }
}

/// Pending chord keys of a window (empty outside chord mode).
pub fn pending_chord(window: &Window, cx: &App) -> Vec<KeyStroke> {
    cx.try_global::<Keys>()
        .and_then(|k| {
            k.chords
                .borrow()
                .get(&window.window_handle().window_id())
                .map(|(s, _)| s.pending().to_vec())
        })
        .unwrap_or_default()
}

/// Key hint shown in menus and tooltips (`Ctrl+Shift+M`), from the effective table — user
/// overrides included. Prefers the last rule without a `when`, like VS Code's primary keybinding.
pub fn key_label(id: &str, cx: &App) -> Option<String> {
    let table = match cx.try_global::<Keys>() {
        Some(k) => k.bindings.table(),
        None => builtin_table(),
    };
    let entries: Vec<_> = table.entries().filter(|(_, c, _)| *c == id).collect();
    let (keys, _, _) = entries
        .iter()
        .rev()
        .find(|(_, _, when)| when.is_none())
        .or_else(|| entries.last())?;
    Some(label(keys))
}

fn builtin_table() -> Arc<rove_commands::keybindings::KeybindingTable> {
    static TABLE: std::sync::OnceLock<Arc<rove_commands::keybindings::KeybindingTable>> =
        std::sync::OnceLock::new();
    TABLE
        .get_or_init(|| {
            let mut defaults: Vec<RawRule> = default_rules().into_iter().map(|d| d.rule).collect();
            defaults.extend(module_defaults());
            KeybindingService::new(defaults).table()
        })
        .clone()
}

/// VS Code display form of a key sequence: `Ctrl+K Ctrl+Shift+N`, `Ctrl+=`, `Ctrl+NumPad0`.
pub fn label(keys: &[KeyStroke]) -> String {
    keys.iter()
        .map(|k| {
            let mut parts = Vec::new();
            for (on, name) in [
                (k.ctrl, "Ctrl"),
                (k.shift, "Shift"),
                (k.alt, "Alt"),
                (k.meta, "Win"),
            ] {
                if on {
                    parts.push(name.to_owned());
                }
            }
            parts.push(key_display(&k.key));
            parts.join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn key_display(key: &str) -> String {
    let fixed = match key {
        "oem_1" => ";",
        "oem_plus" => "=",
        "oem_comma" => ",",
        "oem_minus" => "-",
        "oem_period" => ".",
        "oem_2" => "/",
        "oem_3" => "`",
        "oem_4" => "[",
        "oem_5" => "\\",
        "oem_6" => "]",
        "oem_7" => "'",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "numpad_add" => "NumPad_Add",
        "numpad_subtract" => "NumPad_Subtract",
        "numpad_multiply" => "NumPad_Multiply",
        "numpad_divide" => "NumPad_Divide",
        "numpad_decimal" => "NumPad_Decimal",
        "up" => "UpArrow",
        "down" => "DownArrow",
        "left" => "LeftArrow",
        "right" => "RightArrow",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_owned();
    }
    if let Some(n) = key.strip_prefix("numpad") {
        return format!("NumPad{n}");
    }
    let mut chars = key.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// Parse a VS Code key expression into GPUI keystroke text for `simulate_keystrokes`
/// (`ctrl+k ctrl+oem_5` → `ctrl-k ctrl-\`). Test and tooling helper.
pub fn gpui_keystrokes(vscode: &str) -> Option<String> {
    let keys = parse_keybinding(vscode).ok()?;
    Some(
        keys.iter()
            .map(|k| {
                let mut s = String::new();
                for (on, m) in [
                    (k.ctrl, "ctrl-"),
                    (k.alt, "alt-"),
                    (k.shift, "shift-"),
                    (k.meta, "win-"),
                ] {
                    if on {
                        s.push_str(m);
                    }
                }
                let key = key_display(&k.key);
                let key = match k.key.as_ref() {
                    "pageup" | "pagedown" | "up" | "down" | "left" | "right" => k.key.to_string(),
                    _ if key.chars().count() == 1 => key,
                    _ => k.key.to_string(),
                };
                s.push_str(&key.to_lowercase());
                s
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Shifted US-layout characters → unshifted key name.
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
    }
    #[test]
    fn labels_follow_vscode_style() {
        let l = |s: &str| label(&parse_keybinding(s).unwrap());
        assert_eq!(l("ctrl+b"), "Ctrl+B");
        assert_eq!(l("ctrl+k ctrl+shift+n"), "Ctrl+K Ctrl+Shift+N");
        assert_eq!(l("ctrl+oem_plus"), "Ctrl+=");
        assert_eq!(l("ctrl+numpad0"), "Ctrl+NumPad0");
        assert_eq!(l("f6"), "F6");
        assert_eq!(
            gpui_keystrokes("ctrl+k ctrl+oem_5").as_deref(),
            Some("ctrl-k ctrl-\\")
        );
        assert_eq!(
            gpui_keystrokes("shift+alt+0").as_deref(),
            Some("alt-shift-0")
        );
        assert_eq!(
            gpui_keystrokes("ctrl+pagedown").as_deref(),
            Some("ctrl-pagedown")
        );
    }
    /// Keys reach commands only through the keybinding service: no module installs its own
    /// GPUI keymap (that would bypass `when`, user overrides and `-command` removals).
    #[test]
    fn no_module_binds_keys_directly() {
        fn visit(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, hits);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for needle in [concat!("bind_", "keys("), concat!("KeyBinding", "::new(")] {
                        if text.contains(needle) {
                            hits.push(format!("{} uses {needle}", path.display()));
                        }
                    }
                }
            }
        }
        let mut hits = Vec::new();
        visit(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut hits,
        );
        assert!(hits.is_empty(), "{hits:#?}");
    }

    #[test]
    fn every_registered_command_with_a_module_action_is_known() {
        let center = default_commands().unwrap();
        // Rove-only commands are registered but not implemented yet.
        assert!(center.is_registered("rove.contextBasket.add"));
        for id in [
            "workbench.action.toggleSidebarVisibility",
            "cursorDown",
            "workbench.action.splitEditor",
            "list.focusDown",
            "quickInput.accept",
        ] {
            assert!(center.is_registered(id), "{id}");
            assert!(action_for(id).is_some(), "{id}");
        }
    }
}
