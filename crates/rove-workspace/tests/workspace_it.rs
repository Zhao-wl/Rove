//! End-to-end workspace tests (#15): scanning, exclusions, watching, rules and read paths.
//!
//! These use the public contract only (`Workspace` plus `Folder` construction) and a real file
//! watcher on a temporary directory. Tests wait with bounded timeouts instead of sleeping.

use rove_base::{Events, WorkspacePath};
use rove_workspace::{
    default_settings, wait_until_complete, FileEvent, FileEvents, Folder, RuleSet, Workspace,
    WorkspaceConfig,
};
use std::path::Path;
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

/// Every file below `root`, as sorted workspace-relative `/` paths. The disk is the ground truth
/// in the convergence tests.
fn disk_files(root: &Path) -> Vec<String> {
    fn walk(directory: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                walk(&path, root, out);
            } else if let Ok(relative) = path.strip_prefix(root) {
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn watcher_exclude(patterns: &[(&str, bool)]) -> RuleSet {
    RuleSet::from_value(&rove_base::Value::Object(Arc::new(
        patterns
            .iter()
            .map(|(pattern, enabled)| (pattern.to_string(), rove_base::Value::Bool(*enabled)))
            .collect(),
    )))
    .expect("valid rule object")
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

    // The acceptance criterion is "1000 changes within one second, then the event stream's final
    // state matches the disk". Whether the test machine can create 1000 files in one second is a
    // property of the disk, not of the workspace, so the duration is measured and reported but
    // not asserted; convergence itself is bounded (10 s) instead of "eventually".
    let burst_started = Instant::now();
    for index in 0..1000 {
        write(
            &dir.path().join(format!("Batch{index:04}.cs")),
            "class A {}\n",
        );
    }
    let burst = burst_started.elapsed();
    if burst >= Duration::from_secs(1) {
        eprintln!("注意：本机创建 1000 个文件用了 {burst:?}（>1 s），仍验证最终一致");
    }
    let convergence_started = Instant::now();
    assert!(
        wait_until(Duration::from_secs(10), || {
            folder.files(0..usize::MAX).expect("files").items.len() == 1000
        }),
        "snapshot must converge after a 1000-file burst, saw {}",
        folder.files(0..usize::MAX).expect("files").items.len()
    );
    eprintln!(
        "1000-file create burst {:?}, converged in {:?}",
        burst,
        convergence_started.elapsed()
    );

    let burst_started = Instant::now();
    for index in 0..1000 {
        std::fs::remove_file(dir.path().join(format!("Batch{index:04}.cs"))).expect("remove");
    }
    let delete_burst = burst_started.elapsed();
    let convergence_started = Instant::now();
    assert!(
        wait_until(Duration::from_secs(10), || {
            folder.files(0..usize::MAX).expect("files").items.is_empty()
        }),
        "snapshot must converge after a 1000-file delete, saw {:?}",
        file_paths(&folder)
    );
    eprintln!(
        "1000-file delete burst {:?}, converged in {:?}",
        delete_burst,
        convergence_started.elapsed()
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

/// True when the event mentions a path under `prefix`.
fn mentions(event: &FileEvent, prefix: &str) -> bool {
    match event {
        FileEvent::Created(path) | FileEvent::Changed(path) | FileEvent::Removed(path) => {
            path.as_str().starts_with(prefix)
        }
        FileEvent::Renamed { from, to } => {
            from.as_str().starts_with(prefix) || to.as_str().starts_with(prefix)
        }
        FileEvent::RescanRequired => false,
    }
}

#[test]
fn a_watcher_excluded_directory_produces_no_events() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("Watched")).expect("mkdir");
    std::fs::create_dir_all(dir.path().join("Ignored")).expect("mkdir");
    let mut config = default_config();
    config.watcher_exclude = watcher_exclude(&[("**/Ignored", true)]);
    let folder = Folder::start(dir.path(), config).expect("start workspace");
    assert!(wait_until_complete(&folder, Duration::from_secs(15)));
    assert!(wait_watching(&folder, Duration::from_secs(15)));

    let log = subscribe(&folder);
    write(&dir.path().join("Ignored/A.cs"), "class A {}\n");
    write(&dir.path().join("Watched/B.cs"), "class B {}\n");
    // Wait for the watched directory to prove the watcher is alive and processing this window.
    assert!(
        wait_until(Duration::from_secs(15), || {
            log.any(|event| {
                matches!(event, FileEvent::Created(path) if path.as_str() == "Watched/B.cs")
            })
        }),
        "expected the watched directory to report its file, got {:?}",
        log.snapshot()
    );
    // `Ignored` is excluded from watching, so no event may ever mention it (registration-time
    // exclusion, not post-hoc filtering).
    std::thread::sleep(Duration::from_millis(400));
    let events = log.snapshot();
    assert!(
        !events.iter().any(|event| mentions(event, "Ignored/")),
        "watcher-excluded directory produced events: {events:?}"
    );
    assert!(
        !file_paths(&folder)
            .iter()
            .any(|path| path.starts_with("Ignored/")),
        "watcher-excluded content must not leak into the snapshot: {:?}",
        file_paths(&folder)
    );
}

#[test]
fn a_directory_rename_moves_its_subtree() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("Assets/Scripts")).expect("mkdir");
    write(&dir.path().join("Assets/Scripts/A.cs"), "class A {}\n");
    write(&dir.path().join("Assets/Scripts/B.cs"), "class B {}\n");
    let folder = Folder::start(dir.path(), default_config()).expect("start workspace");
    assert!(wait_until_complete(&folder, Duration::from_secs(15)));
    assert!(wait_watching(&folder, Duration::from_secs(15)));

    let log = subscribe(&folder);
    std::fs::rename(
        dir.path().join("Assets/Scripts"),
        dir.path().join("Assets/Core"),
    )
    .expect("rename directory");

    assert!(
        wait_until(Duration::from_secs(15), || {
            log.any(|event| {
                matches!(
                    event,
                    FileEvent::Renamed { from, to }
                        if from.as_str() == "Assets/Scripts" && to.as_str() == "Assets/Core"
                )
            })
        }),
        "expected one directory Renamed event, got {:?}",
        log.snapshot()
    );
    assert!(
        wait_until(Duration::from_secs(15), || {
            file_paths(&folder) == ["Assets/Core/A.cs", "Assets/Core/B.cs"]
        }),
        "snapshot must show the moved subtree: {:?}",
        file_paths(&folder)
    );

    let events = log.snapshot();
    let renamed: Vec<&FileEvent> = events
        .iter()
        .filter(|event| {
            matches!(event, FileEvent::Renamed { from, .. } if from.as_str() == "Assets/Scripts")
        })
        .collect();
    assert_eq!(renamed.len(), 1, "one rename pair expected: {events:?}");
    // The old subtree must be reported as removed; otherwise indexes and open buffers keep paths
    // that no longer exist.
    for child in ["Assets/Scripts/A.cs", "Assets/Scripts/B.cs"] {
        assert!(
            events
                .iter()
                .any(|event| matches!(event, FileEvent::Removed(path) if path.as_str() == child)),
            "missing Removed for {child}: {events:?}"
        );
    }
    for child in ["Assets/Core/A.cs", "Assets/Core/B.cs"] {
        assert!(
            events
                .iter()
                .any(|event| matches!(event, FileEvent::Created(path) if path.as_str() == child)),
            "missing Created for {child}: {events:?}"
        );
    }
}

#[test]
fn changes_during_the_initial_scan_converge() {
    let dir = tempfile::tempdir().expect("tempdir");
    for group in 0..20 {
        let group_dir = dir.path().join(format!("Group{group:02}"));
        std::fs::create_dir_all(&group_dir).expect("mkdir");
        for file in 0..40 {
            write(&group_dir.join(format!("F{file:03}.cs")), "class F {}\n");
        }
    }

    let folder = Folder::start(dir.path(), default_config()).expect("start workspace");
    // Do not wait for the scan: change the tree while the walker is running. The watcher is
    // registered before the walk and a reconciliation runs before `complete` is published, so
    // the final snapshot must match the disk.
    let doomed = dir.path().join("Group00/F000.cs");
    let removed_existed = std::fs::remove_file(&doomed).is_ok();
    write(&dir.path().join("Group00/Late.cs"), "class Late {}\n");
    write(&dir.path().join("LateRoot.cs"), "class LateRoot {}\n");

    assert!(
        wait_until_complete(&folder, Duration::from_secs(30)),
        "initial scan must finish"
    );
    // `complete` is published only after the watcher is registered and the tree was reconciled,
    // so the snapshot must already match the disk at this exact moment — no extra waiting.
    assert_eq!(
        file_paths(&folder),
        disk_files(dir.path()),
        "snapshot must be consistent at the moment `complete` becomes visible"
    );
    assert!(wait_watching(&folder, Duration::from_secs(30)));
    let paths = file_paths(&folder);
    assert!(paths.iter().any(|path| path == "Group00/Late.cs"));
    assert!(paths.iter().any(|path| path == "LateRoot.cs"));
    if removed_existed {
        assert!(!paths.iter().any(|path| path == "Group00/F000.cs"));
    }
}

#[test]
fn a_directory_copied_in_while_still_being_written_converges() {
    let dir = tempfile::tempdir().expect("tempdir");
    let folder = Folder::start(dir.path(), default_config()).expect("start workspace");
    assert!(wait_until_complete(&folder, Duration::from_secs(15)));
    assert!(wait_watching(&folder, Duration::from_secs(15)));

    // Simulate a copy in progress: the directory becomes visible first, then its files keep
    // arriving while the watcher processes the creation event. The first subtree scan can only
    // see a prefix; registering the new directory and re-listing it must close the gap.
    let copied = dir.path().join("Assets/Copied");
    std::fs::create_dir_all(&copied).expect("create copied directory");
    for index in 0..200 {
        write(&copied.join(format!("F{index:03}.cs")), "class F {}\n");
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(
        wait_until(Duration::from_secs(15), || {
            file_paths(&folder) == disk_files(dir.path())
        }),
        "snapshot must converge after a directory was copied in: snapshot={:?} disk={:?}",
        file_paths(&folder),
        disk_files(dir.path())
    );
    assert_eq!(file_paths(&folder).len(), 200);
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
