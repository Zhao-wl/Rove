//! Disk work for R66 runs outside the window thread.
use std::{fs::File, path::PathBuf, sync::Arc};

use rove_base::{ByteOffset, Error, Result, Span, Version};
use rove_buffer::{
    open_file,
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
    endings: LineEndingCounts,
}

pub(super) struct ReadJob {
    pub document: Arc<Document>,
    pub path: PathBuf,
    pub settings: Arc<dyn Settings>,
    pub policy: SizePolicy,
    pub request: ReloadRequest,
    pub mapped: Option<Arc<MappedText>>,
    pub ordinary: Option<OrdinaryBaseline>,
    /// Present only for a clean UTF-8 editing session at the captured snapshot version.
    pub saved: Option<DiskStamp>,
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
    pub baseline: Option<OrdinaryBaseline>,
    pub change: Result<Change>,
}

pub(super) fn read(mut job: ReadJob) -> ReadResult {
    let changes = apply(&mut job);
    let change = match changes {
        Err(Error::Io {
            kind: std::io::ErrorKind::NotFound,
            ..
        }) => Ok(Change::Missing),
        other => other,
    };
    ReadResult {
        origin: job.document.text.version(),
        baseline: job.ordinary,
        change,
    }
}

fn apply(job: &mut ReadJob) -> Result<Change> {
    let force = job.request == ReloadRequest::ReopenPath;
    if let Some(mapped) = job.mapped.as_ref() {
        if !force {
            match mapped.reload_append()? {
                Some(Reload::Unchanged) => return Ok(Change::Unchanged),
                Some(Reload::Appended(next)) => return Ok(Change::MappedAppend(next)),
                // Reopen through the regular detector to read the new file's encoding and BOM.
                Some(Reload::Replaced(_)) | None => {}
            }
        }
        return opened(job);
    }

    let mut file = File::open(&job.path)?;
    let metadata = file.metadata()?;
    if metadata.len() >= job.policy.huge_bytes && job.saved.is_some() {
        return opened(job);
    }
    if force || job.saved.is_none() {
        job.ordinary = None;
        return ordinary_full(job);
    }
    let saved = job.saved.as_ref().unwrap();
    if job
        .ordinary
        .as_ref()
        .is_none_or(|current| current.stamp != *saved)
    {
        if !saved.same_metadata(&metadata) {
            return ordinary_full(job);
        }
        let fingerprint = TailFingerprint::capture(&mut file)?;
        let mut endings = LineEndingCounts::default();
        let text = job.document.text.as_ref();
        text.visit_chunks(
            Span::new(ByteOffset(0), ByteOffset(text.byte_len()))?,
            &mut |chunk| {
                endings.add(chunk);
            },
        )?;
        job.ordinary = Some(OrdinaryBaseline {
            stamp: saved.clone(),
            fingerprint,
            endings,
        });
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
            let text = match utf8_append(&bytes) {
                Utf8Append::Complete(text) => text,
                Utf8Append::Incomplete => return Ok(Change::Unchanged),
                Utf8Append::Invalid => {
                    job.ordinary = None;
                    return ordinary_full(job);
                }
            };
            let stamp = next.disk_stamp(extend_disk_hash(current.stamp.hash, &bytes));
            let mut endings = current.endings;
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
                endings,
            };
            Ok(change)
        }
    }
}

fn opened(job: &ReadJob) -> Result<Change> {
    let options = LoadOptions::new(&job.document.metadata.language, job.settings.as_ref());
    open_file(&job.path, &options, job.policy).map(Change::Reopened)
}

fn ordinary_full(job: &ReadJob) -> Result<Change> {
    let Some((stamp, bytes)) = DiskStamp::read(&job.path)? else {
        return Ok(Change::Missing);
    };
    Ok(Change::Disk(stamp, bytes))
}
