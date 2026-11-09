//! Structured document models reuse the syntax symbol tree; implemented by #60.
use rove_base::*;
use rove_syntax::Symbol;
use std::sync::Arc;
/// UI-independent structure node with lazy children. None means not loaded; Some(empty) is a leaf.
#[derive(Clone, Debug)]
pub struct StructureNode {
    pub symbol: Symbol,
    pub path: Arc<str>,
    pub value_preview: Arc<str>,
    pub children: Option<Arc<[StructureNode]>>,
}
/// Cached O(log nodes + returned children) access, Send + Sync; build/expand on worker threads.
/// NotReady indicates children still building; OutOfBounds/NotFound never become an empty success.
pub trait Structure: Send + Sync {
    fn version(&self) -> Version;
    fn roots(&self) -> Result<Arc<[StructureNode]>>;
}
pub struct EmptyStructure;
impl Structure for EmptyStructure {
    fn version(&self) -> Version {
        Version(0)
    }
    fn roots(&self) -> Result<Arc<[StructureNode]>> {
        Ok(Arc::from([]))
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
