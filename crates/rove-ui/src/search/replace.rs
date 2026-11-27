//! #62：把已显示的搜索结果核对成编辑事务。编辑器之外的文件经 #49 原子写入。
use crate::find::replace::ReplacePattern;
use rove_base::{ByteOffset, ByteRange, Cancellation, Error};
use rove_buffer::{
    edit::{AppliedEdit, Bias, EditBuffer, Replacement, Transaction},
    open_bytes,
    save::{
        content_hash, encode, write_atomic_with, DiskStamp, Unencodable, WriteError, WriteOptions,
    },
    Encoding, LoadOptions, LoadedText, TextSource,
};
use rove_config::Settings;
use rove_search::{
    engine::{ReplacementMatch, ReplacementMatcher},
    LineMatch,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::File,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug)]
pub(crate) enum Failure {
    Changed,
    Skipped,
    Cancelled,
    Invalid,
    Write(WriteError),
}

impl From<WriteError> for Failure {
    fn from(value: WriteError) -> Self {
        Self::Write(value)
    }
}

impl From<Error> for Failure {
    fn from(value: Error) -> Self {
        match value {
            Error::Cancelled => Self::Cancelled,
            _ => Self::Invalid,
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(value: std::io::Error) -> Self {
        let classified = match value.raw_os_error() {
            Some(32 | 33) if cfg!(windows) => WriteError::Busy { attempts: 1 },
            _ if value.kind() == std::io::ErrorKind::PermissionDenied => WriteError::AccessDenied,
            _ => WriteError::Io(value.into()),
        };
        Self::Write(classified)
    }
}

type Result<T> = std::result::Result<T, Failure>;

// 超长正则可以吃掉整个映射文件。只读行内对照只为可展示的匹配复制正文；其余保留普通搜索片段。
const READ_ONLY_INLINE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct PlannedMatch {
    pub line: u64,
    pub columns: std::ops::Range<u32>,
    pub old: Arc<str>,
    pub edit: Replacement,
}

pub(crate) fn contents(source: &dyn TextSource) -> Result<String> {
    let mut text = String::new();
    let range = ByteRange::new(ByteOffset(0), ByteOffset(source.byte_len()))?;
    source.visit_chunks(range, &mut |part| text.push_str(part))?;
    Ok(text)
}

/// 同一搜索结果可能删除过一部分。只取仍然可见的范围，并核对生成结果时看到的行片段。
/// 有任何范围无法重现就拒绝整份文件，避免偏移已经失效时改到另一处。
pub(crate) fn plan(
    matcher: &mut ReplacementMatcher,
    text: &str,
    lines: &[LineMatch],
    pattern: &ReplacePattern,
    preserve_case: bool,
) -> Result<Vec<PlannedMatch>> {
    let (actual, exact) = matcher.scan(text)?;
    plan_matches(
        |range| text.get(range),
        &actual,
        exact,
        lines,
        pattern,
        preserve_case,
        None,
    )
}

fn plan_matches<'a>(
    text_at: impl Fn(std::ops::Range<usize>) -> Option<&'a str>,
    actual: &[LineMatch],
    exact: Vec<ReplacementMatch>,
    lines: &[LineMatch],
    pattern: &ReplacePattern,
    preserve_case: bool,
    inline_limit: Option<usize>,
) -> Result<Vec<PlannedMatch>> {
    let actual: BTreeMap<_, _> = actual.iter().map(|line| (line.line, line)).collect();
    let mut candidates: BTreeMap<_, VecDeque<ReplacementMatch>> = BTreeMap::new();
    for candidate in exact {
        candidates
            .entry((
                candidate.line,
                candidate.columns.start,
                candidate.columns.end,
            ))
            .or_default()
            .push_back(candidate);
    }
    let mut edits = Vec::new();
    for expected in lines {
        let live = actual.get(&expected.line).ok_or(Failure::Changed)?;
        if live.preview_start != expected.preview_start || live.preview != expected.preview {
            return Err(Failure::Changed);
        }
        for columns in &expected.ranges {
            let candidate = candidates
                .get_mut(&(expected.line, columns.start, columns.end))
                .and_then(VecDeque::pop_front)
                .ok_or(Failure::Changed)?;
            if inline_limit.is_some_and(|limit| {
                candidate.bytes.len() > limit
                    || candidate
                        .groups
                        .iter()
                        .flatten()
                        .any(|range| range.len() > limit)
            }) {
                continue;
            }
            let old = text_at(candidate.bytes.clone()).ok_or(Failure::Invalid)?;
            let replacement = if pattern.has_parameters() {
                if candidate.groups.is_empty() {
                    return Err(Failure::Invalid);
                }
                let groups: Vec<_> = candidate
                    .groups
                    .into_iter()
                    .map(|range| range.and_then(&text_at))
                    .collect();
                pattern.build(&groups, preserve_case)
            } else {
                pattern.build(&[Some(old)], preserve_case)
            };
            let range = ByteRange::new(
                ByteOffset(candidate.bytes.start as u64),
                ByteOffset(candidate.bytes.end as u64),
            )?;
            edits.push(PlannedMatch {
                line: expected.line,
                columns: columns.clone(),
                old: Arc::from(old),
                edit: Replacement::new(range, replacement),
            });
        }
    }
    Ok(edits)
}

pub(crate) fn transaction(changes: impl IntoIterator<Item = PlannedMatch>) -> Result<Transaction> {
    Ok(Transaction::new(
        changes
            .into_iter()
            .filter(|change| change.old.as_ref() != change.edit.text.as_ref())
            .map(|change| change.edit)
            .collect(),
    )?)
}

fn load_for_replacement(bytes: &[u8], settings: &dyn Settings) -> Result<LoadedText> {
    let mut options = LoadOptions::new("", settings);
    // 这里的缓冲只用来计算事务。真实只读权限由工作区规则及写入器另外检查。
    options.read_only = Arc::from([]);
    Ok(open_bytes(bytes, &options)?)
}

pub(crate) struct DiskSnapshot {
    pub bytes: Vec<u8>,
    pub stamp: DiskStamp,
}

impl DiskSnapshot {
    pub fn read(path: &Path, size_limit: u64) -> Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if metadata.len() >= size_limit {
            return Err(Failure::Skipped);
        }
        let mut bytes = Vec::new();
        file.take(size_limit).read_to_end(&mut bytes)?;
        if bytes.len() as u64 >= size_limit {
            return Err(Failure::Skipped);
        }
        let stamp = DiskStamp::from_metadata(&metadata, content_hash(&bytes));
        Ok(Self { bytes, stamp })
    }

    pub fn matches(&self, expected: &DiskStamp) -> bool {
        self.stamp.len == expected.len && self.stamp.hash == expected.hash
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceFormat {
    pub encoding: Encoding,
    pub bom: bool,
}

impl SourceFormat {
    fn of(loaded: &LoadedText) -> Self {
        Self {
            encoding: loaded.metadata.encoding.clone(),
            bom: loaded.metadata.bom,
        }
    }

    fn encode(&self, source: &dyn TextSource) -> Result<Vec<u8>> {
        encode(source, &self.encoding, self.bom, Unencodable::Reject).map_err(|_| Failure::Invalid)
    }
}

pub(crate) struct OpenDiskBaseline {
    pub text: Arc<dyn TextSource>,
    pub stamp: DiskStamp,
    pub format: SourceFormat,
    pub dirty: bool,
}

/// 打开文件可能先保存再关闭。关闭后没有编辑历史，由批次记录恢复原始字节。
pub(crate) struct OpenDiskUndo {
    pub path: PathBuf,
    canonical: PathBuf,
    baseline: DiskStamp,
    original: File,
    format: SourceFormat,
}

impl OpenDiskUndo {
    pub fn capture(
        root: &Path,
        path: PathBuf,
        before: OpenDiskBaseline,
        size_limit: u64,
        cancel: &Cancellation,
    ) -> Result<Self> {
        cancel.check()?;
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(root.canonicalize()?) || path.symlink_metadata()?.is_symlink() {
            return Err(Failure::Changed);
        }
        let disk = DiskSnapshot::read(&path, size_limit)?;
        if !disk.matches(&before.stamp) {
            return Err(Failure::Changed);
        }
        let bytes = if before.dirty {
            before.format.encode(before.text.as_ref())?
        } else {
            disk.bytes
        };
        let mut original = tempfile::tempfile()?;
        original.write_all(&bytes)?;
        Ok(Self {
            path,
            canonical,
            baseline: before.stamp,
            original,
            format: before.format,
        })
    }

    pub fn restore(
        &mut self,
        after: &dyn TextSource,
        write: &WriteOptions,
        cancel: &Cancellation,
    ) -> Result<()> {
        cancel.check()?;
        if self.path.canonicalize()? != self.canonical || self.path.symlink_metadata()?.is_symlink()
        {
            return Err(Failure::Changed);
        }
        let saved = self.format.encode(after)?;
        let expected = DiskStamp::from_bytes(&self.path, &saved);
        let size_limit = self.baseline.len.max(expected.len).saturating_add(1);
        let current = DiskSnapshot::read(&self.path, size_limit)?;
        if current.matches(&self.baseline) {
            return Ok(());
        }
        if current.bytes != saved {
            return Err(Failure::Changed);
        }
        let mut bytes = Vec::new();
        self.original.rewind()?;
        self.original.read_to_end(&mut bytes)?;
        guarded_write(
            &self.path,
            &self.canonical,
            &bytes,
            &expected,
            write,
            cancel,
        )?;
        Ok(())
    }
}

/// 写盘前保留逐字节的原始内容。每次只读一份文件，批量撤销不在内存中累计各文件正文。
pub(crate) struct DiskUndo {
    pub path: PathBuf,
    pub expected: DiskStamp,
    original: File,
    canonical: PathBuf,
}

impl DiskUndo {
    pub fn apply(
        source: DiskSource,
        edits: &Transaction,
        partial: Option<(&rove_search::SearchQuery, &[PlannedMatch])>,
        settings: Arc<dyn Settings>,
        write: &WriteOptions,
        cancel: &Cancellation,
    ) -> Result<(Self, Option<Vec<LineMatch>>)> {
        let DiskSource {
            path,
            canonical,
            stamp: expected,
            format,
        } = source;
        cancel.check()?;
        if path.canonicalize()? != canonical || path.symlink_metadata()?.is_symlink() {
            return Err(Failure::Changed);
        }
        let snap = DiskSnapshot::read(&path, expected.len.saturating_add(1))?;
        if !snap.matches(&expected) {
            return Err(Failure::Changed);
        }
        let loaded = load_for_replacement(&snap.bytes, settings.as_ref())?;
        if SourceFormat::of(&loaded) != format {
            return Err(Failure::Changed);
        }
        let mut buffer = EditBuffer::from_loaded(&loaded)?;
        // 有些有损解码的文件不能逐字节重编码。保持其他字节原样是跨文件替换的前置条件。
        if format.encode(buffer.text())? != snap.bytes {
            return Err(Failure::Invalid);
        }
        let applied = buffer.apply(edits)?;
        let remaining = partial
            .map(|(query, remaining)| {
                let config = rove_search::SearchConfig::from_settings(settings);
                let mut matcher = ReplacementMatcher::new(query, &config, cancel.clone())?;
                remaining_lines(&mut matcher, buffer.text(), &applied, remaining)
            })
            .transpose()?;
        let bytes = format.encode(buffer.text())?;
        let mut original = tempfile::tempfile()?;
        original.write_all(&snap.bytes)?;
        let result = guarded_write(&path, &canonical, &bytes, &expected, write, cancel)?;
        Ok((
            Self {
                path,
                expected: result,
                original,
                canonical,
            },
            remaining,
        ))
    }

    pub fn restore(&mut self, write: &WriteOptions, cancel: &Cancellation) -> Result<()> {
        let mut bytes = Vec::new();
        self.original.rewind()?;
        self.original.read_to_end(&mut bytes)?;
        guarded_write(
            &self.path,
            &self.canonical,
            &bytes,
            &self.expected,
            write,
            cancel,
        )?;
        Ok(())
    }
}

/// 只更新原结果树中还保留的匹配；替换引入的新匹配和用户之前剔除的项不会出现。
pub(crate) fn remaining_lines(
    matcher: &mut ReplacementMatcher,
    text: &dyn TextSource,
    applied: &AppliedEdit,
    remaining: &[PlannedMatch],
) -> Result<Vec<LineMatch>> {
    let wanted: BTreeSet<_> = remaining
        .iter()
        .map(|change| {
            let range = &change.edit.range;
            let start = applied.map_offset(range.start(), Bias::After);
            let end_bias = if range.start() == range.end() {
                Bias::After
            } else {
                Bias::Before
            };
            let end = applied.map_offset(range.end(), end_bias);
            (start.0, end.0)
        })
        .collect();
    let (mut lines, exact) = matcher.scan(&contents(text)?)?;
    let mut displayed = BTreeMap::new();
    for found in exact {
        if wanted.contains(&(found.bytes.start as u64, found.bytes.end as u64)) {
            *displayed
                .entry((found.line, found.columns.start, found.columns.end))
                .or_insert(0usize) += 1;
        }
    }
    for line in &mut lines {
        line.ranges.retain(|range| {
            let key = (line.line, range.start, range.end);
            let Some(count) = displayed.get_mut(&key).filter(|count| **count > 0) else {
                return false;
            };
            *count -= 1;
            true
        });
    }
    lines.retain(|line| !line.ranges.is_empty());
    Ok(lines)
}

fn guarded_write(
    path: &Path,
    canonical: &Path,
    bytes: &[u8],
    expected: &DiskStamp,
    options: &WriteOptions,
    cancel: &Cancellation,
) -> Result<DiskStamp> {
    let written = write_atomic_with(path, bytes, options, &|| {
        cancel.check()?;
        if path.symlink_metadata()?.is_symlink() || path.canonicalize()? != canonical {
            return Err(Failure::Changed);
        }
        let current = DiskSnapshot::read(path, expected.len.saturating_add(1))?;
        if !current.matches(expected) {
            return Err(Failure::Changed);
        }
        Ok::<(), Failure>(())
    })?;
    Ok(written.stamp)
}

#[derive(Clone)]
pub(crate) struct PreviewRequest {
    pub serial: u64,
    pub root: PathBuf,
    pub query: rove_search::SearchQuery,
    pub template: String,
    pub preserve_case: bool,
    pub files: Vec<Arc<super::model::FileNode>>,
    pub cancel: Cancellation,
}

#[derive(Clone)]
pub(crate) struct OpenFile {
    pub id: rove_base::DocumentId,
    pub text: Arc<dyn TextSource>,
    pub read_only: bool,
    pub dirty: bool,
    pub eol: rove_buffer::Eol,
    pub recovery: Option<OpenRecovery>,
}

#[derive(Clone)]
pub(crate) struct OpenRecovery {
    pub disk: DiskStamp,
    pub steps: Vec<Transaction>,
}

#[derive(Clone)]
pub(crate) enum Origin {
    Open(rove_base::DocumentId, rove_base::Version),
    Disk(DiskSource),
}

#[derive(Clone)]
pub(crate) struct DiskSource {
    pub path: PathBuf,
    canonical: PathBuf,
    pub stamp: DiskStamp,
    format: SourceFormat,
}

#[derive(Clone)]
pub(crate) struct FilePlan {
    pub path: rove_base::WorkspacePath,
    pub origin: Origin,
    pub changes: Vec<PlannedMatch>,
    pub remaining: Vec<PlannedMatch>,
    pub read_only: bool,
}

pub(crate) enum PreviewFile {
    Planned(FilePlan),
    Skipped(Vec<ReadOnlyComparison>),
    Changed,
}

pub(crate) struct ReadOnlyComparison {
    key: (u64, u32, u32),
    old: String,
    new: String,
}

impl PreviewFile {
    pub(crate) fn comparison(
        &self,
        line: u64,
        columns: &std::ops::Range<u32>,
    ) -> Option<(&str, &str)> {
        let key = (line, columns.start, columns.end);
        match self {
            Self::Planned(file) => {
                let index = file
                    .changes
                    .binary_search_by_key(&key, |m| (m.line, m.columns.start, m.columns.end))
                    .ok()?;
                let change = &file.changes[index];
                Some((&change.old, &change.edit.text))
            }
            Self::Skipped(rows) => {
                let index = rows.binary_search_by_key(&key, |row| row.key).ok()?;
                Some((&rows[index].old, &rows[index].new))
            }
            Self::Changed => None,
        }
    }
}

/// 常见字面量可直接复用已展示的行片段，不重新读取只读文件。
fn lightweight_comparisons(
    lines: &[LineMatch],
    query: &rove_search::SearchQuery,
    pattern: &ReplacePattern,
    preserve_case: bool,
) -> Vec<ReadOnlyComparison> {
    if query.is_multiline() || pattern.has_parameters() {
        return Vec::new();
    }
    let mut comparisons = Vec::new();
    for line in lines {
        for range in &line.ranges {
            let start = range.start.checked_sub(line.preview_start);
            let end = range.end.checked_sub(line.preview_start);
            let Some(old) = start
                .zip(end)
                .and_then(|(start, end)| line.preview.get(start as usize..end as usize))
            else {
                continue;
            };
            comparisons.push(ReadOnlyComparison {
                key: (line.line, range.start, range.end),
                old: old.to_owned(),
                new: pattern.build(&[Some(old)], preserve_case),
            });
        }
    }
    comparisons
}

fn skipped_comparisons(
    matcher: &mut ReplacementMatcher,
    root: &Path,
    path: &Path,
    lines: &[LineMatch],
    request: &PreviewRequest,
    pattern: &ReplacePattern,
) -> Result<Vec<ReadOnlyComparison>> {
    let lightweight =
        lightweight_comparisons(lines, &request.query, pattern, request.preserve_case);
    if lightweight.len() == lines.iter().map(|line| line.ranges.len()).sum::<usize>() {
        return Ok(lightweight);
    }
    let Some(canonical) = path
        .canonicalize()
        .ok()
        .filter(|path| path.starts_with(root))
    else {
        return Ok(lightweight);
    };
    if path
        .symlink_metadata()
        .map_or(true, |meta| meta.is_symlink())
    {
        return Ok(lightweight);
    }
    // 捕获组、跨行或截短的行片段需要完整命中。只读映射在回调外释放，只保留实际结果的对照文案。
    let changes = matcher
        .scan_file(&canonical, |text, actual, exact| {
            plan_matches(
                |range| {
                    text.get(range)
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                },
                actual,
                exact,
                lines,
                pattern,
                request.preserve_case,
                Some(READ_ONLY_INLINE_BYTES),
            )
        })
        .map_err(Failure::from)
        .and_then(Option::transpose);
    match changes {
        Ok(Some(changes)) => Ok(changes
            .into_iter()
            .map(|change| ReadOnlyComparison {
                key: (change.line, change.columns.start, change.columns.end),
                old: change.old.to_string(),
                new: change.edit.text.to_string(),
            })
            .collect()),
        Err(Failure::Cancelled) => Err(Failure::Cancelled),
        _ => Ok(lightweight),
    }
}

pub(crate) type Preview = BTreeMap<rove_base::WorkspacePath, PreviewFile>;

#[derive(Clone)]
pub(crate) struct ApplyRequest {
    pub serial: u64,
    pub plans: Vec<FilePlan>,
    pub query: rove_search::SearchQuery,
    pub skipped: Vec<rove_base::WorkspacePath>,
    pub stale: Vec<rove_base::WorkspacePath>,
    pub cancel: Cancellation,
}

enum PreviewText {
    Flattened(String),
    SnapshotUtf8 {
        storage: String,
        content_start: usize,
    },
}

impl PreviewText {
    fn from_snapshot(bytes: Vec<u8>, loaded: &LoadedText, allow_reuse: bool) -> Result<Self> {
        const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
        let content_start = if loaded.metadata.bom {
            UTF8_BOM.len()
        } else {
            0
        };
        // Loading already enforced the file policy. Strict UTF-8 rules out lossy decoding.
        let storage = if allow_reuse
            && loaded.metadata.encoding == Encoding::Utf8
            && (content_start == 0 || bytes.starts_with(UTF8_BOM))
        {
            String::from_utf8(bytes).ok()
        } else {
            None
        };
        match storage {
            // The String uses the snapshot's allocation. Skip the BOM consumed by the loader;
            // any subsequent U+FEFF is editable text.
            Some(storage) => Ok(Self::SnapshotUtf8 {
                storage,
                content_start,
            }),
            None => contents(loaded.text.as_ref()).map(Self::Flattened),
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Self::Flattened(text) => text,
            Self::SnapshotUtf8 {
                storage,
                content_start,
            } => &storage[*content_start..],
        }
    }
}

pub(crate) fn prepare_preview(
    request: &PreviewRequest,
    opened: &BTreeMap<rove_base::WorkspacePath, OpenFile>,
    settings: Arc<dyn Settings>,
) -> Result<Preview> {
    let config = rove_search::SearchConfig::from_settings(settings.clone());
    let rules = rove_workspace::WorkspaceConfig::from_settings(settings.as_ref())?;
    let huge_size = rove_buffer::SizePolicy::read(settings.as_ref())?.huge_bytes;
    let root = request.root.canonicalize()?;
    let mut matcher = ReplacementMatcher::new(&request.query, &config, request.cancel.clone())?;
    let pattern = ReplacePattern::parse(&request.template, request.query.is_regex);
    let mut previews = BTreeMap::new();
    for file in &request.files {
        request.cancel.check()?;
        let path = &file.path;
        let absolute = request.root.join(path.as_str());
        let metadata = std::fs::metadata(&absolute);
        let permission = metadata
            .as_ref()
            .is_ok_and(|meta| meta.permissions().readonly());
        let rule_read_only = !rules
            .readonly
            .evaluate(path.as_str(), permission)
            .is_empty();
        let loaded = if let Some(open) = opened.get(path) {
            if open.text.byte_len() >= huge_size {
                Err(Failure::Skipped)
            } else {
                contents(open.text.as_ref()).map(|text| {
                    (
                        PreviewText::Flattened(text),
                        Origin::Open(open.id, open.text.version()),
                        open.read_only,
                        open.eol,
                    )
                })
            }
        } else {
            let target = absolute.canonicalize().map_err(Failure::from);
            target.and_then(|target| {
                if !target.starts_with(&root) || absolute.symlink_metadata()?.is_symlink() {
                    return Err(Failure::Skipped);
                }
                let snapshot = DiskSnapshot::read(&absolute, huge_size)?;
                let loaded = load_for_replacement(&snapshot.bytes, settings.as_ref())?;
                let eol = EditBuffer::from_loaded(&loaded)?.eol();
                let text = PreviewText::from_snapshot(
                    snapshot.bytes,
                    &loaded,
                    !rule_read_only && !permission,
                )?;
                Ok((
                    text,
                    Origin::Disk(DiskSource {
                        path: absolute.clone(),
                        canonical: target,
                        stamp: snapshot.stamp,
                        format: SourceFormat::of(&loaded),
                    }),
                    false,
                    eol,
                ))
            })
        };
        let result = match loaded.and_then(|(text, origin, open_read_only, eol)| {
            let text = text.as_str();
            let open = opened.get(path);
            let mut changes = if let Some(open) =
                open.filter(|open| open.dirty && file.open_result != Some(open.id))
            {
                let recovery = open.recovery.as_ref().ok_or(Failure::Changed)?;
                let original = DiskSnapshot::read(&request.root.join(path.as_str()), huge_size)?;
                if !original.matches(&recovery.disk) {
                    return Err(Failure::Changed);
                }
                let loaded = load_for_replacement(&original.bytes, settings.as_ref())?;
                let baseline = contents(loaded.text.as_ref())?;
                let changes = plan(
                    &mut matcher,
                    &baseline,
                    &file.lines,
                    &pattern,
                    request.preserve_case,
                )?;
                map_open_changes(
                    &mut matcher,
                    changes,
                    EditBuffer::from_loaded(&loaded)?,
                    recovery,
                    text,
                )?
            } else {
                plan(
                    &mut matcher,
                    text,
                    &file.lines,
                    &pattern,
                    request.preserve_case,
                )?
            };
            for change in &mut changes {
                let normalized =
                    rove_buffer::edit::transaction::normalize_newlines(&change.edit.text, eol);
                if matches!(normalized, std::borrow::Cow::Owned(_)) {
                    change.edit.text = Arc::from(normalized.as_ref());
                }
            }
            Ok(FilePlan {
                path: path.clone(),
                origin,
                changes,
                remaining: Vec::new(),
                read_only: rule_read_only || open_read_only,
            })
        }) {
            Ok(plan) => PreviewFile::Planned(plan),
            Err(Failure::Cancelled) => return Err(Failure::Cancelled),
            Err(Failure::Skipped) => PreviewFile::Skipped(skipped_comparisons(
                &mut matcher,
                &root,
                &absolute,
                &file.lines,
                request,
                &pattern,
            )?),
            Err(_) => PreviewFile::Changed,
        };
        previews.insert(path.clone(), result);
    }
    Ok(previews)
}

/// 本地未保存的编辑可能移动搜索结果。仅当磁盘快照、编辑历史和当前匹配都能对上时平移范围。
fn map_open_changes(
    matcher: &mut ReplacementMatcher,
    mut changes: Vec<PlannedMatch>,
    mut buffer: EditBuffer,
    recovery: &OpenRecovery,
    current: &str,
) -> Result<Vec<PlannedMatch>> {
    for step in &recovery.steps {
        let applied = buffer.apply(step)?;
        for change in &mut changes {
            let range = &change.edit.range;
            let start = range.start();
            let end = range.end();
            if applied
                .forward()
                .edits()
                .iter()
                .any(|edit| edit.range.start() < end && edit.range.end() > start)
            {
                return Err(Failure::Changed);
            }
            let start = applied.map_offset(start, Bias::After);
            let end = applied.map_offset(end, Bias::Before);
            change.edit.range = ByteRange::new(start, end).map_err(|_| Failure::Changed)?;
        }
    }
    if buffer.contents() != current {
        return Err(Failure::Changed);
    }
    let (_, live) = matcher.scan(current)?;
    let confirmed: std::collections::BTreeSet<_> = live
        .iter()
        .map(|m| (m.bytes.start as u64, m.bytes.end as u64))
        .collect();
    for change in &changes {
        let range = &change.edit.range;
        if !confirmed.contains(&(range.start().0, range.end().0)) {
            return Err(Failure::Changed);
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_search::{SearchConfig, SearchQuery};

    #[test]
    fn exact_multiline_result_reuses_capture_group_and_replace_semantics() {
        let settings = Arc::new(rove_search::settings::default_settings());
        let config = SearchConfig::from_settings(settings);
        let mut query = SearchQuery::new(r"(old)\r?\n(value)");
        query.is_regex = true;
        let mut matcher =
            ReplacementMatcher::new(&query, &config, Cancellation::default()).unwrap();
        let text = "keep\r\nold\r\nvalue\nkeep again";
        let (lines, exact) = matcher.scan(text).unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(&text[exact[0].bytes.clone()], "old\r\nvalue");
        assert_eq!(exact[0].line, 1);
        let changes = plan(
            &mut matcher,
            text,
            &lines,
            &ReplacePattern::parse("$2:$1", true),
            false,
        )
        .unwrap();
        let changed = &changes[0];
        assert_eq!(changed.old.as_ref(), "old\r\nvalue");
        assert_eq!(changed.edit.text.as_ref(), "value:old");
    }

    #[test]
    fn discarded_match_and_changed_preview_cannot_be_replaced() {
        let settings = Arc::new(rove_search::settings::default_settings());
        let config = SearchConfig::from_settings(settings);
        let query = SearchQuery::new("match");
        let mut matcher =
            ReplacementMatcher::new(&query, &config, Cancellation::default()).unwrap();
        let text = "match and MaTcH";
        let (mut lines, _) = matcher.scan(text).unwrap();
        lines[0].ranges.remove(0);
        let pattern = ReplacePattern::parse("changed", false);
        let changes = plan(&mut matcher, text, &lines, &pattern, true).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].old.as_ref(), "MaTcH");
        assert_eq!(
            changes[0].edit.text.as_ref(),
            crate::find::replace::preserve_case_text("MaTcH", "changed")
        );
        assert!(matches!(
            plan(&mut matcher, "match and match", &lines, &pattern, true),
            Err(Failure::Changed)
        ));
    }

    #[test]
    fn exact_edits_match_editor_capture_and_case_rules_on_same_source() {
        use crate::find::engine::{search_all, FindOptions, Query, Searcher};
        use crate::find::replace::plan_replacements;

        let config =
            SearchConfig::from_settings(Arc::new(rove_search::settings::default_settings()));
        let cancel = Cancellation::default();
        let text = "Old words old WORDS\nold words OLD WORDS";
        let rope = rove_buffer::RopeText::from_text(text);
        for (search, replace, preserve_case) in [
            ("(old) (words)", "$2$1$$-$&-$0", false),
            ("old words", "new words", true),
            ("$", "!", false),
        ] {
            let mut query = SearchQuery::new(search);
            query.is_regex = true;
            let mut matched = ReplacementMatcher::new(&query, &config, cancel.clone()).unwrap();
            let (lines, _) = matched.scan(text).unwrap();
            let pattern = ReplacePattern::parse(replace, true);
            let changes = plan(&mut matched, text, &lines, &pattern, preserve_case).unwrap();
            let searcher = Searcher::compile(&Query {
                text: search.to_owned(),
                options: FindOptions {
                    regex: true,
                    ..Default::default()
                },
                word_separators: Arc::from(""),
            })
            .unwrap()
            .unwrap();
            let found =
                search_all(&rope, &searcher, None, usize::MAX, &cancel, &mut |_, _| {}).unwrap();
            let in_file =
                plan_replacements(&rope, &searcher, &found.matches, &pattern, preserve_case)
                    .unwrap();
            assert_eq!(transaction(changes).unwrap().edits(), in_file);
        }
    }

    #[test]
    fn closed_snapshot_reuses_allocation_and_honors_disk_bom_and_guards() {
        let settings = rove_search::settings::default_settings();
        let raw = b"\xef\xbb\xbf\xef\xbb\xbfplain".to_vec();
        let loaded = load_for_replacement(&raw, &settings).unwrap();
        let expected = contents(loaded.text.as_ref()).unwrap();
        let protected = PreviewText::from_snapshot(raw.clone(), &loaded, false).unwrap();
        assert!(matches!(&protected, PreviewText::Flattened(_)));
        assert_eq!(protected.as_str(), expected);
        let original_body = raw.as_ptr().wrapping_add(3);
        let text = PreviewText::from_snapshot(raw, &loaded, true).unwrap();
        assert!(matches!(&text, PreviewText::SnapshotUtf8 { .. }));
        assert_eq!(text.as_str().as_ptr(), original_body);
        assert_eq!(text.as_str(), "\u{feff}plain");
        assert_eq!(text.as_str(), expected);
        assert!(load_for_replacement(b"binary\0content", &settings).is_err());
    }

    #[test]
    fn closed_utf8_preview_matches_flattened_plans_and_exact_disk_bytes() {
        use rove_base::WorkspacePath;
        use rove_config::{SettingsOptions, SettingsService};

        let directory = tempfile::tempdir().unwrap();
        let plain = "é old\r\nvalue\ntail\r\n";
        let unicode = "😀é old\r\nvalue\ntail\r\n";
        let configure = |name: &str, json: &str| {
            let user = directory.path().join(name);
            std::fs::create_dir(&user).unwrap();
            std::fs::write(user.join("settings.json"), json).unwrap();
            let backup = user.join("backup");
            let mut options = SettingsOptions::new(user, backup);
            options.watch = None;
            Arc::new(SettingsService::open(
                options,
                rove_search::settings::default_settings().0,
            ))
        };
        let utf8 = configure(
            "utf8",
            r#"{"files.encoding":"utf8","files.autoGuessEncoding":false}"#,
        );
        let legacy = configure(
            "legacy",
            r#"{"files.encoding":"windows1252","files.autoGuessEncoding":false}"#,
        );
        let with_prefix = |prefix: &[u8], bytes: &[u8]| [prefix, bytes].concat();
        let utf16 = |text: &str, le: bool| {
            let bom = if le { [0xff, 0xfe] } else { [0xfe, 0xff] };
            bom.into_iter()
                .chain(text.encode_utf16().flat_map(|unit| {
                    if le {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    }
                }))
                .collect::<Vec<_>>()
        };
        let unicode_changed = "😀é value:old\ntail\r\n";
        let expected_cp1252 = b"\xe9 value:old\ntail\r\n".to_vec();
        let mut lossy = with_prefix(b"\xef\xbb\xbf", unicode.as_bytes());
        lossy.extend(std::iter::repeat_n(b'x', 160));
        lossy.push(0xff);
        let cases = [
            (
                "plain UTF-8",
                unicode.as_bytes().to_vec(),
                Some(unicode_changed.as_bytes().to_vec()),
                &utf8,
                Encoding::Utf8,
                true,
            ),
            (
                "BOM UTF-8",
                with_prefix(b"\xef\xbb\xbf", unicode.as_bytes()),
                Some(with_prefix(b"\xef\xbb\xbf", unicode_changed.as_bytes())),
                &utf8,
                Encoding::Utf8,
                true,
            ),
            (
                "double BOM UTF-8",
                with_prefix(b"\xef\xbb\xbf\xef\xbb\xbf", unicode.as_bytes()),
                Some(with_prefix(
                    b"\xef\xbb\xbf\xef\xbb\xbf",
                    unicode_changed.as_bytes(),
                )),
                &utf8,
                Encoding::Utf8,
                true,
            ),
            (
                "UTF-16LE",
                utf16(unicode, true),
                Some(utf16(unicode_changed, true)),
                &legacy,
                Encoding::Utf16Le,
                false,
            ),
            (
                "UTF-16BE",
                utf16(unicode, false),
                Some(utf16(unicode_changed, false)),
                &legacy,
                Encoding::Utf16Be,
                false,
            ),
            (
                "configured Windows-1252",
                b"\xe9 old\r\nvalue\ntail\r\n".to_vec(),
                Some(expected_cp1252),
                &legacy,
                Encoding::Other(Arc::from("windows-1252")),
                false,
            ),
            (
                "strict UTF-8 bytes selected as Windows-1252",
                b"\xc3\xa9 old\r\nvalue\ntail\r\n".to_vec(),
                Some(b"\xc3\xa9 value:old\ntail\r\n".to_vec()),
                &legacy,
                Encoding::Other(Arc::from("windows-1252")),
                false,
            ),
            (
                "accepted lossy UTF-8",
                lossy,
                None,
                &utf8,
                Encoding::Utf8,
                false,
            ),
        ];
        for (label, original, expected, settings, encoding, borrow_expected) in cases {
            let relative = WorkspacePath::new("fixture.txt").unwrap();
            let path = directory.path().join(relative.as_str());
            std::fs::write(&path, &original).unwrap();
            let loaded = load_for_replacement(&original, settings.as_ref()).unwrap();
            assert_eq!(loaded.metadata.encoding, encoding, "{label}");
            let text = contents(loaded.text.as_ref()).unwrap();
            assert!(text.contains("old\r\nvalue"), "{label}: {text:?}");
            if label == "configured Windows-1252" {
                assert_eq!(text, plain);
            }
            let config = SearchConfig::from_settings(settings.clone());
            let mut query = SearchQuery::new(r"(old)\r?\n(value)");
            query.is_regex = true;
            let cancel = Cancellation::default();
            let mut matcher = ReplacementMatcher::new(&query, &config, cancel.clone()).unwrap();
            let (lines, exact) = matcher.scan(&text).unwrap();
            assert_eq!(exact.len(), 1, "{label}");
            let snapshot_text =
                PreviewText::from_snapshot(original.clone(), &loaded, true).unwrap();
            assert_eq!(
                matches!(&snapshot_text, PreviewText::SnapshotUtf8 { .. }),
                borrow_expected,
                "{label}"
            );
            assert_eq!(snapshot_text.as_str(), text, "{label}");
            let template = "$2:$1";
            let baseline = plan(
                &mut matcher,
                &text,
                &lines,
                &ReplacePattern::parse(template, true),
                false,
            )
            .unwrap();
            let request = PreviewRequest {
                serial: 1,
                root: directory.path().to_path_buf(),
                query,
                template: template.to_owned(),
                preserve_case: false,
                files: vec![Arc::new(super::super::model::FileNode::new(
                    relative.clone(),
                    lines,
                ))],
                cancel: cancel.clone(),
            };
            let mut preview =
                prepare_preview(&request, &BTreeMap::new(), settings.clone()).unwrap();
            assert_eq!(preview.len(), 1, "{label}");
            let Some(PreviewFile::Planned(file)) = preview.remove(&relative) else {
                panic!("{label}: preview did not produce an applicable file");
            };
            assert!(!file.read_only && file.remaining.is_empty(), "{label}");
            let Origin::Disk(origin) = file.origin else {
                panic!("{label}: expected disk origin");
            };
            assert_eq!(origin.format, SourceFormat::of(&loaded), "{label}");
            assert_eq!(baseline.len(), 1, "{label}");
            assert_eq!(file.changes.len(), baseline.len(), "{label}");
            let change = &file.changes[0];
            let old_path = &baseline[0];
            assert_eq!(change.line, old_path.line, "{label}");
            assert_eq!(change.columns, old_path.columns, "{label}");
            assert_eq!(change.old, old_path.old, "{label}");
            assert_eq!(change.edit, old_path.edit, "{label}");
            assert_eq!(change.old.as_ref(), "old\r\nvalue");
            assert_eq!(change.edit.text.as_ref(), "value:old");
            assert_eq!(change.edit.range.start().0, exact[0].bytes.start as u64);
            assert_eq!(change.edit.range.end().0, exact[0].bytes.end as u64);
            assert_eq!(change.line, exact[0].line);
            assert_eq!(change.columns, exact[0].columns);
            assert_eq!(change.columns.start as usize, text.find("old").unwrap());

            let edits = transaction(file.changes).unwrap();
            let write = WriteOptions::default();
            let result = DiskUndo::apply(origin, &edits, None, settings.clone(), &write, &cancel);
            if let Some(expected) = &expected {
                let (mut undo, _) = result.unwrap();
                assert_eq!(
                    std::fs::read(&path).unwrap(),
                    *expected,
                    "{label}: write bytes"
                );
                undo.restore(&write, &cancel).unwrap();
            } else {
                assert!(
                    matches!(result, Err(Failure::Invalid)),
                    "{label}: lossy write must fail"
                );
            }
            assert_eq!(
                std::fs::read(&path).unwrap(),
                original,
                "{label}: original bytes"
            );
        }
    }

    #[test]
    fn restore_refuses_to_overwrite_an_external_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.txt");
        let original = b"\xef\xbb\xbfMATCH\r\nkeep\n";
        std::fs::write(&path, original).unwrap();
        let settings = rove_search::settings::default_settings();
        let original = DiskSnapshot::read(&path, 128).unwrap();
        let edit = Transaction::single(
            ByteRange::new(ByteOffset(0), ByteOffset(5)).unwrap(),
            "same",
        );
        let cancel = Cancellation::default();
        let write = WriteOptions::default();
        let (mut undo, _) = DiskUndo::apply(
            DiskSource {
                path: path.clone(),
                canonical: path.canonicalize().unwrap(),
                stamp: original.stamp,
                format: SourceFormat {
                    encoding: Encoding::Utf8,
                    bom: true,
                },
            },
            &edit,
            None,
            Arc::new(settings),
            &write,
            &cancel,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"\xef\xbb\xbfsame\r\nkeep\n");
        std::fs::write(&path, b"external change").unwrap();
        assert!(undo.restore(&write, &cancel).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"external change");
    }
}
