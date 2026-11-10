//! Outline consumer example: expand via the trait without eagerly building descendants.
use rove_base::*;
use rove_docview::{EmptyStructure, Structure, StructureNode, StructureNodeId};
use rove_syntax::{Symbol, SymbolKind};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
};
fn ready<T>(mut task: Task<T>) -> Result<T> {
    match task.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("fixture should finish in one poll"),
    }
}
fn node(id: u64, leaf: bool) -> StructureNode {
    let range = Span::new(Position::default(), Position::default()).unwrap();
    StructureNode {
        id: StructureNodeId(id),
        symbol: Symbol {
            name: "same".into(),
            kind: SymbolKind::Object,
            range: range.clone(),
            selection: range,
            children: Arc::from([]),
        },
        // Intentionally identical paths: display/copy paths cannot serve as identity.
        path: "$.same".into(),
        value_preview: "".into(),
        children: if leaf { Some(Arc::from([])) } else { None },
    }
}
struct LazyFixture {
    constructed: Arc<AtomicUsize>,
}
impl Structure for LazyFixture {
    fn version(&self) -> Version {
        Version(7)
    }
    fn roots(&self) -> Result<Arc<[StructureNode]>> {
        Ok(Arc::from([node(1, false)]))
    }
    fn children(
        &self,
        id: StructureNodeId,
        version: Version,
        cancel: Cancellation,
    ) -> Task<Arc<[StructureNode]>> {
        let constructed = self.constructed.clone();
        Box::pin(async move {
            cancel.check()?;
            if version != Version(7) {
                return Err(Error::Stale);
            }
            match id.0 {
                1 => {
                    constructed.fetch_add(1, Ordering::Relaxed);
                    Ok(Arc::from([node(2, false)]))
                }
                2 => {
                    constructed.fetch_add(1, Ordering::Relaxed);
                    Ok(Arc::from([node(3, true)]))
                }
                3 => Ok(Arc::from([])),
                _ => Err(Error::NotFound),
            }
        })
    }
}
#[test]
fn outline_loads_only_the_expanded_node() {
    let count = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn Structure> = Arc::new(LazyFixture {
        constructed: count.clone(),
    });
    let roots = provider.roots().unwrap();
    assert!(roots[0].children.is_none());
    let task = provider.children(roots[0].id, provider.version(), Cancellation::default());
    assert_eq!(
        count.load(Ordering::Relaxed),
        0,
        "request creation must not build children"
    );
    let children = ready(task).unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert!(children[0].children.is_none(), "grandchildren remain lazy");
    assert!(children[0].symbol.children.is_empty());
    assert_eq!(children[0].path, roots[0].path);
    assert_ne!(children[0].id, roots[0].id);
    assert!(
        roots[0].children.is_none(),
        "previous snapshots remain unchanged"
    );
    let grandchildren =
        ready(provider.children(children[0].id, provider.version(), Cancellation::default()))
            .unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert!(grandchildren[0].children.as_ref().unwrap().is_empty());
    assert!(ready(provider.children(
        grandchildren[0].id,
        provider.version(),
        Cancellation::default()
    ))
    .unwrap()
    .is_empty());
}
#[test]
fn invalid_or_cancelled_expansion_never_builds_nodes() {
    let count = Arc::new(AtomicUsize::new(0));
    let provider: &dyn Structure = &LazyFixture {
        constructed: count.clone(),
    };
    assert!(matches!(
        ready(provider.children(StructureNodeId(1), Version(6), Cancellation::default())),
        Err(Error::Stale)
    ));
    assert!(matches!(
        ready(provider.children(StructureNodeId(99), Version(7), Cancellation::default())),
        Err(Error::NotFound)
    ));
    let cancel = Cancellation::default();
    let task = provider.children(StructureNodeId(1), Version(7), cancel.clone());
    cancel.cancel(); // Collapse after requesting but before the worker polls.
    assert!(matches!(ready(task), Err(Error::Cancelled)));
    assert_eq!(count.load(Ordering::Relaxed), 0);
}
#[test]
fn empty_provider_does_not_invent_a_leaf_for_an_unknown_node() {
    let provider: &dyn Structure = &EmptyStructure;
    assert!(matches!(
        ready(provider.children(StructureNodeId(1), Version(0), Cancellation::default())),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        ready(provider.children(StructureNodeId(1), Version(1), Cancellation::default())),
        Err(Error::Stale)
    ));
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        ready(provider.children(StructureNodeId(1), Version(0), cancel)),
        Err(Error::Cancelled)
    ));
}
