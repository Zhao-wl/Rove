//! Keybinding table and resolver (#14), following VS Code's `KeybindingResolver`.
//!
//! * The default table is generated from VS Code's export (`data/default-keybindings.json`,
//!   produced by `scripts/compat/gen_default_keybindings.py`) and embedded in the binary.
//! * User rules come from the `keybindings.json` shared with VS Code (JSONC). They are appended
//!   after the defaults, so they win (later definitions take precedence).
//! * `"command": "-id"` removes default rules for `id`; if the removal names a `key` and/or a
//!   `when`, only defaults with the same key / equivalent `when` are removed.
//! * Resolution walks candidates from the last definition to the first and takes the first rule
//!   whose `when` holds and whose command is registered; rules for unknown commands are ignored
//!   without any user-visible error (FEATURES §13 rule 7). If that rule needs more chords, the
//!   result is `PendingChord`.
//! * Invalid user entries (bad key, bad `when`, wrong shape) are skipped and reported only as
//!   diagnostics for the log. A file that is not valid JSONC keeps the last valid table.
use crate::{
    keys::{format_keybinding, format_stroke, parse_keybinding},
    when::WhenExpr,
    CommandRegistry, ContextKeys, Invocation, KeyResolution, KeyStroke, Keybindings, When,
};
use rove_base::{Error, Result, Value};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::SystemTime,
};

/// The generated default table, embedded at compile time.
pub const DEFAULT_KEYBINDINGS_JSON: &str = include_str!("../data/default-keybindings.json");

/// Rollout priority from `docs/compat/KEYBINDINGS.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    P0,
    P1,
    P2,
}

/// One rule as written in a keybindings file (before validation).
#[derive(Clone, Debug)]
pub struct RawRule {
    pub key: Option<String>,
    pub command: String,
    pub when: Option<String>,
    pub args: Value,
}

/// One rule of the default table.
#[derive(Clone, Debug)]
pub struct DefaultRule {
    pub rule: RawRule,
    pub priority: Priority,
    /// `vscode` (export), `supplement` (manual addition) or `rove` (Rove-only command).
    pub source: Arc<str>,
}

/// Why an entry of a keybindings file was skipped. For logs; never shown as an error dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub index: usize,
    pub reason: String,
}

#[derive(Clone, Debug)]
struct Rule {
    keys: Arc<[KeyStroke]>,
    ids: Vec<String>,
    command: Arc<str>,
    args: Value,
    when: Option<When>,
    /// `Err` when the clause failed to parse: such a rule can never activate.
    prepared: std::result::Result<Option<Arc<WhenExpr>>, ()>,
    is_default: bool,
}

/// An immutable resolved table: default rules minus removals, then user rules.
#[derive(Clone, Debug, Default)]
pub struct KeybindingTable {
    rules: Vec<Rule>,
    by_first: HashMap<String, Vec<usize>>,
}

impl KeybindingTable {
    /// Build a table. `defaults` and `user` are in definition order.
    pub fn build(defaults: &[RawRule], user: &[RawRule]) -> (Self, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let mut removals: Vec<(String, Option<Vec<String>>, Option<String>)> = Vec::new();
        let mut user_rules = Vec::new();
        for (index, raw) in user.iter().enumerate() {
            let mut skip = |reason: String| diagnostics.push(Diagnostic { index, reason });
            let keys = match raw.key.as_deref().map(parse_keybinding) {
                Some(Ok(k)) => Some(k),
                Some(Err(e)) => {
                    skip(e.to_string());
                    continue;
                }
                None => None,
            };
            if let Some(command) = raw.command.strip_prefix('-') {
                let when = match raw.when.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
                    Some(w) => match WhenExpr::parse(w) {
                        Ok(e) => Some(e.canonical()),
                        Err(_) => {
                            skip(format!("invalid when `{w}`"));
                            continue;
                        }
                    },
                    None => None,
                };
                let ids = keys.map(|k| k.iter().map(format_stroke).collect());
                removals.push((command.to_owned(), ids, when));
                continue;
            }
            let Some(keys) = keys else {
                skip("missing key".into());
                continue;
            };
            if raw.command.is_empty() {
                skip("missing command".into());
                continue;
            }
            let rule = make_rule(keys, raw, false);
            if rule.prepared.is_err() {
                skip(format!(
                    "invalid when `{}`",
                    raw.when.as_deref().unwrap_or("")
                ));
            }
            user_rules.push(rule);
        }
        let mut rules = Vec::with_capacity(defaults.len() + user_rules.len());
        for raw in defaults {
            let Some(Ok(keys)) = raw.key.as_deref().map(parse_keybinding) else {
                continue;
            };
            let rule = make_rule(keys, raw, true);
            if removals.iter().any(|r| is_removed(&rule, r)) {
                continue;
            }
            rules.push(rule);
        }
        rules.extend(user_rules);
        let mut by_first: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in rules.iter().enumerate() {
            by_first.entry(r.ids[0].clone()).or_default().push(i);
        }
        (Self { rules, by_first }, diagnostics)
    }

    /// Number of active rules (after removals).
    pub fn len(&self) -> usize {
        self.rules.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// VS Code `KeybindingResolver.resolve`: `pressed` is the full sequence so far, including the
    /// key just pressed.
    pub fn resolve(
        &self,
        pressed: &[KeyStroke],
        context: &dyn ContextKeys,
        commands: &CommandRegistry,
    ) -> KeyResolution {
        let Some(first) = pressed.first() else {
            return KeyResolution::NoMatch;
        };
        let Some(candidates) = self.by_first.get(&format_stroke(first)) else {
            return KeyResolution::NoMatch;
        };
        let pressed_ids: Vec<String> = pressed.iter().map(format_stroke).collect();
        for &i in candidates.iter().rev() {
            let rule = &self.rules[i];
            if rule.ids.len() < pressed_ids.len()
                || rule.ids[..pressed_ids.len()] != pressed_ids[..]
            {
                continue;
            }
            if commands.get(&rule.command).is_none() {
                continue;
            }
            let active = match &rule.prepared {
                Ok(None) => true,
                Ok(Some(expr)) => expr.evaluate(context),
                Err(()) => false,
            };
            if !active {
                continue;
            }
            return if pressed_ids.len() < rule.ids.len() {
                KeyResolution::PendingChord
            } else {
                KeyResolution::Command(Invocation {
                    id: rule.command.clone(),
                    args: rule.args.clone(),
                })
            };
        }
        KeyResolution::NoMatch
    }

    /// Active rules in definition order: `(keys, command, when)`. For key hints (#24).
    pub fn entries(&self) -> impl Iterator<Item = (&[KeyStroke], &str, Option<&When>)> {
        self.rules
            .iter()
            .map(|r| (&r.keys[..], r.command.as_ref(), r.when.as_ref()))
    }

    /// Rules bound to `command`, as canonical key text, in definition order.
    pub fn keys_for(&self, command: &str) -> Vec<String> {
        self.entries()
            .filter(|(_, c, _)| *c == command)
            .map(|(k, _, _)| format_keybinding(k))
            .collect()
    }
}

fn make_rule(keys: Vec<KeyStroke>, raw: &RawRule, is_default: bool) -> Rule {
    let when_text = raw.when.as_deref().map(str::trim).filter(|w| !w.is_empty());
    let prepared = match when_text {
        None => Ok(None),
        Some(w) => WhenExpr::parse(w)
            .map(|e| Some(Arc::new(e)))
            .map_err(|_| ()),
    };
    Rule {
        ids: keys.iter().map(format_stroke).collect(),
        keys: keys.into(),
        command: raw.command.as_str().into(),
        args: raw.args.clone(),
        when: when_text.map(|w| When(w.into())),
        prepared,
        is_default,
    }
}

/// VS Code `_isTargetedForRemoval`.
fn is_removed(
    rule: &Rule,
    (command, keys, when): &(String, Option<Vec<String>>, Option<String>),
) -> bool {
    if !rule.is_default || rule.command.as_ref() != command {
        return false;
    }
    if let Some(keys) = keys {
        if *keys != rule.ids {
            return false;
        }
    }
    if let Some(when) = when {
        if when == "true" {
            return true;
        }
        match &rule.prepared {
            Ok(Some(expr)) => expr.canonical() == *when,
            _ => false,
        }
    } else {
        true
    }
}

// ---------------------------------------------------------------- file formats

fn to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => Value::String(s.as_str().into()),
        serde_json::Value::Array(a) => Value::Array(a.iter().map(to_value).collect()),
        serde_json::Value::Object(o) => Value::Object(Arc::new(
            o.iter()
                .map(|(k, v)| (k.clone(), to_value(v)))
                .collect::<BTreeMap<_, _>>(),
        )),
    }
}

/// Parse a `keybindings.json` (JSONC: comments and trailing commas allowed). An empty file means
/// no rules. Not-JSONC or not-an-array => `InvalidInput` (caller keeps its previous table).
/// Entries with the wrong shape are skipped with a diagnostic.
pub fn parse_user_keybindings(text: &str) -> Result<(Vec<RawRule>, Vec<Diagnostic>)> {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let value: Option<serde_json::Value> =
        jsonc_parser::parse_to_serde_value(text, &jsonc_parser::ParseOptions::default())
            .map_err(|_| Error::InvalidInput)?;
    let items = match value {
        None => return Ok((Vec::new(), Vec::new())),
        Some(serde_json::Value::Array(items)) => items,
        Some(_) => return Err(Error::InvalidInput),
    };
    let mut rules = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            diagnostics.push(Diagnostic {
                index,
                reason: "entry is not an object".into(),
            });
            continue;
        };
        let Some(command) = obj.get("command").and_then(|c| c.as_str()) else {
            diagnostics.push(Diagnostic {
                index,
                reason: "missing command".into(),
            });
            continue;
        };
        let key = match obj.get("key") {
            Some(serde_json::Value::String(k)) => Some(k.clone()),
            None => None,
            Some(_) => {
                diagnostics.push(Diagnostic {
                    index,
                    reason: "key is not a string".into(),
                });
                continue;
            }
        };
        let when = match obj.get("when") {
            Some(serde_json::Value::String(w)) => Some(w.clone()),
            None | Some(serde_json::Value::Null) => None,
            Some(_) => {
                diagnostics.push(Diagnostic {
                    index,
                    reason: "when is not a string".into(),
                });
                continue;
            }
        };
        rules.push(RawRule {
            key,
            command: command.to_owned(),
            when,
            args: obj.get("args").map_or(Value::Null, to_value),
        });
    }
    Ok((rules, diagnostics))
}

/// Parse the embedded default table.
pub fn default_rules() -> Vec<DefaultRule> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(DEFAULT_KEYBINDINGS_JSON)
        .expect("generated default table is valid JSON");
    rows.iter()
        .map(|row| DefaultRule {
            rule: RawRule {
                key: row["key"].as_str().map(str::to_owned),
                command: row["command"].as_str().unwrap_or_default().to_owned(),
                when: row["when"].as_str().map(str::to_owned),
                args: row.get("args").map_or(Value::Null, to_value),
            },
            priority: match row["priority"].as_str() {
                Some("P0") => Priority::P0,
                Some("P1") => Priority::P1,
                _ => Priority::P2,
            },
            source: row["source"].as_str().unwrap_or("vscode").into(),
        })
        .collect()
}

// ---------------------------------------------------------------- service

#[derive(Clone, Debug, PartialEq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

/// The keybinding service: default rules + the user's `keybindings.json`, hot-reloadable.
/// `resolve` reads an `Arc` snapshot (UI-safe); reloading parses off the UI thread and swaps the
/// table atomically. A failed reload keeps the previous table.
pub struct KeybindingService {
    defaults: Vec<RawRule>,
    table: RwLock<Arc<KeybindingTable>>,
    user_file: Mutex<Option<(PathBuf, Option<FileStamp>)>>,
    diagnostics: Mutex<Vec<Diagnostic>>,
}
impl KeybindingService {
    /// Service with the given default rules and no user rules.
    pub fn new(defaults: Vec<RawRule>) -> Self {
        let (table, _) = KeybindingTable::build(&defaults, &[]);
        Self {
            defaults,
            table: RwLock::new(Arc::new(table)),
            user_file: Mutex::new(None),
            diagnostics: Mutex::new(Vec::new()),
        }
    }
    /// Service with the embedded default table.
    pub fn with_builtin_defaults() -> Self {
        Self::new(default_rules().into_iter().map(|d| d.rule).collect())
    }
    pub fn table(&self) -> Arc<KeybindingTable> {
        self.table.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// Diagnostics of the last successful user load.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    /// Replace user rules from file text. Invalid JSONC => `InvalidInput`, previous table kept.
    pub fn set_user_text(&self, text: &str) -> Result<()> {
        let (user, mut diagnostics) = parse_user_keybindings(text)?;
        let (table, more) = KeybindingTable::build(&self.defaults, &user);
        diagnostics.extend(more);
        *self.table.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(table);
        *self.diagnostics.lock().unwrap_or_else(|e| e.into_inner()) = diagnostics;
        Ok(())
    }
    /// Use `path` as the user keybindings file and load it now. A missing file means no user
    /// rules (it may be created later and is picked up by `reload_if_changed`).
    pub fn watch_user_file(&self, path: impl Into<PathBuf>) -> Result<()> {
        *self.user_file.lock().unwrap_or_else(|e| e.into_inner()) = Some((path.into(), None));
        self.reload_if_changed().map(|_| ())
    }
    /// Reload the user file if its timestamp or size changed. Blocking file I/O: call from a
    /// background thread. Returns whether the table was replaced.
    pub fn reload_if_changed(&self) -> Result<bool> {
        let mut guard = self.user_file.lock().unwrap_or_else(|e| e.into_inner());
        let Some((path, last)) = guard.as_mut() else {
            return Ok(false);
        };
        let stamp = stamp(path);
        if last.is_some() && *last == stamp {
            return Ok(false);
        }
        let text = match &stamp {
            None => String::new(),
            Some(_) => match std::fs::read(&*path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(e.into()),
            },
        };
        // Record the stamp even when parsing fails, so a broken file is not re-parsed every poll.
        *last = stamp;
        drop(guard);
        self.set_user_text(&text)?;
        Ok(true)
    }
    pub fn user_file(&self) -> Option<PathBuf> {
        self.user_file
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|(p, _)| p.clone())
    }
}
fn stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}
impl Keybindings for KeybindingService {
    fn resolve(
        &self,
        prefix: &[KeyStroke],
        context: &dyn ContextKeys,
        commands: &CommandRegistry,
    ) -> Result<KeyResolution> {
        Ok(self.table().resolve(prefix, context, commands))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::ContextKeyService, keys::parse_keybinding, Command, ContextSnapshot};

    fn raw(key: &str, command: &str, when: Option<&str>) -> RawRule {
        RawRule {
            key: (!key.is_empty()).then(|| key.to_owned()),
            command: command.to_owned(),
            when: when.map(str::to_owned),
            args: Value::Null,
        }
    }
    fn registry(ids: &[&str]) -> CommandRegistry {
        let mut r = CommandRegistry::default();
        for id in ids {
            r.register(
                *id,
                Command {
                    title_key: "t".into(),
                    category_key: None,
                },
            )
            .unwrap();
        }
        r
    }
    fn ctx(keys: &[&str]) -> ContextSnapshot {
        keys.iter()
            .map(|k| (Arc::from(*k), Value::Bool(true)))
            .collect()
    }
    fn hit(
        t: &KeybindingTable,
        keys: &str,
        c: &ContextSnapshot,
        r: &CommandRegistry,
    ) -> Option<String> {
        match t.resolve(&parse_keybinding(keys).unwrap(), c, r) {
            KeyResolution::Command(i) => Some(i.id.to_string()),
            KeyResolution::PendingChord => Some("<pending>".into()),
            KeyResolution::NoMatch => None,
        }
    }

    #[test]
    fn later_definition_wins_and_when_falls_through() {
        let defaults = [
            raw(
                "f12",
                "editor.action.revealDefinition",
                Some("editorTextFocus"),
            ),
            raw("f12", "goToNextReference", Some("referenceSearchVisible")),
        ];
        let (t, _) = KeybindingTable::build(&defaults, &[]);
        let r = registry(&["editor.action.revealDefinition", "goToNextReference"]);
        assert_eq!(
            hit(&t, "f12", &ctx(&["editorTextFocus"]), &r).as_deref(),
            Some("editor.action.revealDefinition")
        );
        // Both match: the later rule wins.
        assert_eq!(
            hit(
                &t,
                "f12",
                &ctx(&["editorTextFocus", "referenceSearchVisible"]),
                &r
            )
            .as_deref(),
            Some("goToNextReference")
        );
        assert_eq!(hit(&t, "f12", &ctx(&[]), &r), None);
    }

    #[test]
    fn user_overrides_default() {
        let defaults = [raw("ctrl+p", "workbench.action.quickOpen", None)];
        let user = [raw("ctrl+p", "workbench.action.showCommands", None)];
        let (t, _) = KeybindingTable::build(&defaults, &user);
        let r = registry(&[
            "workbench.action.quickOpen",
            "workbench.action.showCommands",
        ]);
        assert_eq!(
            hit(&t, "ctrl+p", &ctx(&[]), &r).as_deref(),
            Some("workbench.action.showCommands")
        );
        // A user rule whose when is false falls through to the default.
        let user = [raw(
            "ctrl+p",
            "workbench.action.showCommands",
            Some("inQuickOpen"),
        )];
        let (t, _) = KeybindingTable::build(&defaults, &user);
        assert_eq!(
            hit(&t, "ctrl+p", &ctx(&[]), &r).as_deref(),
            Some("workbench.action.quickOpen")
        );
        assert_eq!(
            hit(&t, "ctrl+p", &ctx(&["inQuickOpen"]), &r).as_deref(),
            Some("workbench.action.showCommands")
        );
    }

    #[test]
    fn minus_command_removes_defaults() {
        let defaults = [
            raw("ctrl+p", "workbench.action.quickOpen", None),
            raw("ctrl+e", "workbench.action.quickOpen", None),
            raw(
                "f12",
                "editor.action.revealDefinition",
                Some("editorHasDefinitionProvider && editorTextFocus"),
            ),
            raw(
                "ctrl+f12",
                "editor.action.revealDefinition",
                Some("editorTextFocus && isWeb"),
            ),
        ];
        let r = registry(&[
            "workbench.action.quickOpen",
            "editor.action.revealDefinition",
        ]);
        let c = ctx(&["editorHasDefinitionProvider", "editorTextFocus", "isWeb"]);
        // Removal by command only: every default for that command.
        let (t, _) =
            KeybindingTable::build(&defaults, &[raw("", "-workbench.action.quickOpen", None)]);
        assert_eq!(hit(&t, "ctrl+p", &c, &r), None);
        assert_eq!(hit(&t, "ctrl+e", &c, &r), None);
        // Removal by command + key: only that key.
        let (t, _) = KeybindingTable::build(
            &defaults,
            &[raw("ctrl+p", "-workbench.action.quickOpen", None)],
        );
        assert_eq!(hit(&t, "ctrl+p", &c, &r), None);
        assert!(hit(&t, "ctrl+e", &c, &r).is_some());
        // Removal by command + equivalent when (operand order does not matter).
        let (t, _) = KeybindingTable::build(
            &defaults,
            &[raw(
                "",
                "-editor.action.revealDefinition",
                Some("editorTextFocus && editorHasDefinitionProvider"),
            )],
        );
        assert_eq!(hit(&t, "f12", &c, &r), None);
        assert!(hit(&t, "ctrl+f12", &c, &r).is_some());
        // A different when removes nothing.
        let (t, _) = KeybindingTable::build(
            &defaults,
            &[raw("f12", "-editor.action.revealDefinition", Some("isWeb"))],
        );
        assert!(hit(&t, "f12", &c, &r).is_some());
        // Removal plus a new user binding = rebinding.
        let (t, _) = KeybindingTable::build(
            &defaults,
            &[
                raw("ctrl+p", "-workbench.action.quickOpen", None),
                raw("ctrl+shift+o", "workbench.action.quickOpen", None),
            ],
        );
        assert_eq!(hit(&t, "ctrl+p", &c, &r), None);
        assert_eq!(
            hit(&t, "ctrl+shift+o", &c, &r).as_deref(),
            Some("workbench.action.quickOpen")
        );
        // Removals never touch user rules.
        let (t, _) = KeybindingTable::build(
            &[],
            &[
                raw("ctrl+p", "workbench.action.quickOpen", None),
                raw("", "-workbench.action.quickOpen", None),
            ],
        );
        assert!(hit(&t, "ctrl+p", &c, &r).is_some());
    }

    #[test]
    fn unknown_commands_are_ignored() {
        let defaults = [raw("f5", "workbench.action.quickOpen", None)];
        let user = [raw("f5", "extension.doesNotExist", None)];
        let (t, diagnostics) = KeybindingTable::build(&defaults, &user);
        assert!(
            diagnostics.is_empty(),
            "unknown commands are not diagnostics"
        );
        let r = registry(&["workbench.action.quickOpen"]);
        assert_eq!(
            hit(&t, "f5", &ctx(&[]), &r).as_deref(),
            Some("workbench.action.quickOpen")
        );
        assert_eq!(hit(&t, "f5", &ctx(&[]), &registry(&[])), None);
    }

    #[test]
    fn chords_resolve_to_pending_then_command() {
        let defaults = [
            raw(
                "ctrl+k ctrl+s",
                "workbench.action.openGlobalKeybindings",
                None,
            ),
            raw(
                "ctrl+k b",
                "rove.contextBasket.add",
                Some("editorTextFocus"),
            ),
            raw("ctrl+k", "single.k", Some("never")),
        ];
        let (t, _) = KeybindingTable::build(&defaults, &[]);
        let r = registry(&[
            "workbench.action.openGlobalKeybindings",
            "rove.contextBasket.add",
            "single.k",
        ]);
        let c = ctx(&[]);
        assert_eq!(hit(&t, "ctrl+k", &c, &r).as_deref(), Some("<pending>"));
        assert_eq!(
            hit(&t, "ctrl+k ctrl+s", &c, &r).as_deref(),
            Some("workbench.action.openGlobalKeybindings")
        );
        assert_eq!(hit(&t, "ctrl+k b", &c, &r), None, "when not satisfied");
        assert_eq!(
            hit(&t, "ctrl+k b", &ctx(&["editorTextFocus"]), &r).as_deref(),
            Some("rove.contextBasket.add")
        );
        assert_eq!(hit(&t, "ctrl+k x", &c, &r), None);
    }

    #[test]
    fn args_are_passed_through() {
        let (rules, _) = parse_user_keybindings(
            r#"[{ "key": "end", "command": "cursorEnd", "args": { "sticky": false } }]"#,
        )
        .unwrap();
        let (t, _) = KeybindingTable::build(&[], &rules);
        let KeyResolution::Command(i) = t.resolve(
            &parse_keybinding("end").unwrap(),
            &ctx(&[]),
            &registry(&["cursorEnd"]),
        ) else {
            panic!()
        };
        let Value::Object(o) = i.args else { panic!() };
        assert_eq!(o.get("sticky"), Some(&Value::Bool(false)));
    }

    #[test]
    fn jsonc_user_file_and_invalid_entries() {
        let text = "\u{feff}// comment\n[\n  { \"key\": \"ctrl+shift+alt+o\", \"command\": \"workbench.action.quickOpen\" }, /* block */\n  { \"key\": \"ctrl+\", \"command\": \"a\" },\n  { \"key\": \"f1\", \"command\": \"b\", \"when\": \"(\" },\n  { \"key\": 3, \"command\": \"c\" },\n  { \"command\": \"d\" },\n  \"nope\",\n  { \"key\": \"f2\" },\n]\n";
        let (rules, mut diagnostics) = parse_user_keybindings(text).unwrap();
        let (t, more) = KeybindingTable::build(&[], &rules);
        diagnostics.extend(more);
        assert_eq!(
            t.len(),
            2,
            "valid rule + rule with broken when (never active)"
        );
        assert_eq!(diagnostics.len(), 6, "{diagnostics:?}");
        let r = registry(&["workbench.action.quickOpen", "b"]);
        assert!(hit(&t, "ctrl+shift+alt+o", &ctx(&[]), &r).is_some());
        assert_eq!(
            hit(&t, "f1", &ctx(&[]), &r),
            None,
            "invalid when never activates"
        );
        assert!(parse_user_keybindings("").unwrap().0.is_empty());
        assert!(parse_user_keybindings("// only a comment")
            .unwrap()
            .0
            .is_empty());
        assert_eq!(
            parse_user_keybindings("{}").unwrap_err(),
            Error::InvalidInput
        );
        assert_eq!(
            parse_user_keybindings("[ { ").unwrap_err(),
            Error::InvalidInput
        );
    }

    #[test]
    fn service_keeps_last_valid_table_and_hot_reloads() {
        let dir = std::env::temp_dir().join(format!("rove-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keybindings.json");
        let _ = std::fs::remove_file(&path);
        let s = KeybindingService::new(vec![raw("ctrl+p", "workbench.action.quickOpen", None)]);
        let r = registry(&[
            "workbench.action.quickOpen",
            "workbench.action.showCommands",
        ]);
        let c = ContextKeyService::default();
        let resolve = |k: &str| match s.resolve(&parse_keybinding(k).unwrap(), &c, &r).unwrap() {
            KeyResolution::Command(i) => Some(i.id.to_string()),
            _ => None,
        };
        // Missing file: defaults only.
        s.watch_user_file(&path).unwrap();
        assert_eq!(
            resolve("ctrl+p").as_deref(),
            Some("workbench.action.quickOpen")
        );
        std::fs::write(
            &path,
            r#"[{"key":"ctrl+p","command":"workbench.action.showCommands"}]"#,
        )
        .unwrap();
        assert!(s.reload_if_changed().unwrap());
        assert_eq!(
            resolve("ctrl+p").as_deref(),
            Some("workbench.action.showCommands")
        );
        assert!(
            !s.reload_if_changed().unwrap(),
            "unchanged file is not re-read"
        );
        // Broken file: error, last valid table stays.
        std::fs::write(&path, "[ {").unwrap();
        assert_eq!(s.reload_if_changed(), Err(Error::InvalidInput));
        assert_eq!(
            resolve("ctrl+p").as_deref(),
            Some("workbench.action.showCommands")
        );
        std::fs::remove_file(&path).unwrap();
        assert!(s.reload_if_changed().unwrap());
        assert_eq!(
            resolve("ctrl+p").as_deref(),
            Some("workbench.action.quickOpen")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builtin_defaults_resolve_vscode_keys() {
        let s = KeybindingService::with_builtin_defaults();
        let r = registry(&[
            "workbench.action.quickOpen",
            "workbench.action.showCommands",
            "editor.action.revealDefinition",
        ]);
        let c = ContextKeyService::with_platform_defaults();
        let id = |k: &str| match s.resolve(&parse_keybinding(k).unwrap(), &c, &r).unwrap() {
            KeyResolution::Command(i) => Some(i.id.to_string()),
            _ => None,
        };
        assert_eq!(id("ctrl+p").as_deref(), Some("workbench.action.quickOpen"));
        assert_eq!(
            id("ctrl+shift+p").as_deref(),
            Some("workbench.action.showCommands")
        );
        assert_eq!(id("f1").as_deref(), Some("workbench.action.showCommands"));
        assert_eq!(
            id("f12"),
            None,
            "needs editorHasDefinitionProvider && editorTextFocus"
        );
        c.set_many([
            (
                "editorHasDefinitionProvider".into(),
                Some(Value::Bool(true)),
            ),
            ("editorTextFocus".into(), Some(Value::Bool(true))),
        ])
        .unwrap();
        assert_eq!(id("f12").as_deref(), Some("editor.action.revealDefinition"));
    }
}
