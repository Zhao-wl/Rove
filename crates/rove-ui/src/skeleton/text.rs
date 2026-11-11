//! 文档加载走 `rove-buffer`（编码识别、BOM、换行符与 rope 快照）。
use rove_base::*;
use rove_buffer::{
    open_bytes, Document, Documents, LoadOptions, ReadOnlyReason, SizeClass, TextSource,
};
use rove_config::{DefaultSettings, SettingRegistry, Settings};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};

/// 装配用的默认设置：#13 的设置层合并前，用契约里的 `DefaultSettings`
/// 提供 `files.encoding` / `files.autoGuessEncoding` / 候选编码的缺省值。
pub fn default_settings() -> Arc<dyn Settings> {
    let mut registry = SettingRegistry::default();
    rove_buffer::settings::register(&mut registry).expect("编码设置键必须在启动时登记");
    crate::code_view::options::register(&mut registry).expect("代码视图设置键必须在启动时登记");
    Arc::new(DefaultSettings(registry))
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
            let bytes = std::fs::read(&path)?;
            let bytes_on_disk = bytes.len() as u64;
            cancel.check()?;
            let language = rove_syntax::highlight::language_for_path(&path, None);
            let mut options = LoadOptions::new(&language, &*settings);
            if bytes_on_disk >= policy.huge_bytes {
                options.size = SizeClass::Huge;
                options.read_only = Arc::from([ReadOnlyReason::Session, ReadOnlyReason::HugeFile]);
            }
            let mut loaded = open_bytes(&bytes, &options)?;
            if options.size == SizeClass::Normal
                && policy.optimize
                && (bytes_on_disk >= 20 * 1024 * 1024 || loaded.text.lines().known >= 300_000)
            {
                loaded.metadata.size = SizeClass::Large;
            }
            let doc = Arc::new(Document {
                id: DocumentId(next.fetch_add(1, Ordering::Relaxed)),
                resource: resource.clone(),
                metadata: loaded.metadata,
                text: loaded.text,
            });
            cancel.check()?;
            let mut index = cache.lock().unwrap();
            index.documents.insert(path.clone(), Arc::downgrade(&doc));
            index.aliases.insert(resource, path);
            Ok(doc)
        })
    }
}
