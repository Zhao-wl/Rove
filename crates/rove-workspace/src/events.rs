//! Broadcast source for [`FileEvents`](crate::contracts::FileEvents).

use crate::contracts::FileEvents;
use rove_base::{Events, Subscription};
use std::sync::{Arc, RwLock};

type Listener = Arc<dyn Fn(FileEvents) + Send + Sync>;

/// File-event broadcast. Listener callbacks run on the producer thread after the snapshot lock is
/// released; they must enqueue and return (ARCHITECTURE §3).
#[derive(Clone, Default)]
pub struct EventSource {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    listeners: RwLock<Vec<Listener>>,
}

impl EventSource {
    /// True when at least one listener is registered. The scanner skips building event batches
    /// (and the path clones they need) when nobody listens, which keeps headless scans cheap.
    pub fn has_listeners(&self) -> bool {
        match self.inner.listeners.read() {
            Ok(guard) => !guard.is_empty(),
            Err(poisoned) => !poisoned.into_inner().is_empty(),
        }
    }

    pub fn emit(&self, event: FileEvents) {
        let listeners: Vec<_> = {
            let guard = match self.inner.listeners.read() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.clone()
        };
        for listener in listeners {
            listener(event.clone());
        }
    }
}

impl Events<FileEvents> for EventSource {
    fn subscribe(&self, listener: Arc<dyn Fn(FileEvents) + Send + Sync>) -> Subscription {
        match self.inner.listeners.write() {
            Ok(mut guard) => guard.push(listener.clone()),
            Err(poisoned) => poisoned.into_inner().push(listener.clone()),
        }
        let inner = Arc::clone(&self.inner);
        Subscription::new(move || match inner.listeners.write() {
            Ok(mut guard) => guard.retain(|candidate| !Arc::ptr_eq(candidate, &listener)),
            Err(poisoned) => poisoned
                .into_inner()
                .retain(|candidate| !Arc::ptr_eq(candidate, &listener)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn dropped_subscription_stops_delivery() {
        let source = EventSource::default();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let subscription = source.subscribe(Arc::new(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
        }));
        source.emit(FileEvents {
            revision: rove_base::Version(1),
            events: Arc::from([]),
        });
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        drop(subscription);
        source.emit(FileEvents {
            revision: rove_base::Version(2),
            events: Arc::from([]),
        });
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }
}
