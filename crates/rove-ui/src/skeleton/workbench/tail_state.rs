//! Follow state for one editor and file-event coalescing for one document.
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FollowMode {
    Following,
    Paused,
}

impl FollowMode {
    /// Only call this for input from the user, never for a programmatic viewport change.
    pub fn user_scrolled(&mut self, old_top: f64, new_top: f64, at_end: bool) -> bool {
        let before = *self;
        if new_top < old_top {
            *self = Self::Paused;
        } else if at_end {
            *self = Self::Following;
        }
        *self != before
    }

    pub fn follows(&self) -> bool {
        *self == Self::Following
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum ReloadRequest {
    #[default]
    None,
    PollPath,
    CheckAppend,
    ReopenPath,
}

pub enum PathEvent<'a> {
    Changed(&'a str),
    Created(&'a str),
    Removed(&'a str),
    Renamed { from: &'a str, to: &'a str },
    Rescan,
}

fn affects_path(event: &str, active: &str) -> bool {
    active == event
        || active
            .strip_prefix(event)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Always watches the path the user opened. A move to an archive only invalidates that path.
pub fn path_request(active: &str, event: PathEvent<'_>) -> ReloadRequest {
    match event {
        PathEvent::Changed(path) if active == path => ReloadRequest::CheckAppend,
        PathEvent::Created(path) | PathEvent::Removed(path) if affects_path(path, active) => {
            ReloadRequest::ReopenPath
        }
        PathEvent::Renamed { from, to }
            if affects_path(from, active) || affects_path(to, active) =>
        {
            ReloadRequest::ReopenPath
        }
        PathEvent::Rescan => ReloadRequest::ReopenPath,
        _ => ReloadRequest::None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Refresh {
    nonce: u64,
    pub request: ReloadRequest,
}

static NEXT_REFRESH: AtomicU64 = AtomicU64::new(1);

/// One physical read at a time for a document, even when multiple views follow it.
#[derive(Default)]
pub struct ReloadQueue {
    pending: ReloadRequest,
    flight: Option<Refresh>,
}

impl ReloadQueue {
    pub fn request(&mut self, request: ReloadRequest) {
        self.pending = self.pending.max(request);
    }

    pub fn start(&mut self) -> Option<Refresh> {
        if self.flight.is_some() || self.pending == ReloadRequest::None {
            return None;
        }
        let work = Refresh {
            nonce: NEXT_REFRESH.fetch_add(1, Ordering::Relaxed),
            request: std::mem::take(&mut self.pending),
        };
        self.flight = Some(work);
        Some(work)
    }

    pub fn accepts(&self, work: Refresh) -> bool {
        self.flight == Some(work)
    }

    /// A known rotation makes any already running read of the previous file obsolete.
    pub fn complete(&mut self, work: Refresh) -> bool {
        if self.flight != Some(work) {
            return false;
        }
        self.flight = None;
        self.pending != ReloadRequest::ReopenPath
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_up_pauses_until_the_last_line() {
        let mut first = FollowMode::Following;
        let second = FollowMode::Following;
        assert!(first.user_scrolled(100.0, 40.0, false));
        assert!(!first.follows());
        assert!(second.follows());
        assert!(!first.user_scrolled(40.0, 60.0, false));
        assert!(first.user_scrolled(60.0, 100.0, true));
        assert!(first.follows());
    }

    #[test]
    fn rename_create_reopens_the_user_path_after_discarding_old_in_flight_append() {
        let mut queue = ReloadQueue::default();
        let log = "Logs/Editor.log";
        queue.request(path_request(log, PathEvent::Changed(log)));
        let old = queue.start().unwrap();
        assert_eq!(old.request, ReloadRequest::CheckAppend);
        queue.request(path_request(
            log,
            PathEvent::Renamed {
                from: log,
                to: "Logs/Archive.log",
            },
        ));
        queue.request(path_request(log, PathEvent::Created(log)));
        queue.request(path_request(log, PathEvent::Changed("Logs/Archive.log")));
        assert_eq!(queue.start(), None);
        assert!(!queue.complete(old));
        let current = queue.start().unwrap();
        assert_eq!(current.request, ReloadRequest::ReopenPath);
        assert!(queue.complete(current));
        assert_eq!(queue.start(), None);
    }

    #[test]
    fn late_results_cannot_match_a_closed_and_reopened_tab_generation() {
        let mut closed = ReloadQueue::default();
        closed.request(ReloadRequest::CheckAppend);
        let late = closed.start().unwrap();
        let mut opened = ReloadQueue::default();
        opened.request(ReloadRequest::CheckAppend);
        let current = opened.start().unwrap();
        assert!(!opened.complete(late));
        assert!(opened.complete(current));
    }

    #[test]
    fn ancestor_rotation_does_not_match_a_neighbor_directory() {
        assert_eq!(
            path_request("Logs/Editor.log", PathEvent::Removed("Logs")),
            ReloadRequest::ReopenPath
        );
        assert_eq!(
            path_request("Logs-old/Editor.log", PathEvent::Removed("Logs")),
            ReloadRequest::None
        );
    }
}
