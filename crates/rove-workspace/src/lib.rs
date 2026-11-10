//! Workspace scanning, exclusion/read-only rules, file watching and path identity (#15).
//!
//! The public surface is the frozen [`Workspace`] contract plus the concrete [`Folder`]
//! implementation and the settings registration this crate owns.
//!
//! # Guarantees
//!
//! - **Never reads `.gitignore`.** The parallel scan disables every ignore-file switch; only
//!   `files.exclude` and `rove.files.extraExclude` exclude paths (AGENTS §1).
//! - **Streaming.** [`Folder::start`] lists the root directory before returning; batches of the
//!   rest of the tree follow through revisions and `Created` events. `Entries::complete` says
//!   whether the initial scan finished.
//! - **Fault tolerant.** A non-UTF-8 name, an unreadable directory or a file that disappears
//!   mid-scan is counted ([`Folder::skipped`]) and skipped; it never fails the scan.
//! - **Long paths.** The root is canonicalized with the Windows extended-length prefix, so
//!   anything past `MAX_PATH` is readable through [`Folder::read_file`].
//! - **Case-insensitive identity, case-preserving display.** See [`fold_key`]: the identity key
//!   folds case on Windows; the snapshot keeps the on-disk spelling.
//! - **Overflow recovery.** When the watcher event queue overflows, the snapshot is rebuilt and
//!   `FileEvent::RescanRequired` is reported. Windows `From`/`To` pairs surface as one
//!   `FileEvent::Renamed`.
//!
//! # Known limitations
//!
//! - `files.exclude`/`rove.files.*` values that are objects (`{"when": "$(basename).ts"}`) are
//!   ignored, not evaluated; boolean values are fully supported (see [`RuleSet`]).
//! - Watching is not recursive at the OS level: one non-recursive watch per visible directory,
//!   so `files.watcherExclude` is honoured at registration time. Very deep trees therefore use
//!   one handle per directory.
//! - `.gitignore` support, global search, fuzzy matching and multi-root workspaces are other
//!   issues (#25, #24, #23); this crate does not implement them.

mod contracts;
mod events;
mod folder;
mod glob;
mod io;
mod path;
mod rules;
mod scan;
mod snapshot;
mod watch;

pub use contracts::*;
pub use events::EventSource;
pub use folder::{open, wait_until_complete, Folder};
pub use glob::RuleSet;
pub use io::read_shared;
pub use path::fold_key;
pub use rules::{
    default_settings, register_settings, ReadonlyRules, WorkspaceConfig, FILES_EXCLUDE,
    FILES_READONLY_EXCLUDE, FILES_READONLY_FROM_PERMISSIONS, FILES_READONLY_INCLUDE,
    FILES_WATCHER_EXCLUDE, REASON_EXTRA_READONLY_INCLUDE, REASON_READONLY_INCLUDE,
    ROVE_EXTRA_EXCLUDE, ROVE_EXTRA_READONLY_INCLUDE,
};
