//! Shared contracts. No UI or executor dependency; all offsets refer to decoded UTF-8.
//!
//! Coordinate units cannot be silently interchanged:
//! ```compile_fail
//! use rove_base::{Utf8Column, Utf16Column};
//! let utf8: Utf8Column = Utf16Column(2);
//! ```
//! ```compile_fail
//! use rove_base::{ByteOffset, Utf8Column};
//! let offset: ByteOffset = Utf8Column(2);
//! ```
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

/// Stable failure categories, not presentation-ready UI messages. Send + Sync; constant-size dispatch.
/// Diagnostic text may be localized by the OS; the UI maps categories to registered string keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    InvalidInput,
    OutOfBounds,
    NotReady,
    NotFound,
    Duplicate(String),
    Unsupported,
    /// Filesystem failure with machine-readable category and log/diagnostic-only text.
    /// ErrorKind is non_exhaustive: consumers must include a fallback match arm.
    /// This cloneable summary does not retain the original error chain; Error::source() is None.
    Io {
        kind: std::io::ErrorKind,
        message: String,
    },
    Stale,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
/// Executor-neutral background operation. Poll outside paint/layout; cancellation is explicit.
pub type Task<T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>;

macro_rules! unit {
    ($name:ident, $doc:literal) => {
        #[doc=$doc]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);
    };
}
unit!(
    ByteOffset,
    "Zero-based byte offset in decoded UTF-8, not disk encoding or a column."
);
unit!(
    Utf8Column,
    "Zero-based UTF-8 byte column, at a Unicode scalar boundary; not a visual column."
);
unit!(
    Utf16Column,
    "Zero-based UTF-16 code-unit column; never cast to Utf8Column."
);
unit!(Line, "Zero-based logical line index; display adds one.");
unit!(
    Version,
    "Monotonic revision within one document identity; never compared across documents."
);
unit!(
    DocumentId,
    "Stable document identity for the lifetime of its open model."
);
/// Ordered half-open range. Constructors reject inverted ranges; empty ranges are valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span<T> {
    start: T,
    end: T,
}
impl<T: Ord + Copy> Span<T> {
    pub fn new(start: T, end: T) -> Result<Self> {
        if start > end {
            Err(Error::InvalidInput)
        } else {
            Ok(Self { start, end })
        }
    }
    pub fn start(&self) -> T {
        self.start
    }
    pub fn end(&self) -> T {
        self.end
    }
}
pub type ByteRange = Span<ByteOffset>;
pub type LineRange = Span<Line>;
/// Position in decoded text, zero-based; conversion validates scalar boundaries in TextSource.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: Line,
    pub column: Utf8Column,
}
pub type TextRange = Span<Position>;
/// Replacement against one expected revision. Owned inserted text only, never a document copy.
#[derive(Clone, Debug)]
pub struct TextEdit {
    pub version: Version,
    pub range: ByteRange,
    pub inserted: Arc<str>,
}
/// Normalized workspace-relative path. No disk I/O; lexical validation is O(path length).
/// Workspace identity and Windows filesystem case/alias canonicalization belong to #15.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspacePath(Arc<str>);
impl WorkspacePath {
    pub fn new(path: &str) -> Result<Self> {
        if path.is_empty()
            || path.contains(['\\', ':', '\0'])
            || path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(Error::InvalidInput);
        }
        Ok(Self(path.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Addressable source: disk files outside the workspace and generated metadata are supported.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resource {
    File(std::path::PathBuf),
    Workspace(WorkspacePath),
    Virtual(Arc<str>),
}
/// Cooperative cancellation; clones share state. O(1), Send + Sync, no callbacks or blocking.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
/// RAII listener ownership. Dropping unregisters; a callback already in flight may complete.
/// Cleanup must not block or panic. UI owners drop before their view is destroyed.
#[derive(Default)]
pub struct Subscription(Option<Box<dyn FnOnce() + Send + Sync>>);
impl Subscription {
    pub fn new(unsubscribe: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self(Some(Box::new(unsubscribe)))
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f()
        }
    }
}
/// Events arrive on producer threads after locks are released; callbacks must enqueue and return.
/// No I/O or UI mutation here. Subscription is O(1) expected; broadcast O(listeners).
pub trait Events<T>: Send + Sync {
    fn subscribe(&self, listener: Arc<dyn Fn(T) + Send + Sync>) -> Subscription;
}
/// Empty event source used by fixed providers; retains no callbacks, emits no events.
pub struct NoEvents;
impl<T> Events<T> for NoEvents {
    fn subscribe(&self, _: Arc<dyn Fn(T) + Send + Sync>) -> Subscription {
        Subscription::default()
    }
}
/// Common registration mechanism: deterministic ID ordering; duplicate IDs are errors.
/// Assemble before sharing immutable registries (Send + Sync if T is); O(log n) lookup/insert.
pub struct Registry<T>(BTreeMap<String, T>);
impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}
impl<T> Registry<T> {
    pub fn register(&mut self, id: impl Into<String>, value: T) -> Result<()> {
        let id = id.into();
        if id.is_empty() {
            return Err(Error::InvalidInput);
        }
        if self.0.contains_key(&id) {
            return Err(Error::Duplicate(id));
        }
        self.0.insert(id, value);
        Ok(())
    }
    pub fn get(&self, id: &str) -> Option<&T> {
        self.0.get(id)
    }
    pub fn entries(&self) -> impl Iterator<Item = (&str, &T)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}
/// JSON-compatible values, shared by configuration and command arguments. No parsing policy here.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(Arc<str>),
    Array(Arc<[Value]>),
    /// Shared immutable object; cloning does not copy the table or nested values.
    Object(Arc<BTreeMap<String, Value>>),
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consumer_uses_units_paths_and_cancellation() {
        assert!(WorkspacePath::new("../outside").is_err());
        assert_eq!(
            WorkspacePath::new("Assets/A.cs").unwrap().as_str(),
            "Assets/A.cs"
        );
        assert!(Span::new(ByteOffset(3), ByteOffset(2)).is_err());
        let c = Cancellation::default();
        c.clone().cancel();
        assert_eq!(c.check(), Err(Error::Cancelled));
    }
    #[test]
    fn io_consumers_can_branch_on_error_kind() {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied,
        ] {
            let error: Error = std::io::Error::new(kind, "fixture").into();
            assert_eq!(
                error,
                Error::Io {
                    kind,
                    message: "fixture".into()
                }
            );
        }
    }
    #[test]
    fn object_snapshots_share_the_same_table() {
        let original = Value::Object(Arc::new(BTreeMap::from([(
            "enabled".into(),
            Value::Bool(true),
        )])));
        let cloned = original.clone();
        let (Value::Object(a), Value::Object(b)) = (original, cloned) else {
            unreachable!()
        };
        assert!(Arc::ptr_eq(&a, &b));
    }
    #[test]
    fn registration_never_overwrites() {
        let mut r = Registry::default();
        r.register("a", 1).unwrap();
        assert!(r.register("a", 2).is_err());
        assert_eq!(r.get("a"), Some(&1));
    }
    #[test]
    fn subscription_releases_listener() {
        let flag = Arc::new(AtomicBool::new(false));
        let copy = flag.clone();
        {
            let _s = Subscription::new(move || copy.store(true, Ordering::Relaxed));
        }
        assert!(flag.load(Ordering::Relaxed));
    }
}
