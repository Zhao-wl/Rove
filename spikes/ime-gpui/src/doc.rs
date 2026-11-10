//! Read-only document backed by mmap + a background-built line offset index.
//! Identical copy lives in spikes/ui-wgpu/src/doc.rs so both prototypes are comparable.

use std::borrow::Cow;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use memmap2::Mmap;

/// Bytes of the file indexed synchronously before the first frame, so the first
/// screen is available immediately regardless of file size.
const EAGER_BYTES: usize = 1 << 20; // 1 MiB
const CHUNK: usize = 32 << 20; // 32 MiB per background step

pub struct LineIndex {
    /// Byte offset of the start of every known line. `offsets[0] == 0`.
    pub offsets: RwLock<Vec<u64>>,
    pub complete: AtomicBool,
    pub scanned: AtomicU64,
    pub total: u64,
    /// ms from `Doc::open` to the moment the full index finished.
    pub full_index_ms: AtomicU64,
}

impl LineIndex {
    pub fn known_lines(&self) -> usize {
        self.offsets.read().unwrap().len()
    }
    pub fn progress(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.scanned.load(Ordering::Relaxed) as f64 / self.total as f64
        }
    }
}

pub struct Doc {
    pub path: PathBuf,
    pub mmap: Arc<Mmap>,
    pub index: Arc<LineIndex>,
    /// ms spent in `open()` (mmap + eager index of the first screen).
    pub open_ms: f64,
}

impl Doc {
    pub fn open(path: &Path) -> std::io::Result<Doc> {
        let t0 = Instant::now();
        let file = File::open(path)?;
        let mmap = Arc::new(unsafe { Mmap::map(&file)? });
        let total = mmap.len() as u64;

        // Eagerly index the head of the file: enough for the first screen.
        let head = &mmap[..mmap.len().min(EAGER_BYTES)];
        let mut offsets: Vec<u64> = Vec::with_capacity(1024);
        offsets.push(0);
        for pos in memchr::memchr_iter(b'\n', head) {
            offsets.push(pos as u64 + 1);
        }
        let eager_end = head.len() as u64;

        let index = Arc::new(LineIndex {
            offsets: RwLock::new(offsets),
            complete: AtomicBool::new(total <= EAGER_BYTES as u64),
            scanned: AtomicU64::new(eager_end),
            total,
            full_index_ms: AtomicU64::new(0),
        });
        let open_ms = t0.elapsed().as_secs_f64() * 1000.0;

        if !index.complete.load(Ordering::Relaxed) {
            // Background indexing of the remainder.
            let mmap2 = mmap.clone();
            let index2 = index.clone();
            std::thread::spawn(move || {
                let bytes: &[u8] = &mmap2;
                let mut pos = eager_end as usize;
                while pos < bytes.len() {
                    let end = (pos + CHUNK).min(bytes.len());
                    let mut local: Vec<u64> = Vec::with_capacity(CHUNK / 40);
                    for p in memchr::memchr_iter(b'\n', &bytes[pos..end]) {
                        local.push((pos + p) as u64 + 1);
                    }
                    {
                        let mut w = index2.offsets.write().unwrap();
                        w.extend_from_slice(&local);
                    }
                    pos = end;
                    index2.scanned.store(pos as u64, Ordering::Relaxed);
                }
                index2
                    .full_index_ms
                    .store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
                index2.complete.store(true, Ordering::Release);
            });
        } else {
            index
                .full_index_ms
                .store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
        }

        Ok(Doc {
            path: path.to_path_buf(),
            mmap,
            index,
            open_ms,
        })
    }

    pub fn len(&self) -> u64 {
        self.mmap.len() as u64
    }

    pub fn line_count(&self) -> usize {
        self.index.known_lines()
    }

    /// Text of line `i` (0-based), without the trailing newline. Clamped so one
    /// pathological line cannot stall layout.
    pub fn line(&self, i: usize, max_bytes: usize) -> Cow<'_, str> {
        let offsets = self.index.offsets.read().unwrap();
        if i >= offsets.len() {
            return Cow::Borrowed("");
        }
        let start = offsets[i] as usize;
        let end = if i + 1 < offsets.len() {
            (offsets[i + 1] as usize).saturating_sub(1)
        } else {
            self.mmap.len()
        };
        drop(offsets);
        let end = end.min(start + max_bytes).min(self.mmap.len());
        let mut slice = &self.mmap[start..end.max(start)];
        if slice.last() == Some(&b'\r') {
            slice = &slice[..slice.len() - 1];
        }
        match std::str::from_utf8(slice) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => Cow::Owned(String::from_utf8_lossy(slice).into_owned()),
        }
    }
}
