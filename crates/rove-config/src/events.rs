//! 简单的事件分发：回调在发布线程上、释放内部锁以后执行（ARCHITECTURE §3 第 5 条）。

use rove_base::{Events, Subscription};
use std::sync::{Arc, Mutex, PoisonError};

type Listener<T> = Arc<dyn Fn(T) + Send + Sync>;
type Listeners<T> = Mutex<Vec<(u64, Listener<T>)>>;

pub(crate) struct EventHub<T> {
    listeners: Arc<Listeners<T>>,
    next: Mutex<u64>,
}

impl<T> Default for EventHub<T> {
    fn default() -> Self {
        Self {
            listeners: Arc::default(),
            next: Mutex::new(0),
        }
    }
}

impl<T: Clone> EventHub<T> {
    pub fn emit(&self, value: T) {
        let listeners: Vec<Listener<T>> = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for listener in listeners {
            listener(value.clone());
        }
    }
}

impl<T: 'static> Events<T> for EventHub<T> {
    fn subscribe(&self, listener: Arc<dyn Fn(T) + Send + Sync>) -> Subscription {
        let id = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, listener));
        let weak = Arc::downgrade(&self.listeners);
        Subscription::new(move || {
            if let Some(listeners) = weak.upgrade() {
                listeners
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .retain(|(i, _)| *i != id);
            }
        })
    }
}
