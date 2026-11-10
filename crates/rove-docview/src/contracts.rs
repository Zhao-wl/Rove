//! Structured document models reuse the syntax symbol tree; implemented by #60.
use rove_base::*;
use rove_syntax::Symbol;
use std::sync::Arc;
/// Opaque identity unique within one Structure provider and Version. Not a display path: duplicate
/// JSON keys and YAML documents may have identical paths. Do not pass IDs to another provider instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StructureNodeId(pub u64);
/// UI-independent node. None means not loaded; Some(empty) is a known leaf. `symbol.children`
/// must be empty: structural descendants live only here, not in a second eagerly built symbol tree.
/// Loading children returns a new immutable slice; it does not mutate previously returned nodes.
#[derive(Clone, Debug)]
pub struct StructureNode {
    pub id: StructureNodeId,
    pub symbol: Symbol,
    pub path: Arc<str>,
    pub value_preview: Arc<str>,
    pub children: Option<Arc<[StructureNode]>>,
}
/// Versioned structure provider, Send + Sync. roots is cached O(returned roots), UI-safe, no parsing;
/// NotReady means root construction is pending. Retain the provider and its version with node IDs.
/// Expand only requested direct children, never recursively materialize the whole document.
pub trait Structure: Send + Sync {
    fn version(&self) -> Version;
    fn roots(&self) -> Result<Arc<[StructureNode]>>;
    /// Enqueue/poll this task on a worker: invocation must return promptly without parsing or I/O.
    /// Cached lookup O(log nodes + returned children); uncached cost is proportional to the requested
    /// node's source span, with periodic cancellation checks. Unexpanded descendants stay None.
    /// expected_version must match the provider's source revision, including at completion;
    /// otherwise Stale. Unknown node => NotFound; known leaf => Ok(empty); cancellation => Cancelled.
    /// NotReady is allowed when prerequisite parse data is unavailable, never an empty success.
    /// Returned nodes belong to expected_version; consumers check provider identity/version again
    /// before applying them. A successful load remains queryable; callers may retain the returned
    /// slice instead of re-reading roots. Collapsing a node may cancel a pending request.
    fn children(
        &self,
        node: StructureNodeId,
        expected_version: Version,
        cancel: Cancellation,
    ) -> Task<Arc<[StructureNode]>>;
}
pub struct EmptyStructure;
impl Structure for EmptyStructure {
    fn version(&self) -> Version {
        Version(0)
    }
    fn roots(&self) -> Result<Arc<[StructureNode]>> {
        Ok(Arc::from([]))
    }
    fn children(
        &self,
        _: StructureNodeId,
        expected_version: Version,
        cancel: Cancellation,
    ) -> Task<Arc<[StructureNode]>> {
        Box::pin(async move {
            cancel.check()?;
            if expected_version != Version(0) {
                return Err(Error::Stale);
            }
            Err(Error::NotFound)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outline_reads_empty_structure() {
        let s: &dyn Structure = &EmptyStructure;
        assert!(s.roots().unwrap().is_empty());
    }
}
