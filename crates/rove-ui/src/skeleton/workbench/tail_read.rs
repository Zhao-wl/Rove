//! Disk work for R66 runs outside the window thread.
use std::{fs::File, path::PathBuf, sync::Arc};

use rove_base::{ByteOffset, Error, Result, Span, Version};
use rove_buffer::{
    open_follow_file,
    save::{
        tail::{
            extend_disk_hash, utf8_append, LineEndingCounts, ReadChange, TailFingerprint,
            Utf8Append,
        },
        DiskStamp,
    },
    Document, LoadOptions, MappedText, OpenedFile, Reload, SizePolicy,
};
use rove_config::Settings;

use super::tail_state::ReloadRequest;

pub(super) struct OrdinaryBaseline {
    stamp: DiskStamp,
    fingerprint: TailFingerprint,
    endings: Option<LineEndingCounts>,
}

#[derive(Clone, Default)]
pub(super) struct PathWatch {
    initialized: bool,
    pub accepted: Option<u64>,
    pub has_baseline: bool,
    #[cfg(windows)]
    handle: Option<Arc<rove_buffer::save::tail::TailSizeWatch>>,
}

impl PathWatch {
    fn initialize(&mut self, path: &std::path::Path) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        #[cfg(windows)]
        {
            let initial_len = path.metadata().map_or(0, |metadata| metadata.len());
            self.handle = rove_buffer::save::tail::TailSizeWatch::start(path, initial_len)
                .ok()
                .map(Arc::new);
        }
        #[cfg(not(windows))]
        let _ = path;
    }

    #[cfg(windows)]
    fn synchronize_baseline(
        &self,
        path: &std::path::Path,
        generation: Option<u64>,
        change: &Change,
    ) -> Result<()> {
        let (Some(watch), Some(generation)) = (&self.handle, generation) else {
            return Ok(());
        };
        let length = match change {
            Change::Disk(stamp, _) => stamp.len,
            Change::Reopened(opened) => match opened.text.mapped() {
                Some(mapped) => mapped.raw_byte_len(),
                None => path.metadata()?.len(),
            },
            _ => return Ok(()),
        };
        // A false result means the watcher raced the read; the caller checks the
        // observed generation again when it is about to publish the snapshot.
        watch.accept_baseline(path, generation, length)?;
        Ok(())
    }

    pub fn generation(&self) -> Option<u64> {
        #[cfg(windows)]
        {
            self.handle
                .as_ref()
                .filter(|watch| watch.is_live())
                .map(|watch| watch.generation())
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

pub(super) struct ReadJob {
    pub document: Arc<Document>,
    pub path: PathBuf,
    pub settings: Arc<dyn Settings>,
    pub policy: SizePolicy,
    pub request: ReloadRequest,
    pub watch: PathWatch,
    pub mapped: Option<Arc<MappedText>>,
    pub ordinary: Option<OrdinaryBaseline>,
    pub saved: Option<DiskStamp>,
    pub clean: bool,
    /// Only a clean UTF-8 editing session can consume a suffix as an external edit.
    pub can_append: bool,
}

pub(super) enum Change {
    Unchanged,
    Missing,
    MappedAppend(Arc<MappedText>),
    Reopened(OpenedFile),
    Disk(DiskStamp, Vec<u8>),
    OrdinaryAppend {
        base: DiskStamp,
        next: DiskStamp,
        suffixed: String,
        endings: LineEndingCounts,
    },
}

pub(super) struct ReadResult {
    pub origin: Version,
    pub storage_ready: bool,
    pub watch: PathWatch,
    pub generation: Option<u64>,
    pub baseline: Option<OrdinaryBaseline>,
    pub change: Result<Change>,
}

pub(super) fn read(mut job: ReadJob) -> ReadResult {
    job.watch.initialize(&job.path);
    let generation = job.watch.generation();
    if !job.watch.has_baseline || job.watch.accepted != generation {
        job.request = ReloadRequest::ReopenPath;
    }
    let storage_ready = job.mapped.as_ref().is_none_or(|mapped| {
        let was_mapped = mapped.is_memory_mapped();
        let ready = mapped.enable_shared_reads();
        if was_mapped && ready {
            mapped.warm_tail();
        }
        ready
    });
    let changes = if storage_ready {
        let result = apply(&mut job);
        #[cfg(windows)]
        let result = result.and_then(|change| {
            job.watch
                .synchronize_baseline(&job.path, generation, &change)?;
            Ok(change)
        });
        result
    } else {
        Ok(Change::Unchanged)
    };
    let change = match changes {
        Err(Error::Io {
            kind: std::io::ErrorKind::NotFound,
            ..
        }) => Ok(Change::Missing),
        other => other,
    };
    ReadResult {
        origin: job.document.text.version(),
        storage_ready,
        watch: job.watch,
        generation,
        baseline: job.ordinary,
        change,
    }
}

fn apply(job: &mut ReadJob) -> Result<Change> {
    let force = job.request == ReloadRequest::ReopenPath;
    if let Some(mapped) = job.mapped.as_ref() {
        if !force {
            match mapped.reload_append(job.watch.generation().is_some())? {
                Some(Reload::Unchanged) => return Ok(Change::Unchanged),
                Some(Reload::Appended(next)) => {
                    next.wait_complete(std::time::Duration::from_millis(50));
                    next.warm_tail();
                    return Ok(Change::MappedAppend(next));
                }
                // Reopen through the regular detector to read the new file's encoding and BOM.
                Some(Reload::Replaced(_)) | None => {}
            }
        }
        return opened(job);
    }

    let mut file = File::open(&job.path)?;
    let metadata = file.metadata()?;
    if metadata.len() >= job.policy.huge_bytes && job.clean {
        return opened(job);
    }
    if force {
        job.ordinary = None;
        let change = ordinary_full(job)?;
        if let Change::Disk(stamp, _) = &change {
            if job.saved.as_ref() == Some(stamp) && stamp.same_metadata(&metadata) {
                job.ordinary = capture_ordinary_baseline(job, &mut file, stamp)?;
            }
        }
        return Ok(change);
    }
    let Some(saved) = job.saved.as_ref() else {
        return Ok(Change::Unchanged);
    };
    if !job.can_append {
        if job.request == ReloadRequest::PollPath
            && job.ordinary.as_ref().is_some_and(|current| {
                current.stamp.same_metadata(&metadata)
                    && current
                        .fingerprint
                        .is_replaced(&file)
                        .is_ok_and(|changed| !changed)
            })
        {
            return Ok(Change::Unchanged);
        }
        if job.ordinary.is_none() && saved.same_metadata(&metadata) {
            job.ordinary = Some(OrdinaryBaseline {
                stamp: saved.clone(),
                fingerprint: TailFingerprint::capture(&mut file)?,
                endings: None,
            });
            if job.request == ReloadRequest::PollPath {
                return Ok(Change::Unchanged);
            }
        }
        let change = ordinary_full(job)?;
        if let Change::Disk(stamp, _) = &change {
            if stamp.same_metadata(&file.metadata()?) {
                job.ordinary = Some(OrdinaryBaseline {
                    stamp: stamp.clone(),
                    fingerprint: TailFingerprint::capture(&mut file)?,
                    endings: None,
                });
            }
        }
        return Ok(change);
    }
    if job
        .ordinary
        .as_ref()
        .is_none_or(|current| current.stamp != *saved || current.endings.is_none())
    {
        if !saved.same_metadata(&metadata) {
            return ordinary_full(job);
        }
        job.ordinary = capture_ordinary_baseline(job, &mut file, saved)?;
        if job.ordinary.is_none() {
            return ordinary_full(job);
        }
    }
    let current = job.ordinary.as_mut().unwrap();
    match current.fingerprint.read_change(
        &mut file,
        false,
        job.policy.huge_bytes.saturating_sub(1),
    )? {
        ReadChange::Unchanged => Ok(Change::Unchanged),
        ReadChange::Replaced => {
            job.ordinary = None;
            ordinary_full(job)
        }
        ReadChange::Appended { bytes, next } => {
            if job.watch.generation().is_none()
                && !current
                    .fingerprint
                    .matches_prefix(&mut file, current.stamp.hash)?
            {
                job.ordinary = None;
                return ordinary_full(job);
            }
            let text = match utf8_append(&bytes) {
                Utf8Append::Complete(text) => text,
                Utf8Append::Incomplete => return Ok(Change::Unchanged),
                Utf8Append::Invalid => {
                    job.ordinary = None;
                    return ordinary_full(job);
                }
            };
            let stamp = next.disk_stamp(extend_disk_hash(current.stamp.hash, &bytes));
            let mut endings = current.endings.unwrap();
            endings.add(text);
            let change = Change::OrdinaryAppend {
                base: current.stamp.clone(),
                next: stamp.clone(),
                suffixed: text.to_owned(),
                endings,
            };
            *current = OrdinaryBaseline {
                stamp,
                fingerprint: next,
                endings: Some(endings),
            };
            Ok(change)
        }
    }
}

fn capture_ordinary_baseline(
    job: &ReadJob,
    file: &mut File,
    saved: &DiskStamp,
) -> Result<Option<OrdinaryBaseline>> {
    let fingerprint = TailFingerprint::capture(file)?;
    if fingerprint.disk_stamp(saved.hash) != *saved {
        return Ok(None);
    }
    let mut endings = LineEndingCounts::default();
    if job.can_append {
        let text = job.document.text.as_ref();
        text.visit_chunks(
            Span::new(ByteOffset(0), ByteOffset(text.byte_len()))?,
            &mut |chunk| endings.add(chunk),
        )?;
    }
    Ok(Some(OrdinaryBaseline {
        stamp: saved.clone(),
        fingerprint,
        endings: job.can_append.then_some(endings),
    }))
}

fn opened(job: &ReadJob) -> Result<Change> {
    let mut options = LoadOptions::new(&job.document.metadata.language, job.settings.as_ref());
    options.read_only = job
        .document
        .metadata
        .read_only
        .iter()
        .filter(|reason| !matches!(reason, rove_buffer::ReadOnlyReason::HugeFile))
        .cloned()
        .collect();
    let opened = open_follow_file(&job.path, &options, job.policy)?;
    if let Some(mapped) = opened.text.mapped() {
        mapped.warm_tail();
    }
    Ok(Change::Reopened(opened))
}

fn ordinary_full(job: &ReadJob) -> Result<Change> {
    let Some((stamp, bytes)) = DiskStamp::read(&job.path)? else {
        return Ok(Change::Missing);
    };
    Ok(Change::Disk(stamp, bytes))
}
