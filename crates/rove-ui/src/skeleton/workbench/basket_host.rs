use super::{Context, FileEvent, Workbench};
use crate::context_basket::{model::Basket, persistence::BasketStore};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

pub(super) struct WindowBasket {
    store: Option<BasketStore>,
    pub(super) contents: Basket,
    loaded: bool,
    revision: u64,
    written: Arc<Mutex<u64>>,
}

impl WindowBasket {
    pub(super) fn new(root: &Path) -> Self {
        let store = (!root.as_os_str().is_empty()).then(|| BasketStore::for_workspace(root));
        Self {
            loaded: store.is_none(),
            store,
            contents: Basket::default(),
            revision: 0,
            written: Arc::default(),
        }
    }

    fn write_job(&self) -> Option<impl FnOnce() + Send + 'static> {
        let store = self.store.clone()?;
        let revision = self.revision;
        if !self.loaded || revision == 0 {
            return None;
        }
        let written = self.written.clone();
        let snapshot = self.contents.clone();
        Some(move || {
            let mut latest = written.lock().unwrap_or_else(|error| error.into_inner());
            if *latest >= revision {
                return;
            }
            match store.save(&snapshot) {
                Ok(()) => *latest = revision,
                Err(error) => eprintln!("context basket: {error}"),
            }
        })
    }
}

impl Workbench {
    pub(super) fn restore_basket(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.basket.store.clone() else {
            return;
        };
        let task = cx.background_executor().spawn(async move { store.load() });
        cx.spawn(async move |this, cx| {
            let contents = task.await;
            let _ = this.update(cx, |view, cx| {
                view.basket.contents = contents;
                view.basket.loaded = true;
                cx.notify();
            });
        })
        .detach();
    }

    fn basket_changed(&mut self, cx: &mut Context<Self>) {
        self.basket.revision += 1;
        if let Some(write) = self.basket.write_job() {
            cx.background_executor()
                .spawn(async move { write() })
                .detach();
        }
        cx.notify();
    }

    pub(super) fn flush_basket(&self) {
        if let Some(write) = self.basket.write_job() {
            write();
        }
    }

    pub(super) fn basket_workspace_files_changed(
        &mut self,
        events: &[FileEvent],
        cx: &mut Context<Self>,
    ) {
        if !self.basket.loaded || self.basket.contents.items().is_empty() {
            return;
        }
        let before = self.basket.contents.clone();
        for event in events {
            match event {
                FileEvent::Renamed { from, to } => self.basket.contents.rename(from, to),
                FileEvent::Removed(path) => self.basket.contents.mark_removed(path),
                _ => {}
            }
        }
        if self.basket.contents != before {
            self.basket_changed(cx);
        }
    }
}
