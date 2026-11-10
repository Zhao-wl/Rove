//! File watching: debounce, merge, rename pairing and overflow recovery (#15).
//!
//! `notify` maps to `ReadDirectoryChangesW` on Windows. Two design points matter:
//!
//! - Watchers are registered **per directory** with [`RecursiveMode::NonRecursive`] after the
//!   exclusion rules were applied. A recursive root watch cannot honour `files.watcherExclude`,
//!   and Unity's `Library/` would otherwise flood the queue before the first event is filtered.
//! - Events are debounced with a quiet window (50 ms) and merged into one batch. A batch that
//!   grows past [`WorkspaceConfig::event_queue_limit`] is treated as lost events: the batch is
//!   discarded, the workspace re-scans and reports [`FileEvent::RescanRequired`].
//!
//! Windows reports a rename as a `From`/`To` pair; [`parse`] pairs them back into a single
//! [`WatchChange::Renamed`] instead of a delete plus a create.

use crate::folder::Folder;
use notify::event::{Flag, ModifyKind, RenameMode};
use notify::{Event, EventKind, RecursiveMode, Watcher as _};
use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// Quiet window: once no event arrived for this long, the batch is applied.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(50);
/// Upper bound for waiting on the command channel when idle.
const IDLE_POLL: Duration = Duration::from_millis(100);

/// A structural change derived from one or more raw backend events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WatchChange {
    Created(PathBuf),
    Changed(PathBuf),
    Removed(PathBuf),
    Renamed(PathBuf, PathBuf),
}

pub(crate) enum WatchCommand {
    /// Register the directories, then optionally acknowledge on `done`.
    WatchDirs {
        dirs: Vec<PathBuf>,
        done: Option<Sender<()>>,
    },
}

pub(crate) struct WatcherHandle {
    commands: Sender<WatchCommand>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl WatcherHandle {
    /// Cloneable command channel. Callers must not hold the folder's watcher mutex while waiting
    /// for an acknowledgement, or the watcher thread could block on the same mutex.
    pub(crate) fn commands(&self) -> Sender<WatchCommand> {
        self.commands.clone()
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        // The thread polls with a short timeout, so joining is bounded and keeps the process
        // from leaking one thread per closed workspace.
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.send(WatchCommand::WatchDirs {
            dirs: Vec::new(),
            done: None,
        });
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Spawns the watcher thread and registers the initial directories.
///
/// Blocks (bounded) until the thread has issued the initial `watch` calls, so a caller that sees
/// [`Folder::watching`] return `true` cannot lose a write to a registration race.
pub(crate) fn spawn(folder: &Arc<Folder>, dirs: Vec<PathBuf>) -> Option<WatcherHandle> {
    let (commands, command_receiver) = mpsc::channel();
    let (ready_sender, ready_receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let weak = Arc::downgrade(folder);
    let join = std::thread::Builder::new()
        .name("rove-workspace-watch".into())
        .spawn(move || run(weak, command_receiver, thread_stop, dirs, ready_sender))
        .ok()?;
    let _ = ready_receiver.recv_timeout(Duration::from_secs(2));
    Some(WatcherHandle {
        commands,
        stop,
        join: Some(join),
    })
}

fn run(
    folder: Weak<Folder>,
    commands: Receiver<WatchCommand>,
    stop: Arc<AtomicBool>,
    dirs: Vec<PathBuf>,
    ready: Sender<()>,
) {
    let (sender, receiver) = mpsc::channel();
    let Ok(mut watcher) = notify::recommended_watcher(sender) else {
        return;
    };
    for dir in dirs {
        let _ = watcher.watch(&dir, RecursiveMode::NonRecursive);
    }
    // Registration is done; unblock `spawn` before waiting for events.
    let _ = ready.send(());

    let mut pending: Vec<Event> = Vec::new();
    let mut deadline: Option<Instant> = None;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        while let Ok(command) = commands.try_recv() {
            match command {
                WatchCommand::WatchDirs { dirs, done } => {
                    for dir in dirs {
                        let _ = watcher.watch(&dir, RecursiveMode::NonRecursive);
                    }
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                }
            }
        }
        let wait = deadline.map_or(IDLE_POLL, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        match receiver.recv_timeout(wait.max(Duration::from_millis(1))) {
            Ok(Ok(event)) => {
                pending.push(event);
                deadline = Some(Instant::now() + DEBOUNCE);
            }
            // A backend error (for example an unreadable directory) is treated like an
            // overflow: the retained snapshot may be stale, so re-scan immediately instead of
            // waiting for a quiet window that would never come (the error is not a file event).
            Ok(Err(_)) => {
                pending.push(rescan_event());
                deadline = Some(Instant::now());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let due = deadline.is_some_and(|deadline| Instant::now() >= deadline);
        let Some(folder) = folder.upgrade() else {
            break;
        };
        let overflow = pending.len() > folder.event_queue_limit();
        if due || overflow {
            let events = std::mem::take(&mut pending);
            folder.apply_watch_events(events, overflow);
            deadline = None;
        }
    }
}

fn rescan_event() -> Event {
    Event::new(EventKind::Other).set_flag(Flag::Rescan)
}

/// Turns a raw debounced batch into ordered structural changes.
///
/// Order: renames, removals, creations, then content changes. A path that appears in more than
/// one list keeps its strongest meaning (rename > create/remove > changed). A path that was both
/// removed and created inside one window becomes `Changed`: it existed before and exists now, and
/// the snapshot layer decides from the live snapshot whether that is a `Changed` or a `Created`
/// (so "write twice" collapses to one event and "delete + recreate" is never silently dropped).
pub(crate) fn parse(events: &[Event]) -> Vec<WatchChange> {
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut created: BTreeSet<PathBuf> = BTreeSet::new();
    let mut removed: BTreeSet<PathBuf> = BTreeSet::new();
    let mut changed: BTreeSet<PathBuf> = BTreeSet::new();
    let mut pending_from: VecDeque<PathBuf> = VecDeque::new();

    for event in events {
        match event.kind {
            EventKind::Create(_) => created.extend(event.paths.iter().cloned()),
            EventKind::Remove(_) => removed.extend(event.paths.iter().cloned()),
            EventKind::Modify(ModifyKind::Name(mode)) => match mode {
                RenameMode::Both => {
                    if event.paths.len() >= 2 {
                        renames.push((event.paths[0].clone(), event.paths[1].clone()));
                    }
                }
                RenameMode::From => pending_from.extend(event.paths.iter().cloned()),
                RenameMode::To => {
                    for path in &event.paths {
                        match pending_from.pop_front() {
                            Some(from) => renames.push((from, path.clone())),
                            None => {
                                created.insert(path.clone());
                            }
                        }
                    }
                }
                _ => changed.extend(event.paths.iter().cloned()),
            },
            EventKind::Modify(_) => changed.extend(event.paths.iter().cloned()),
            // Reads, opens and metadata-only noise are not workspace changes.
            EventKind::Access(_) => {}
            _ => changed.extend(event.paths.iter().cloned()),
        }
    }
    // A `From` without a matching `To` is a move out of the watched subtree.
    removed.extend(pending_from);

    let renamed_paths: BTreeSet<PathBuf> = renames
        .iter()
        .flat_map(|(from, to)| [from.clone(), to.clone()])
        .collect();
    for path in &renamed_paths {
        created.remove(path);
        removed.remove(path);
        changed.remove(path);
    }
    let recreated: Vec<PathBuf> = created
        .iter()
        .filter(|path| removed.contains(*path))
        .cloned()
        .collect();
    for path in recreated {
        // Existed before the window and exists after it: a content change, not delete+create.
        removed.remove(&path);
        created.remove(&path);
        changed.insert(path);
    }
    for path in &created {
        changed.remove(path);
    }
    for path in &removed {
        changed.remove(path);
    }

    let mut changes =
        Vec::with_capacity(renames.len() + created.len() + removed.len() + changed.len());
    changes.extend(
        renames
            .into_iter()
            .map(|(from, to)| WatchChange::Renamed(from, to)),
    );
    changes.extend(removed.into_iter().map(WatchChange::Removed));
    changes.extend(created.into_iter().map(WatchChange::Created));
    changes.extend(changed.into_iter().map(WatchChange::Changed));
    changes
}

/// True when this batch has to be answered with a re-scan.
pub(crate) fn needs_rescan(events: &[Event]) -> bool {
    events.iter().any(|event| event.need_rescan())
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, RemoveKind};

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        let mut event = Event::new(kind);
        event.paths = paths.iter().map(PathBuf::from).collect();
        event
    }

    #[test]
    fn a_from_to_pair_becomes_one_rename() {
        let events = [
            event(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                &["old.cs"],
            ),
            event(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                &["new.cs"],
            ),
        ];
        assert_eq!(
            parse(&events),
            vec![WatchChange::Renamed("old.cs".into(), "new.cs".into())]
        );
    }

    #[test]
    fn a_both_event_is_a_rename_too() {
        let events = [event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["old.cs", "new.cs"],
        )];
        assert_eq!(
            parse(&events),
            vec![WatchChange::Renamed("old.cs".into(), "new.cs".into())]
        );
    }

    #[test]
    fn repeated_writes_collapse_into_one_change() {
        let events = [
            event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &["A.cs"],
            ),
            event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &["A.cs"],
            ),
            event(EventKind::Modify(ModifyKind::Any), &["A.cs"]),
        ];
        assert_eq!(parse(&events), vec![WatchChange::Changed("A.cs".into())]);
    }

    #[test]
    fn create_then_modify_reports_only_the_creation() {
        let events = [
            event(EventKind::Create(CreateKind::File), &["A.cs"]),
            event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &["A.cs"],
            ),
        ];
        assert_eq!(parse(&events), vec![WatchChange::Created("A.cs".into())]);
    }

    #[test]
    fn removal_wins_over_a_prior_write() {
        let events = [
            event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &["A.cs"],
            ),
            event(EventKind::Remove(RemoveKind::File), &["A.cs"]),
        ];
        assert_eq!(parse(&events), vec![WatchChange::Removed("A.cs".into())]);
    }

    #[test]
    fn a_recreated_path_is_reported_as_changed_not_dropped() {
        let events = [
            event(EventKind::Remove(RemoveKind::File), &["A.cs"]),
            event(EventKind::Create(CreateKind::File), &["A.cs"]),
        ];
        // The path existed before the window and exists after it. `Changed` lets the snapshot
        // layer decide (Changed for a known path, Created for an unknown one) instead of
        // dropping the event because `add()` would be a no-op.
        assert_eq!(parse(&events), vec![WatchChange::Changed("A.cs".into())]);
    }

    #[test]
    fn a_rename_pair_is_paired_first_in_first_out() {
        let events = [
            event(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                &["a.cs"],
            ),
            event(
                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                &["b.cs"],
            ),
            event(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                &["x.cs"],
            ),
            event(
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                &["y.cs"],
            ),
        ];
        assert_eq!(
            parse(&events),
            vec![
                WatchChange::Renamed("a.cs".into(), "x.cs".into()),
                WatchChange::Renamed("b.cs".into(), "y.cs".into()),
            ]
        );
    }

    #[test]
    fn access_events_are_ignored() {
        let events = [event(
            EventKind::Access(notify::event::AccessKind::Read),
            &["A.cs"],
        )];
        assert!(parse(&events).is_empty());
    }

    #[test]
    fn a_rescan_flag_is_detected() {
        let events = [rescan_event()];
        assert!(needs_rescan(&events));
        assert!(!needs_rescan(&[event(
            EventKind::Create(CreateKind::File),
            &["A.cs"]
        )]));
    }
}
