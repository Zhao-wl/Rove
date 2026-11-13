//! 文档加载走 `rove-buffer`（编码识别、BOM、换行符与 rope 快照）。
use rove_base::*;
use rove_buffer::{open_file, Document, Documents, LoadOptions, ReadOnlyReason};
use rove_config::Settings;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};

/// Read-only reasons from the workspace rules (#15) for a file being opened.
fn read_only_rules(
    root: &std::path::Path,
    resource: &Resource,
    canonical: &std::path::Path,
    settings: &dyn Settings,
) -> Vec<ReadOnlyReason> {
    let Ok(config) = rove_workspace::WorkspaceConfig::from_settings(settings) else {
        return Vec::new();
    };
    let relative = match resource {
        Resource::Workspace(p) => p.as_str().to_string(),
        Resource::File(p) => {
            let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
            canonical
                .strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        }
        Resource::Virtual(p) => p.to_string(),
    };
    let permission = std::fs::metadata(canonical).is_ok_and(|m| m.permissions().readonly());
    config.readonly.evaluate(&relative, permission)
}

/// Live documents are owned by tabs and callers. The cache only indexes their weak identities.
#[derive(Default)]
struct FileCache {
    documents: BTreeMap<PathBuf, Weak<Document>>,
    aliases: BTreeMap<Resource, PathBuf>,
    inflight: BTreeMap<PathBuf, Weak<Mutex<()>>>,
}

impl FileCache {
    fn find(&self, resource: &Resource) -> Option<Arc<Document>> {
        self.aliases
            .get(resource)
            .or(match resource {
                Resource::File(path) => Some(path),
                _ => None,
            })
            .and_then(|path| self.documents.get(path))
            .and_then(Weak::upgrade)
    }

    fn prune(&mut self) {
        self.documents.retain(|_, doc| doc.strong_count() > 0);
        self.aliases
            .retain(|_, path| self.documents.contains_key(path));
        self.inflight.retain(|_, gate| gate.strong_count() > 0);
    }
}

pub struct FileDocuments {
    root: PathBuf,
    settings: Arc<dyn Settings>,
    cache: Arc<Mutex<FileCache>>,
    next: Arc<AtomicU64>,
    size: crate::editors::settings::SizePolicy,
}
impl FileDocuments {
    pub fn new(root: PathBuf, settings: Arc<dyn Settings>) -> Self {
        Self {
            root,
            settings,
            cache: Default::default(),
            next: Arc::new(AtomicU64::new(1)),
            size: Default::default(),
        }
    }

    pub fn with_size_policy(mut self, size: crate::editors::settings::SizePolicy) -> Self {
        self.size = size;
        self
    }
}
impl FileDocuments {
    /// #47：编辑后同一文档换了新快照（新的 `Arc<Document>`，身份不变）：缓存改指向它，
    /// 之后再打开同一文件仍得到这个文档（不会读盘另建一个）。
    pub fn replace(&self, document: &Arc<Document>) {
        let mut index = self.cache.lock().unwrap();
        let path = index
            .documents
            .iter()
            .find(|(_, weak)| weak.upgrade().is_some_and(|d| d.id == document.id))
            .map(|(path, _)| path.clone())
            .or_else(|| index.aliases.get(&document.resource).cloned());
        if let Some(path) = path {
            index.documents.insert(path, Arc::downgrade(document));
        }
    }
}

impl Documents for FileDocuments {
    fn find(&self, r: &Resource) -> Option<Arc<Document>> {
        self.cache.lock().unwrap().find(r)
    }
    fn open(&self, resource: Resource, cancel: Cancellation) -> Task<Arc<Document>> {
        let root = self.root.clone();
        let settings = self.settings.clone();
        let cache = self.cache.clone();
        let next = self.next.clone();
        let policy = self.size;
        Box::pin(async move {
            cancel.check()?;
            if let Some(doc) = cache.lock().unwrap().find(&resource) {
                return Ok(doc);
            }
            let path = match &resource {
                Resource::Workspace(p) => root.join(p.as_str()),
                Resource::File(p) => p.clone(),
                _ => return Err(Error::Unsupported),
            };
            let path = std::fs::canonicalize(path)?;
            // Serialize only reads of the same canonical file; never hold the index lock for I/O.
            let gate = {
                let mut index = cache.lock().unwrap();
                index.prune();
                match index.inflight.get(&path).and_then(Weak::upgrade) {
                    Some(gate) => gate,
                    None => {
                        let gate = Arc::new(Mutex::new(()));
                        index.inflight.insert(path.clone(), Arc::downgrade(&gate));
                        gate
                    }
                }
            };
            let _guard = gate.lock().unwrap();
            cancel.check()?;
            {
                let mut index = cache.lock().unwrap();
                if let Some(doc) = index.documents.get(&path).and_then(Weak::upgrade) {
                    index.aliases.insert(resource, path);
                    return Ok(doc);
                }
            }
            let language = rove_syntax::highlight::language_for_path(&path, Some(&*settings));
            let options = LoadOptions::new(&language, &*settings);
            // 分级（普通/大文件/超大文件）与超大文件的内存映射都在 rove-buffer（#8）。
            let policy = rove_buffer::SizePolicy {
                huge_bytes: policy.huge_bytes,
                optimize: policy.optimize,
            };
            let loaded = open_file(&path, &options, policy)?;
            cancel.check()?;
            // #15 的只读规则（files.readonlyInclude/Exclude/FromPermissions 与
            // rove.files.extraReadonlyInclude）；#20 负责表现。
            let mut loaded = loaded;
            let mut read_only: Vec<ReadOnlyReason> = loaded.metadata.read_only.to_vec();
            read_only.extend(read_only_rules(&root, &resource, &path, &*settings));
            loaded.metadata.read_only = read_only.into();
            let doc = Arc::new(Document {
                id: DocumentId(next.fetch_add(1, Ordering::Relaxed)),
                resource: resource.clone(),
                metadata: loaded.metadata,
                text: loaded.text.source(),
            });
            cancel.check()?;
            let mut index = cache.lock().unwrap();
            index.documents.insert(path.clone(), Arc::downgrade(&doc));
            index.aliases.insert(resource, path);
            Ok(doc)
        })
    }
}
