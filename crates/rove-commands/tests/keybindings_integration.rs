//! #14 entry-point integration tests:
//! * the generated default table covers every P0 row of `docs/compat/KEYBINDINGS.md`;
//! * reading a `keybindings.json` → pressing keys → dispatching to the command handler.
use rove_base::{Cancellation, Value};
use rove_commands::{
    center::{sync_handler, CommandCenter},
    chord::{ChordState, KeyOutcome},
    context::{keys, ContextKeyService},
    keybindings::{default_rules, KeybindingService, Priority},
    keys::parse_chord,
    Command, CommandExecutor, ContextKeys,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Instant,
};

fn repo_file(path: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path)).unwrap()
}

fn unescape_md(cell: &str) -> String {
    cell.trim().trim_matches('`').replace("\\|", "|")
}

/// Same normalization the doc generator applies to `when` (user directory → %APPDATA%).
fn sanitize(when: &str) -> String {
    let mut s = when.to_owned();
    while let Some(i) = s.find("/Users/") {
        let Some(rest) = s[i..].find("/AppData/Roaming") else {
            break;
        };
        let start = s[..i].rfind(['c', 'C']).unwrap_or(i);
        s.replace_range(start..i + rest + "/AppData/Roaming".len(), "%APPDATA%");
    }
    s
}

#[test]
fn every_p0_row_of_keybindings_md_is_in_the_default_table() {
    let md = repo_file("docs/compat/KEYBINDINGS.md");
    let start = md.find("## 3. ").unwrap();
    let end = md.find("## 4. ").unwrap();
    let mut expected = Vec::new();
    for line in md[start..end].lines().filter(|l| l.starts_with("| P0 |")) {
        // Split on unescaped pipes only.
        let mut cells = Vec::new();
        let mut cur = String::new();
        let mut prev = '\0';
        for ch in line.chars() {
            if ch == '|' && prev != '\\' {
                cells.push(std::mem::take(&mut cur));
            } else {
                cur.push(ch);
            }
            prev = ch;
        }
        expected.push((
            unescape_md(&cells[2]),
            unescape_md(&cells[3]),
            unescape_md(&cells[4]),
        ));
    }
    // KEYBINDINGS.md §2 statistic.
    let stat: usize = md
        .lines()
        .find(|l| l.starts_with("| 实现 P0 |"))
        .and_then(|l| l.split('|').nth(2))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(expected.len(), stat, "§3 P0 rows vs §2 count");

    let table: BTreeSet<(String, String, String)> = default_rules()
        .into_iter()
        .filter(|d| d.priority == Priority::P0 && d.source.as_ref() == "vscode")
        .map(|d| {
            (
                d.rule.key.unwrap(),
                d.rule.command,
                sanitize(&d.rule.when.unwrap_or_default()),
            )
        })
        .collect();
    let missing: Vec<_> = expected
        .iter()
        .filter(|row| !table.contains(*row))
        .collect();
    assert!(missing.is_empty(), "missing P0 bindings: {missing:#?}");
    assert_eq!(table.len(), expected.iter().collect::<BTreeSet<_>>().len());

    // Manual supplement (§4 table) is present too.
    assert!(default_rules()
        .iter()
        .any(|d| d.source.as_ref() == "supplement"
            && d.rule.command == "workbench.actions.view.problems"
            && d.rule.key.as_deref() == Some("ctrl+shift+m")));
}

#[test]
fn keybindings_json_to_key_press_to_command() {
    let dir = std::env::temp_dir().join(format!("rove-keys-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("keybindings.json");
    std::fs::write(
        &path,
        r#"// User keybindings shared with VS Code
[
    // rebinding: drop the default ctrl+p, use a chord instead
    { "key": "ctrl+p", "command": "-workbench.action.quickOpen" },
    { "key": "ctrl+k ctrl+o", "command": "workbench.action.quickOpen", "args": { "from": "user" } },
    // a command Rove does not have: silently ignored
    { "key": "f5", "command": "workbench.action.debug.start" },
    { "key": "ctrl+k b", "command": "rove.contextBasket.add", "when": "editorTextFocus && !editorReadonly" },
]"#,
    )
    .unwrap();

    let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let mut center = CommandCenter::default();
    rove_commands::builtin::register_rove_commands(&mut center).unwrap();
    for id in [
        "workbench.action.quickOpen",
        "workbench.action.showCommands",
    ] {
        center
            .register(
                id,
                Command {
                    title_key: format!("t.{id}").into(),
                    category_key: None,
                },
            )
            .unwrap();
    }
    for id in ["workbench.action.quickOpen", "rove.contextBasket.add"] {
        let sink = seen.clone();
        center
            .implement(
                id,
                sync_handler(move |inv| {
                    sink.lock()
                        .unwrap()
                        .push((inv.id.to_string(), inv.args.clone()));
                    Ok(Value::Null)
                }),
            )
            .unwrap();
    }

    let service = KeybindingService::with_builtin_defaults();
    service.watch_user_file(&path).unwrap();
    assert!(
        service.diagnostics().is_empty(),
        "{:?}",
        service.diagnostics()
    );
    let context = ContextKeyService::with_platform_defaults();
    let mut chord = ChordState::default();
    let now = Instant::now();
    let mut press = |k: &str| {
        chord
            .press(
                parse_chord(k).unwrap(),
                now,
                &service,
                &context,
                center.registry(),
            )
            .unwrap()
    };
    let run = |outcome: KeyOutcome| {
        let KeyOutcome::Command(invocation) = outcome else {
            panic!("{outcome:?}")
        };
        let mut task = center.execute(invocation, Cancellation::default());
        let waker = std::task::Waker::noop();
        match task
            .as_mut()
            .poll(&mut std::task::Context::from_waker(waker))
        {
            std::task::Poll::Ready(r) => r.unwrap(),
            std::task::Poll::Pending => panic!("sync handler"),
        };
    };

    // Default ctrl+p was removed by the user file.
    assert!(matches!(press("ctrl+p"), KeyOutcome::Unhandled));
    // The user chord dispatches with its args.
    assert!(matches!(press("ctrl+k"), KeyOutcome::Pending(_)));
    run(press("ctrl+o"));
    // Unknown command in the file: no binding, no error.
    assert!(matches!(press("f5"), KeyOutcome::Unhandled));
    // `when` gates the Rove command.
    assert!(matches!(press("ctrl+k"), KeyOutcome::Pending(_)));
    assert!(matches!(press("b"), KeyOutcome::Aborted(_)));
    context
        .set(keys::EDITOR_TEXT_FOCUS.into(), Some(Value::Bool(true)))
        .unwrap();
    press("ctrl+k");
    run(press("b"));
    // Registered but not implemented: resolves, and the executor reports Unsupported.
    let KeyOutcome::Command(inv) = press("ctrl+shift+p") else {
        panic!()
    };
    assert_eq!(inv.id.as_ref(), "workbench.action.showCommands");
    assert!(!center.is_implemented(&inv.id));

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "workbench.action.quickOpen");
    let Value::Object(args) = &seen[0].1 else {
        panic!()
    };
    assert_eq!(args.get("from"), Some(&Value::String("user".into())));
    assert_eq!(seen[1].0, "rove.contextBasket.add");
    let _ = std::fs::remove_dir_all(&dir);
}
