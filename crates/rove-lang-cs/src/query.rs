//! C# query contracts for #38. Independent of index.rs and its v1/v2 storage representation.
use rove_base::*;
use rove_syntax::Symbol;
use std::sync::Arc;
/// Exact compiler binding, approximate candidate, or syntax-only result. Never silently upgraded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    Exact,
    Approximate,
    Syntax,
}
#[derive(Clone, Debug)]
pub enum ApproximationReason {
    IndexNotReady,
    ModifiedRegion,
    ComponentUnavailable,
    IndexDisabled,
    Other(Arc<str>),
}
/// Every response carries provenance, even an empty response. version is Some for file queries and
/// None for workspace-symbol queries (which use index_generation instead);
/// index_generation lets UI invalidate responses from replaced indexes. Precision is per batch;
/// mixed results must use the weakest precision. reason is populated for non-exact responses.
#[derive(Clone, Debug)]
pub struct QueryResult<T> {
    pub value: T,
    pub precision: Precision,
    pub reason: Option<ApproximationReason>,
    pub version: Option<Version>,
    pub index_generation: u64,
}
/// Source or generated metadata target, always UTF-8 coordinates; v1 UTF-16 is converted by #38.
#[derive(Clone, Debug)]
pub struct Target {
    pub resource: Resource,
    pub range: TextRange,
    pub selection: TextRange,
}
#[derive(Clone, Debug)]
pub struct QueryPoint {
    pub resource: Resource,
    pub position: Position,
    pub version: Version,
}
#[derive(Clone, Copy, Debug)]
pub enum NavigationKind {
    Definition,
    TypeDefinition,
    References,
    Implementations,
    DerivedTypes,
    BaseTypes,
    Overrides,
}
#[derive(Clone, Debug)]
pub struct Hover {
    pub signature: Arc<str>,
    pub documentation: Arc<str>,
    pub range: TextRange,
}
#[derive(Clone, Debug)]
pub struct WorkspaceSymbol {
    pub name: Arc<str>,
    pub container: Arc<str>,
    pub target: Target,
}
/// Single-line UTF-8 byte length (not UTF-16 units or scalar count). Modifiers/type use VS Code IDs.
#[derive(Clone, Debug)]
pub struct SemanticToken {
    pub line: Line,
    pub column: Utf8Column,
    pub length_bytes: u64,
    pub token_type: Arc<str>,
    pub modifiers: Arc<[Arc<str>]>,
}
/// Synchronous, Send + Sync query service; ALL methods run on background workers because mmap can
/// fault (M0 cold HDD max 2.8s). Hot indexed lookup O(log n + results), definition <16ms, refs <50ms;
/// these are budgets, not guarantees of the empty provider. Check cancellation during enumeration.
/// Stale input => Stale, unavailable layer => NotReady/Unsupported, legitimate no match => empty.
/// Returned owned/shared data must keep backing mappings alive across index replacement. Never
/// copy whole source files. File outputs refer to requested version; edited regions fall back.
pub trait CsQueries: Send + Sync {
    fn navigate(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[Target]>>>;
    fn hover(
        &self,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Option<Hover>>>;
    fn file_symbols(
        &self,
        file: &Resource,
        version: Version,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[Symbol]>>>;
    fn workspace_symbols(
        &self,
        query: &str,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[WorkspaceSymbol]>>>;
    fn semantic_tokens(
        &self,
        file: &Resource,
        version: Version,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[SemanticToken]>>>;
    fn inactive_regions(
        &self,
        file: &Resource,
        version: Version,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[TextRange]>>>;
}
/// Fixed unavailable provider: distinguishes unavailable semantic data from exact empty results.
pub struct EmptyCsQueries;
impl CsQueries for EmptyCsQueries {
    fn navigate(
        &self,
        _: NavigationKind,
        _: &QueryPoint,
        c: &Cancellation,
    ) -> Result<QueryResult<Arc<[Target]>>> {
        c.check()?;
        Err(Error::NotReady)
    }
    fn hover(&self, _: &QueryPoint, c: &Cancellation) -> Result<QueryResult<Option<Hover>>> {
        c.check()?;
        Err(Error::NotReady)
    }
    fn file_symbols(
        &self,
        _: &Resource,
        _: Version,
        c: &Cancellation,
    ) -> Result<QueryResult<Arc<[Symbol]>>> {
        c.check()?;
        Err(Error::NotReady)
    }
    fn workspace_symbols(
        &self,
        _: &str,
        c: &Cancellation,
    ) -> Result<QueryResult<Arc<[WorkspaceSymbol]>>> {
        c.check()?;
        Err(Error::NotReady)
    }
    fn semantic_tokens(
        &self,
        _: &Resource,
        _: Version,
        c: &Cancellation,
    ) -> Result<QueryResult<Arc<[SemanticToken]>>> {
        c.check()?;
        Err(Error::NotReady)
    }
    fn inactive_regions(
        &self,
        _: &Resource,
        _: Version,
        c: &Cancellation,
    ) -> Result<QueryResult<Arc<[TextRange]>>> {
        c.check()?;
        Err(Error::NotReady)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_ui_can_distinguish_unavailable_and_cancelled() {
        let q: &dyn CsQueries = &EmptyCsQueries;
        let p = QueryPoint {
            resource: Resource::Workspace(WorkspacePath::new("Assets/A.cs").unwrap()),
            position: Position::default(),
            version: Version(3),
        };
        let c = Cancellation::default();
        for kind in [
            NavigationKind::Definition,
            NavigationKind::TypeDefinition,
            NavigationKind::References,
            NavigationKind::Implementations,
            NavigationKind::DerivedTypes,
            NavigationKind::BaseTypes,
            NavigationKind::Overrides,
        ] {
            assert!(matches!(q.navigate(kind, &p, &c), Err(Error::NotReady)));
        }
        assert!(matches!(q.hover(&p, &c), Err(Error::NotReady)));
        assert!(q.file_symbols(&p.resource, p.version, &c).is_err());
        assert!(q.workspace_symbols("A", &c).is_err());
        assert!(q.semantic_tokens(&p.resource, p.version, &c).is_err());
        assert!(q.inactive_regions(&p.resource, p.version, &c).is_err());
        c.cancel();
        assert!(matches!(
            q.navigate(NavigationKind::Definition, &p, &c),
            Err(Error::Cancelled)
        ));
    }
}
