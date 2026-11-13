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

/// 虚拟文档（没有磁盘文件的文档，如 #42 的元数据视图）的内容来源。`name` 是 `Resource::Virtual` 的整串；
/// 不认识这个方案时返回 `None`（打开按 `Unsupported` 失败）。
pub trait VirtualSource: Send + Sync {
    fn load(&self, name: &str) -> Option<Result<VirtualContent>>;
}

/// 虚拟文档的内容。
pub struct VirtualContent {
    pub text: String,
    pub language: Arc<str>,
    pub read_only: Vec<ReadOnlyReason>,
}

/// Live documents are owned by tabs and callers. The cache only indexes their weak identities.
#[derive(Default)]
struct FileCache {
    documents: BTreeMap<PathBuf, Weak<Document>>,
    /// 虚拟文档按资源名缓存（弱引用，由标签持有）。
    virtuals: BTreeMap<Arc<str>, Weak<Document>>,
    aliases: BTreeMap<Resource, PathBuf>,
    inflight: BTreeMap<PathBuf, Weak<Mutex<()>>>,
}

impl FileCache {
    fn find(&self, resource: &Resource) -> Option<Arc<Document>> {
        if let Resource::Virtual(name) = resource {
            return self.virtuals.get(name).and_then(Weak::upgrade);
        }
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
        self.virtuals.retain(|_, doc| doc.strong_count() > 0);
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
    /// 虚拟文档的内容来源，按登记顺序依次询问（#42 元数据视图、#28 欢迎页）。
    virtual_sources: Vec<Arc<dyn VirtualSource>>,
    /// #27：编码是否由自动识别得出。冻结的 `DocumentMetadata` 没有这个字段（见
    /// `rove_buffer::LoadedText` 的说明），状态栏的「自动识别」悬停提示只能从这里取。
    guessed: Arc<Mutex<BTreeMap<DocumentId, bool>>>,
}

/// 依次询问各个来源，第一个认领这个资源名的胜出。
fn load_virtual(sources: &[Arc<dyn VirtualSource>], name: &str) -> Option<Result<VirtualContent>> {
    sources.iter().find_map(|s| s.load(name))
}
impl FileDocuments {
    pub fn new(root: PathBuf, settings: Arc<dyn Settings>) -> Self {
        Self {
            root,
            settings,
            cache: Default::default(),
            next: Arc::new(AtomicU64::new(1)),
            size: Default::default(),
            virtual_sources: Vec::new(),
            guessed: Default::default(),
        }
    }

    /// #27：`document` 的编码是不是自动识别出来的。
    pub fn encoding_guessed(&self, document: DocumentId) -> bool {
        self.guessed
            .lock()
            .unwrap()
            .get(&document)
            .copied()
            .unwrap_or(false)
    }

    /// #27：「通过编码重新打开」。用 `encoding`（VS Code 的编码 ID）重读同一个文件，
    /// **文档身份不变**（标签、滚动位置与选区由 `EditorService::reload_document` 保持），
    /// 返回新的快照。虚拟文档与不认识的编码返回 `Unsupported`。
    pub fn reopen_with_encoding(
        &self,
        document: &Arc<Document>,
        encoding: &str,
    ) -> Result<Arc<Document>> {
        if rove_buffer::settings::encoding_from_id(encoding).is_none() {
            return Err(Error::Unsupported);
        }
        let resource = document.resource.clone();
        let path = match &resource {
            Resource::Workspace(p) => self.root.join(p.as_str()),
            Resource::File(p) => p.clone(),
            Resource::Virtual(_) => return Err(Error::Unsupported),
        };
        let path = std::fs::canonicalize(path)?;
        let language = document.metadata.language.clone();
        let mut options = LoadOptions::new(&language, &*self.settings);
        options.explicit_encoding = Some(encoding);
        // 用户明确选了编码：即使解不出合法文本也按 lossy 显示（VS Code 的「通过编码重新打开」
        // 就是用来看乱码、再换一个编码的），不要报「二进制或不支持的编码」。
        options.force_open = true;
        let policy = rove_buffer::SizePolicy {
            huge_bytes: self.size.huge_bytes,
            optimize: self.size.optimize,
        };
        let mut loaded = open_file(&path, &options, policy)?;
        let mut read_only: Vec<ReadOnlyReason> = loaded.metadata.read_only.to_vec();
        read_only.extend(read_only_rules(
            &self.root,
            &resource,
            &path,
            &*self.settings,
        ));
        loaded.metadata.read_only = read_only.into();
        let reopened = Arc::new(Document {
            id: document.id,
            resource,
            metadata: loaded.metadata,
            text: loaded.text.source(),
        });
        self.guessed
            .lock()
            .unwrap()
            .insert(document.id, loaded.detection.guessed);
        let mut index = self.cache.lock().unwrap();
        index.documents.insert(path, Arc::downgrade(&reopened));
        Ok(reopened)
    }


    /// 追加一个虚拟文档的内容来源（#42 的元数据视图、#28 的欢迎页）。先登记的先被询问。
    pub fn with_virtual_source(mut self, source: Arc<dyn VirtualSource>) -> Self {
        self.virtual_sources.push(source);
        self
    }

    /// 虚拟文档重新取内容：文档身份不变，文本是新的快照（版本更新）。没有打开的文档返回 `None`。
    pub fn reload_virtual(&self, resource: &Resource) -> Result<Option<Arc<Document>>> {
        let Resource::Virtual(name) = resource else {
            return Ok(None);
        };
        let Some(old) = self.cache.lock().unwrap().find(resource) else {
            return Ok(None);
        };
        let Some(content) = load_virtual(&self.virtual_sources, name) else {
            return Ok(None);
        };
        let doc = Arc::new(virtual_document(old.id, resource, content?));
        self.cache
            .lock()
            .unwrap()
            .virtuals
            .insert(name.clone(), Arc::downgrade(&doc));
        Ok(Some(doc))
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
        let virtual_sources = self.virtual_sources.clone();
        let guessed = self.guessed.clone();
        Box::pin(async move {
            cancel.check()?;
            if let Some(doc) = cache.lock().unwrap().find(&resource) {
                return Ok(doc);
            }
            let path = match &resource {
                Resource::Workspace(p) => root.join(p.as_str()),
                Resource::File(p) => p.clone(),
                Resource::Virtual(name) => {
                    let content =
                        load_virtual(&virtual_sources, name).ok_or(Error::Unsupported)??;
                    let id = DocumentId(next.fetch_add(1, Ordering::Relaxed));
                    let doc = Arc::new(virtual_document(id, &resource, content));
                    cache
                        .lock()
                        .unwrap()
                        .virtuals
                        .insert(name.clone(), Arc::downgrade(&doc));
                    return Ok(doc);
                }
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
            let detection_guessed = loaded.detection.guessed;
            let mut read_only: Vec<ReadOnlyReason> = loaded.metadata.read_only.to_vec();
            read_only.extend(read_only_rules(&root, &resource, &path, &*settings));
            loaded.metadata.read_only = read_only.into();
            let doc = Arc::new(Document {
                id: DocumentId(next.fetch_add(1, Ordering::Relaxed)),
                resource: resource.clone(),
                metadata: loaded.metadata,
                text: loaded.text.source(),
            });
            // #27：状态栏的「自动识别」悬停提示（元信息里没有这个字段）。
            guessed.lock().unwrap().insert(doc.id, detection_guessed);
            cancel.check()?;
            let mut index = cache.lock().unwrap();
            index.documents.insert(path.clone(), Arc::downgrade(&doc));
            index.aliases.insert(resource, path);
            Ok(doc)
        })
    }
}

/// 虚拟文档：UTF-8、LF，普通大小，只读原因由来源给出。
fn virtual_document(id: DocumentId, resource: &Resource, content: VirtualContent) -> Document {
    Document {
        id,
        resource: resource.clone(),
        metadata: rove_buffer::DocumentMetadata {
            language: content.language,
            encoding: rove_buffer::Encoding::Utf8,
            bom: false,
            line_endings: rove_buffer::LineEndings::Uniform(rove_buffer::Eol::Lf),
            read_only: content.read_only.into(),
            size: rove_buffer::SizeClass::Normal,
        },
        text: Arc::new(rove_buffer::RopeText::from_text(&content.text)),
    }
}
