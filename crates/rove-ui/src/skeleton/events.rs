use rove_base::{Events, Subscription};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
type Listener<T> = Arc<dyn Fn(T) + Send + Sync>;
type Listeners<T> = Arc<Mutex<(u64, BTreeMap<u64, Listener<T>>)>>;
pub struct EventBus<T> {
    listeners: Listeners<T>,
}
impl<T: Clone + Send + 'static> EventBus<T> {
    pub fn emit(&self, value: T) {
        let listeners: Vec<_> = self.listeners.lock().unwrap().1.values().cloned().collect();
        for listener in listeners {
            listener(value.clone());
        }
    }
}
impl<T: Send + 'static> Events<T> for EventBus<T> {
    fn subscribe(&self, listener: Listener<T>) -> Subscription {
        let mut state = self.listeners.lock().unwrap();
        state.0 += 1;
        let id = state.0;
        state.1.insert(id, listener);
        let weak = Arc::downgrade(&self.listeners);
        Subscription::new(move || {
            if let Some(state) = weak.upgrade() {
                state.lock().unwrap().1.remove(&id);
            }
        })
    }
}

impl<T> EventBus<T> {
    pub fn empty() -> Self {
        Self {
            listeners: Arc::new(Mutex::new((0, BTreeMap::new()))),
        }
    }
}
