//! End-to-end workspace tests (#15): scanning, exclusions, watching, rules and read paths.
//!
//! These use the public contract only (`Workspace` plus `Folder` construction) and a real file
//! watcher on a temporary directory. Tests wait with bounded timeouts instead of sleeping.

use rove_base::{Events, WorkspacePath};
use rove_workspace::{
    default_settings, wait_until_complete, FileEvent, FileEvents, Folder, Workspace,
    WorkspaceConfig,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn default_config() -> WorkspaceConfig {
    WorkspaceConfig::from_settings(&default_settings()).expect("registered defaults")
}

struct EventLog {
    events: Arc<Mutex<Vec<FileEvent>>>,
    _subscription: rove_base::Subscription,
}

impl EventLog {
    fn snapshot(&self) -> Vec<FileEvent> {
        self.events.lock().expect("event log").clone()
    }

    fn any(&self, predicate: impl Fn(&FileEvent) -> bool) -> bool {
        self.events.lock().expect("event log").iter().any(predicate)
    }
}

fn subscribe(folder: &Arc<Folder>) -> EventLog {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let subscription = folder
        .changes()
        .subscribe(Arc::new(move |batch: FileEvents| {
            sink.lock()
                .expect("event log")
                .extend(batch.events.iter().cloned());
        }));
    EventLog {
        events,
        _subscription: subscription,
    }
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_watching(folder: &Folder, timeout: Duration) -> bool {
    wait_until(timeout, || folder.watching())
}

fn file_paths(folder: &Folder) -> Vec<String> {
    folder
        .files(0..usize::MAX)
        .expect("files page")
        .items
        .iter()
        .map(|entry| entry.path.as_str().to_string())
        .collect()
}

fn write(path: &std::path::Path, contents: &str) {
    std::fs::write(path, contents).expect("write fixture");
}

#[test]
fn scan_then_exclusions_then_change_then_rules() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("Assets/Scripts")).expect("mkdir");
    std::fs::create_dir_all(dir.path().join("Library/PackageCache")).expect("mkdir");
    write(
        &dir.path().join("Assets/Scripts/Player.cs"),
        "class Player {}\n",
    );
    write(
        &dir.path().join("Library/PackageCache/Generated.cs"),
        "class Generated {}\n",
    );
    // The scan must ignore this file completely (AGENTS §1).
    write(&dir.path().join(".gitignore"), "Assets\nLibrary\n");

    let folder = Folder::start(dir.path(), default_config()).expect("start workspace");
    assert!(
        wait_until_complete(&folder, Duration::from_secs(15)),
        "initial scan must finish"
    );
    assert!(
        wait_watching(&folder, Duration::from_secs(15)),
        "watcher must register"
    );

    let paths = file_paths(&folder);
    assert!(
        paths.iter().any(|path| path == "Assets/Scripts/Player.cs"),
        "gitignore must not hide files: {paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path.starts_with("Library/")),
        "Rove extra exclude default must apply: {paths:?}"
    );

    let player = WorkspacePath::new("Assets/Scripts/Player.cs").expect("path");
    let player_rules = folder.rules(&player).expect("rules for a scanned file");
    assert!(!player_rules.excluded);
    assert!(player_rules.read_only.is_empty());

    let generated = WorkspacePath::new("Library/PackageCache/Generated.cs").expect("path");
    let generated_rules = folder
        .rules(&generated)
        .expect("rules for an excluded path");
    assert!(generated_rules.excluded);
    assert!(generated_rules.read_only.is_empty());

    let log = subscribe(&folder);
    write(
        &dir.path().join("Assets/Scripts/Player.cs"),
        "class Player { int Health; }\n",
    );
    assert!(
        wait_until(Duration::from_secs(15), || {
            log.any(|event| {
                matches!(event, FileEvent::Changed(path) if path.as_str() == "Assets/Scripts/Player.cs")
            })
        }),
        "expected a Changed event, got {:?}",
        log.snapshot()
    );
    assert_eq!(
        folder.read_file(&player).expect("read file"),
        b"class Player { int Health; }\n"
    );
}

#[test]
fn a_thousand_created_and_deleted_files_converge() {
    let dir = tempfile::tempdir().expect("tempdir");
    let folder = Folder::start(dir.path(), default_config()).expect("start workspace");
    assert!(wait_until_complete(&folder, Duration::from_secs(15)));
    assert!(wait_watching(&folder, Duration::from_secs(15)));

    for index in 0..1000 {
        write(
            &dir.path().join(format!("Batch{index:04}.cs")),
            "class A {}\n",
        );
    }
    assert!(
        wait_until(Duration::from_secs(30), || {
            folder.files(0..usize::MAX).expect("files").items.len() == 1000
        }),
        "snapshot must converge after a 1000-file burst, saw {}",
        folder.files(0..usize::MAX).expect("files").items.len()
    );

    for index in 0..1000 {
        std::fs::remove_file(dir.path().join(format!("Batch{index:04}.cs"))).expect("remove");
    }
    assert!(
        wait_until(Duration::from_secs(30), || {
            folder.files(0..usize::MAX).expect("files").items.is_empty()
        }),
        "snapshot must converge after a 1000-file delete, saw {:?}",
        file_paths(&folder)
    );
}

#[test]
fn a_rename_is_reported_as_one_rename_event() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(&dir.path().join("Before.cs"), "class Before {}\n");
    let folder = Folder::start(dir.path(), default_config()).expect("start workspace");
    assert!(wait_until_complete(&folder, Duration::from_secs(15)));
    assert!(wait_watching(&folder, Duration::from_secs(15)));

    let log = subscribe(&folder);
    std::fs::rename(dir.path().join("Before.cs"), dir.path().join("After.cs")).expect("rename");

    assert!(
        wait_until(Duration::from_secs(15), || {
            log.any(|event| matches!(event, FileEvent::Renamed { .. }))
        }),
        "expected one Renamed event, got {:?}",
        log.snapshot()
    );
    let events = log.snapshot();
    let renamed: Vec<&FileEvent> = events
        .iter()
        .filter(|event| matches!(event, FileEvent::Renamed { .. }))
        .collect();
    assert_eq!(renamed.len(), 1, "events: {events:?}");
    let FileEvent::Renamed { from, to } = renamed[0] else {
        unreachable!()
    };
    assert_eq!(from.as_str(), "Before.cs");
    assert_eq!(to.as_str(), "After.cs");
    assert!(
        !events.iter().any(|event| matches!(
            event,
            FileEvent::Removed(path) if path.as_str() == "Before.cs"
        )),
        "a rename must not be reported as a removal: {events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            FileEvent::Created(path) if path.as_str() == "After.cs"
        )),
        "a rename must not be reported as a creation: {events:?}"
    );

    assert!(
        wait_until(Duration::from_secs(15), || {
            let paths = file_paths(&folder);
            paths == ["After.cs"]
        }),
        "snapshot must show the renamed file: {:?}",
        file_paths(&folder)
    );
}

#[test]
fn an_event_overflow_recovers_by_rescanning() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = default_config().with_event_queue_limit(2);
    let folder = Folder::start(dir.path(), config).expect("start workspace");
    assert!(wait_until_complete(&folder, Duration::from_secs(15)));
    assert!(wait_watching(&folder, Duration::from_secs(15)));

    let log = subscribe(&folder);
    for index in 0..100 {
        write(
            &dir.path().join(format!("Flood{index:03}.cs")),
            "class Flood {}\n",
        );
    }

    assert!(
        wait_until(Duration::from_secs(30), || {
            log.any(|event| matches!(event, FileEvent::RescanRequired))
        }),
        "overflow must be reported with RescanRequired, got {:?}",
        log.snapshot()
    );
    assert!(
        wait_until(Duration::from_secs(30), || {
            folder.files(0..usize::MAX).expect("files").items.len() == 100
        }),
        "snapshot must match the disk after the overflow rescan: {:?}",
        file_paths(&folder)
    );
}

#[test]
fn the_root_is_visible_before_the_scan_completes() {
    let dir = tempfile::tempdir().expect("tempdir");
    for group in 0..15 {
        let group_dir = dir.path().join(format!("Group{group:02}"));
        std::fs::create_dir_all(&group_dir).expect("mkdir");
        for file in 0..100 {
            write(&group_dir.join(format!("F{file:03}.cs")), "class F {}\n");
        }
    }

    let started = Instant::now();
    let folder = Folder::start(dir.path(), default_config()).expect("start workspace");
    let open_elapsed = started.elapsed();

    // Root listing is synchronous: the first frame can paint it.
    let root = folder.children(None, 0..64).expect("root children");
    assert_eq!(root.items.len(), 15, "root directories listed at open");
    assert!(
        open_elapsed < Duration::from_secs(1),
        "start must return without scanning the tree (took {open_elapsed:?})"
    );
    // The scan is still running: `start` does not wait for the full tree.
    assert!(
        !folder.files(0..0).expect("files").complete,
        "start() must not block until the scan is complete"
    );

    assert!(
        wait_until_complete(&folder, Duration::from_secs(30)),
        "initial scan must finish"
    );
    assert_eq!(file_paths(&folder).len(), 1500);
}

#[test]
fn re_including_library_exposes_the_extra_read_only_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("Library/PackageCache/com.unity.x")).expect("mkdir");
    write(
        &dir.path()
            .join("Library/PackageCache/com.unity.x/Editor.cs"),
        "class Editor {}\n",
    );
    // A user explicitly re-includes Library; the Rove read-only default must then apply to the
    // package cache (Unity overwrites it), which is the only state where the rule is observable.
    let settings = OverrideSettings::new(
        "files.exclude",
        rove_base::Value::Object(Arc::new(
            [("**/Library".to_string(), rove_base::Value::Bool(false))].into(),
        )),
    );
    let config = WorkspaceConfig::from_settings(&settings).expect("config");
    let folder = Folder::scan_root(dir.path(), config.without_watching()).expect("scan");

    let path = WorkspacePath::new("Library/PackageCache/com.unity.x/Editor.cs").expect("path");
    let rules = folder.rules(&path).expect("rules");
    assert!(!rules.excluded, "Library was re-included");
    assert_eq!(rules.read_only.len(), 1);
    assert!(
        matches!(
            &rules.read_only[0],
            rove_buffer::ReadOnlyReason::Rule(key)
                if key.as_ref() == rove_workspace::REASON_EXTRA_READONLY_INCLUDE
        ),
        "unexpected reasons: {:?}",
        rules.read_only
    );
}

/// Settings implementation that serves the registered defaults plus one explicit override, i.e.
/// what a loaded `settings.json` would look like after #13.
struct OverrideSettings {
    overrides: std::collections::BTreeMap<String, rove_base::Value>,
}

impl OverrideSettings {
    fn new(key: &str, value: rove_base::Value) -> Self {
        Self {
            overrides: std::collections::BTreeMap::from([(key.to_string(), value)]),
        }
    }
}

impl rove_config::Settings for OverrideSettings {
    fn read(&self, key: &str, scope: &rove_config::Scope) -> rove_base::Result<rove_base::Value> {
        if let Some(value) = self.overrides.get(key) {
            return Ok(value.clone());
        }
        default_settings().read(key, scope)
    }

    fn write(
        &self,
        _: Arc<str>,
        _: rove_config::Scope,
        _: rove_base::Value,
        cancel: rove_base::Cancellation,
    ) -> rove_base::Task<()> {
        Box::pin(async move {
            cancel.check()?;
            Err(rove_base::Error::Unsupported)
        })
    }

    fn changes(&self) -> &dyn Events<rove_config::SettingsChange> {
        &rove_base::NoEvents
    }
}
