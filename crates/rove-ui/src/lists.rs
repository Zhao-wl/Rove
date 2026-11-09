//! Virtual list/tree and quick-input interfaces (#18); code view uses a separate custom Element.
use rove_base::*;
use rove_i18n::Message;
use std::sync::Arc;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ItemId(pub Arc<str>);
/// label/detail are content data; fixed UI wording must be resolved through the string table.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: ItemId,
    pub label: Arc<str>,
    pub detail: Option<Arc<str>>,
    pub depth: u32,
    pub expandable: bool,
    pub expanded: bool,
    pub disabled: bool,
}
/// Immutable Send + Sync flattened visible tree/list. O(1) count, O(log n + visible rows) slice;
/// never materialize all rows just to paint a viewport. Index is transient; selection uses ItemId.
/// Rows clip at end; start>count or inverted range => OutOfBounds/InvalidInput.
pub trait ListModel: Send + Sync {
    fn revision(&self) -> Version;
    fn count(&self) -> usize;
    fn rows(&self, visible: std::ops::Range<usize>) -> Result<Arc<[Row]>>;
}
#[derive(Clone, Copy, Debug)]
pub enum Navigation {
    Next,
    Previous,
    First,
    Last,
    PageUp,
    PageDown,
    Parent,
    Expand,
    Collapse,
    Accept,
    Cancel,
}
/// UI-thread control; navigation O(visible rows), filter enqueues background matching and cancels
/// older generation. Stable IDs preserve focus/selection across refresh. Unknown IDs => NotFound.
pub trait ListController {
    fn navigate(&mut self, action: Navigation) -> Result<()>;
    fn filter(&mut self, text: Arc<str>, cancel: Cancellation) -> Result<()>;
    fn selection(&self) -> Arc<[ItemId]>;
    fn expand(&mut self, id: &ItemId, expanded: bool) -> Result<()>;
}
pub struct EmptyList;
impl ListModel for EmptyList {
    fn revision(&self) -> Version {
        Version(0)
    }
    fn count(&self) -> usize {
        0
    }
    fn rows(&self, r: std::ops::Range<usize>) -> Result<Arc<[Row]>> {
        if r.start > r.end {
            Err(Error::InvalidInput)
        } else if r.start > 0 {
            Err(Error::OutOfBounds)
        } else {
            Ok(Arc::from([]))
        }
    }
}
impl ListController for EmptyList {
    fn navigate(&mut self, _: Navigation) -> Result<()> {
        Ok(())
    }
    fn filter(&mut self, _: Arc<str>, c: Cancellation) -> Result<()> {
        c.check()
    }
    fn selection(&self) -> Arc<[ItemId]> {
        Arc::from([])
    }
    fn expand(&mut self, _: &ItemId, _: bool) -> Result<()> {
        Err(Error::NotFound)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickInputId(pub u64);
pub struct QuickInputOptions {
    pub placeholder: Message,
    pub initial: Arc<str>,
    pub items: Arc<dyn ListModel>,
    pub multi_select: bool,
}
#[derive(Clone, Debug)]
pub enum QuickInputEvent {
    Changed {
        id: QuickInputId,
        text: Arc<str>,
    },
    Accepted {
        id: QuickInputId,
        items: Arc<[ItemId]>,
        text: Arc<str>,
    },
    Cancelled(QuickInputId),
}
/// UI-thread nonblocking quick input, O(visible rows). Subscribe first, then show. Enter accepts,
/// Escape cancels; close/cancel never masquerades as acceptance. Replacement is generation checked
/// by caller; #24 owns scoring and prefix parsing, not this generic component.
pub trait QuickInput {
    fn show(&mut self, options: QuickInputOptions) -> Result<QuickInputId>;
    fn update(&mut self, id: QuickInputId, items: Arc<dyn ListModel>) -> Result<()>;
    fn close(&mut self, id: QuickInputId) -> Result<()>;
    fn events(&self) -> &dyn Events<QuickInputEvent>;
}
pub struct EmptyQuickInput;
impl QuickInput for EmptyQuickInput {
    fn show(&mut self, _: QuickInputOptions) -> Result<QuickInputId> {
        Err(Error::Unsupported)
    }
    fn update(&mut self, _: QuickInputId, _: Arc<dyn ListModel>) -> Result<()> {
        Err(Error::NotFound)
    }
    fn close(&mut self, _: QuickInputId) -> Result<()> {
        Err(Error::NotFound)
    }
    fn events(&self) -> &dyn Events<QuickInputEvent> {
        &NoEvents
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_view_reads_only_viewport() {
        let mut list = EmptyList;
        assert!(list.rows(0..30).unwrap().is_empty());
        list.navigate(Navigation::Next).unwrap();
        list.filter("A".into(), Cancellation::default()).unwrap();
        assert!(list.selection().is_empty());
        assert!(list.expand(&ItemId("a".into()), true).is_err());
    }
    #[test]
    fn palette_uses_generic_quick_input() {
        let mut q = EmptyQuickInput;
        let _sub = q.events().subscribe(Arc::new(|_| {}));
        assert_eq!(
            q.show(QuickInputOptions {
                placeholder: Message::key("common.loading"),
                initial: "".into(),
                items: Arc::new(EmptyList),
                multi_select: false
            }),
            Err(Error::Unsupported)
        );
    }
}
