//! Incremental observation of a followed ordinary text file.
//! Huge files continue to use the existing MappedText::reload path.
mod decode;
#[cfg(windows)]
mod watch;
#[cfg(windows)]
pub use watch::TailSizeWatch;

pub use decode::{extend_disk_hash, utf8_append, LineEndingCounts, Utf8Append};

use super::DiskStamp;

use std::{
    collections::hash_map::DefaultHasher,
    fs::{File, Metadata},
    hash::Hasher,
    io::{self, Read, Seek, SeekFrom},
    time::SystemTime,
};

const SAMPLE: u64 = 4096;

#[derive(Debug)]
pub struct TailFingerprint {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    identity: Option<(u64, u64)>,
    head: u64,
    tail: u64,
}

pub enum ReadChange {
    Unchanged,
    Appended {
        bytes: Vec<u8>,
        next: TailFingerprint,
    },
    /// The caller loads this path through the normal opening/rotation path.
    Replaced,
}

fn hash_at(file: &mut File, start: u64, size: u64) -> io::Result<u64> {
    let mut sample = [0; SAMPLE as usize];
    let sample = &mut sample[..size as usize];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(sample)?;
    let mut hash = DefaultHasher::new();
    hash.write(sample);
    Ok(hash.finish())
}

#[cfg(windows)]
pub(crate) fn file_identity(file: &File, _: &Metadata) -> Option<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the File owns a live handle and info is valid writable storage.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return None;
    }
    Some((
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}

#[cfg(unix)]
pub(crate) fn file_identity(_: &File, metadata: &Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(any(windows, unix)))]
pub(crate) fn file_identity(_: &File, _: &Metadata) -> Option<(u64, u64)> {
    None
}

impl TailFingerprint {
    pub fn capture(file: &mut File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Self::capture_at(file, &metadata)
    }

    fn capture_at(file: &mut File, metadata: &Metadata) -> io::Result<Self> {
        let len = metadata.len();
        let size = len.min(SAMPLE);
        Ok(Self {
            len,
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            identity: file_identity(file, metadata),
            head: hash_at(file, 0, size)?,
            tail: hash_at(file, len - size, size)?,
        })
    }

    pub fn disk_stamp(&self, hash: u64) -> DiskStamp {
        DiskStamp {
            len: self.len,
            modified: self.modified,
            hash,
        }
    }

    fn object_changed(&self, file: &File, metadata: &Metadata) -> bool {
        self.identity
            .zip(file_identity(file, metadata))
            .is_some_and(|(old, new)| old != new)
            || self
                .created
                .zip(metadata.created().ok())
                .is_some_and(|(old, new)| old != new)
    }

    /// Validate the complete prior prefix when reliable truncation notifications are unavailable.
    pub fn matches_prefix(&self, file: &mut File, expected_hash: u64) -> io::Result<bool> {
        let mut remaining = self.len;
        let mut hash = super::content_hash(&[]);
        let mut chunk = [0u8; 32 * 1024];
        file.seek(SeekFrom::Start(0))?;
        while remaining != 0 {
            let count = remaining.min(chunk.len() as u64) as usize;
            file.read_exact(&mut chunk[..count])?;
            hash = extend_disk_hash(hash, &chunk[..count]);
            remaining -= count as u64;
        }
        Ok(hash == expected_hash)
    }

    pub fn is_replaced(&self, file: &File) -> io::Result<bool> {
        Ok(self.object_changed(file, &file.metadata()?))
    }

    /// The file is opened anew at the same user path, after checking the event sequence.
    pub fn read_change(
        &self,
        file: &mut File,
        must_reopen: bool,
        max_file_bytes: u64,
    ) -> io::Result<ReadChange> {
        let metadata = file.metadata()?;
        let len = metadata.len();
        if must_reopen
            || len < self.len
            || len > max_file_bytes
            || self.object_changed(file, &metadata)
        {
            return Ok(ReadChange::Replaced);
        }
        let size = self.len.min(SAMPLE);
        if hash_at(file, 0, size)? != self.head
            || hash_at(file, self.len - size, size)? != self.tail
        {
            return Ok(ReadChange::Replaced);
        }
        if len == self.len {
            return Ok(if self.modified == metadata.modified().ok() {
                ReadChange::Unchanged
            } else {
                ReadChange::Replaced
            });
        }
        let size = usize::try_from(len - self.len)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut bytes = vec![0u8; size];
        file.seek(SeekFrom::Start(self.len))?;
        file.read_exact(&mut bytes)?;
        Ok(ReadChange::Appended {
            bytes,
            next: Self::capture_at(file, &metadata)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        fs::OpenOptions,
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(data: &[u8]) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rove-r66-append-{}-{}.log",
                std::process::id(),
                NEXT_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(data).unwrap();
            Self(path)
        }

        fn read(&self) -> File {
            File::open(&self.0).unwrap()
        }

        fn append(&self, data: &[u8]) {
            OpenOptions::new()
                .append(true)
                .open(&self.0)
                .unwrap()
                .write_all(data)
                .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn fallback_prefix_validation_rejects_an_in_place_rewrite_with_matching_samples() {
        let mut original = vec![b'a'; 24_576];
        let fixture = Fixture::new(&original);
        let known_hash = super::super::content_hash(&original);
        let state = TailFingerprint::capture(&mut fixture.read()).unwrap();
        original[8_192] = b'b';
        original.extend_from_slice(b"late row\n");
        let mut writer = OpenOptions::new().write(true).open(&fixture.0).unwrap();
        writer.set_len(0).unwrap();
        writer.write_all(&original).unwrap();
        drop(writer);
        assert!(matches!(
            state
                .read_change(&mut fixture.read(), false, u64::MAX)
                .unwrap(),
            ReadChange::Appended { .. }
        ));
        assert!(!state
            .matches_prefix(&mut fixture.read(), known_hash)
            .unwrap());
    }

    #[test]
    fn successive_appends_deliver_exact_suffixes_only() {
        let f = Fixture::new(&vec![b'a'; 20_000]);
        let state = TailFingerprint::capture(&mut f.read()).unwrap();
        assert!(matches!(
            state.read_change(&mut f.read(), false, u64::MAX).unwrap(),
            ReadChange::Unchanged
        ));
        f.append(b"\r\nnext");
        let ReadChange::Appended { bytes, next } =
            state.read_change(&mut f.read(), false, u64::MAX).unwrap()
        else {
            panic!("should reuse the prefix")
        };
        assert_eq!(bytes, b"\r\nnext");
        f.append(b"\nlast\n");
        let ReadChange::Appended { bytes, .. } =
            next.read_change(&mut f.read(), false, u64::MAX).unwrap()
        else {
            panic!("should reuse the new prefix")
        };
        assert_eq!(bytes, b"\nlast\n");
    }

    #[test]
    fn explicit_rotation_and_real_truncation_force_reopen() {
        let f = Fixture::new(b"before\nlonger\n");
        let state = TailFingerprint::capture(&mut f.read()).unwrap();
        f.append(b"more\n");
        assert!(matches!(
            state.read_change(&mut f.read(), true, u64::MAX).unwrap(),
            ReadChange::Replaced
        ));
        fs::write(&f.0, b"small\n").unwrap();
        assert!(matches!(
            state.read_change(&mut f.read(), false, u64::MAX).unwrap(),
            ReadChange::Replaced
        ));
    }

    #[test]
    fn ordinary_buffer_commits_a_split_crlf_without_losing_the_saved_baseline() {
        use crate::edit::EditOptions;
        use crate::{Eol, LineEndings, TextSource};
        use rove_base::{ByteOffset, Span};
        use rove_config::DefaultSettings;

        let f = Fixture::new(b"a\r");
        let mut text = super::super::TextFile::load(
            &f.0,
            "plaintext",
            &DefaultSettings(rove_config::builtin_registry()),
            std::sync::Arc::from([]),
            EditOptions::default(),
        )
        .unwrap();
        let base = text.disk_stamp().unwrap().clone();
        let state = TailFingerprint::capture(&mut f.read()).unwrap();
        f.append(b"\nb\n");
        let ReadChange::Appended { bytes, next } =
            state.read_change(&mut f.read(), false, u64::MAX).unwrap()
        else {
            panic!("must read only the suffix");
        };
        let suffix = std::str::from_utf8(&bytes).unwrap();
        let mut counts = LineEndingCounts::default();
        counts.add("a\r");
        counts.add(suffix);
        let stamp = next.disk_stamp(extend_disk_hash(base.hash, &bytes));
        text.append_from_disk(
            &base,
            stamp.clone(),
            suffix,
            counts.line_endings(),
            std::time::Instant::now(),
        )
        .unwrap();
        assert!(!text.is_dirty());
        assert_eq!(text.disk_stamp(), Some(&stamp));
        assert!(matches!(
            text.line_endings(),
            LineEndings::Mixed {
                preferred: Eol::CrLf
            }
        ));
        assert_eq!(stamp.hash, super::super::content_hash(b"a\r\nb\n"));
        let snapshot = text.snapshot();
        let mut content = String::new();
        snapshot
            .visit_chunks(
                Span::new(ByteOffset(0), ByteOffset(snapshot.byte_len())).unwrap(),
                &mut |chunk| content.push_str(chunk),
            )
            .unwrap();
        assert_eq!(content, "a\r\nb\n");
    }

    #[test]
    fn altered_head_or_tail_invalidates_the_append() {
        let f = Fixture::new(&vec![b'a'; 20_000]);
        let state = TailFingerprint::capture(&mut f.read()).unwrap();
        f.append(b"next\n");
        let mut file = OpenOptions::new().write(true).open(&f.0).unwrap();
        file.seek(SeekFrom::Start(19_999)).unwrap();
        file.write_all(b"b").unwrap();
        drop(file);
        assert!(matches!(
            state.read_change(&mut f.read(), false, u64::MAX).unwrap(),
            ReadChange::Replaced
        ));
    }
}
