//! 文档加载走 `rove-buffer`（编码识别、BOM、换行符与 rope 快照）。
//! SKELETON(editors)：文档缓存与打开策略仍由 #17 替换；这里只保留最简缓存。
use rove_base::*;
use rove_buffer::{open_path, Document, Documents, LoadOptions};
use rove_config::{DefaultSettings, SettingRegistry, Settings};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
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

pub struct FileDocuments {
    root: PathBuf,
    settings: Arc<dyn Settings>,
    cache: Arc<Mutex<BTreeMap<Resource, Arc<Document>>>>,
    next: Arc<AtomicU64>,
}

impl FileDocuments {
    pub fn new(root: PathBuf, settings: Arc<dyn Settings>) -> Self {
        Self {
            root,
            settings,
            cache: Default::default(),
            next: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl Documents for FileDocuments {
    fn find(&self, r: &Resource) -> Option<Arc<Document>> {
        self.cache.lock().unwrap().get(r).cloned()
    }

    fn open(&self, resource: Resource, cancel: Cancellation) -> Task<Arc<Document>> {
        let root = self.root.clone();
        let settings = self.settings.clone();
        let cache = self.cache.clone();
        let next = self.next.clone();
        Box::pin(async move {
            cancel.check()?;
            if let Some(doc) = cache.lock().unwrap().get(&resource).cloned() {
                return Ok(doc);
            }
            let path = match &resource {
                Resource::Workspace(p) => root.join(p.as_str()),
                Resource::File(p) => p.clone(),
                _ => return Err(Error::Unsupported),
            };
            let path = std::fs::canonicalize(path)?;
            let identity = Resource::File(path.clone());
            {
                let mut docs = cache.lock().unwrap();
                if let Some(doc) = docs.get(&identity).cloned() {
                    docs.insert(resource, doc.clone());
                    return Ok(doc);
                }
            }
            cancel.check()?;
            let language = rove_syntax::highlight::language_for_path(&path, None);
            let loaded = open_path(&path, &LoadOptions::new(&language, &*settings))?;
            cancel.check()?;
            let mut docs = cache.lock().unwrap();
            let doc = docs
                .entry(identity)
                .or_insert_with(|| {
                    let text: Arc<dyn rove_buffer::TextSource> = loaded.text;
                    Arc::new(Document {
                        id: DocumentId(next.fetch_add(1, Ordering::Relaxed)),
                        resource: resource.clone(),
                        metadata: loaded.metadata,
                        text,
                    })
                })
                .clone();
            docs.insert(resource, doc.clone());
            Ok(doc)
        })
    }
}
