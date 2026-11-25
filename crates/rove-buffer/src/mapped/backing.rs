use memmap2::Mmap;
use std::{
    collections::VecDeque,
    fs::File,
    ops::Range,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};

pub(super) const PAGE_BYTES: u64 = 64 * 1024;
const CACHE_PAGES: usize = 8;
const UTF8_LOOKAHEAD: u64 = 3;

pub(super) enum Storage {
    File(Box<DiskBytes>),
    Owned {
        bytes: Box<[u8]>,
        shared_reads: AtomicBool,
    },
}

pub(super) struct DiskBytes {
    path: PathBuf,
    identity: Option<(u64, u64)>,
    skip: u64,
    len: u64,
    shared: AtomicBool,
    // All copies of a snapshot switch backing together, so an editor or
    // worker cannot keep the old mapping alive after its current read ends.
    mapping: Mutex<Mapping>,
    pages: Mutex<VecDeque<(u64, Arc<[u8]>)>>,
}

enum Mapping {
    Active { map: Arc<Mmap>, file: Arc<File> },
    Releasing { map: Weak<Mmap>, file: Weak<File> },
    SharedRead,
}

impl Storage {
    pub(super) fn disk(
        path: PathBuf,
        file: File,
        identity: Option<(u64, u64)>,
        skip: u64,
        len: u64,
        map: Option<Mmap>,
    ) -> Self {
        let shared = map.is_none();
        let mapping = match map {
            Some(map) => Mapping::Active {
                map: Arc::new(map),
                file: Arc::new(file),
            },
            None => Mapping::SharedRead,
        };
        Self::File(Box::new(DiskBytes {
            path,
            identity,
            skip,
            len,
            shared: AtomicBool::new(shared),
            mapping: Mutex::new(mapping),
            pages: Mutex::default(),
        }))
    }

    pub(super) fn owned(bytes: Box<[u8]>, shared_reads: bool) -> Self {
        Self::Owned {
            bytes,
            shared_reads: AtomicBool::new(shared_reads),
        }
    }

    pub(super) fn uses_shared_reads(&self) -> bool {
        match self {
            Self::Owned { shared_reads, .. } => shared_reads.load(Ordering::Relaxed),
            Self::File(disk) => disk.shared.load(Ordering::Acquire),
        }
    }

    pub(super) fn len(&self) -> u64 {
        match self {
            Self::File(disk) => disk.len,
            Self::Owned { bytes, .. } => bytes.len() as u64,
        }
    }

    pub(super) fn has_disk_bytes(&self) -> bool {
        matches!(self, Self::File(_))
    }

    pub(super) fn is_mapped(&self) -> bool {
        let Self::File(disk) = self else { return false };
        match &*disk.mapping.lock().unwrap_or_else(|e| e.into_inner()) {
            Mapping::Active { .. } => true,
            Mapping::Releasing { map, .. } => map.strong_count() > 0,
            Mapping::SharedRead => false,
        }
    }

    pub(super) fn enable_shared_reads(&self) -> bool {
        let disk = match self {
            Self::File(disk) => disk,
            Self::Owned { shared_reads, .. } => {
                shared_reads.store(true, Ordering::Relaxed);
                return true;
            }
        };
        let mut state = disk.mapping.lock().unwrap_or_else(|e| e.into_inner());
        if let Mapping::Active { map, file } = &*state {
            *state = Mapping::Releasing {
                map: Arc::downgrade(map),
                file: Arc::downgrade(file),
            };
            disk.shared.store(true, Ordering::Release);
        }
        if matches!(&*state, Mapping::Releasing { map, file }
            if map.strong_count() != 0 || file.strong_count() != 0)
        {
            return false;
        }
        *state = Mapping::SharedRead;
        true
    }

    pub(super) fn with_range<T>(&self, range: Range<u64>, visitor: impl FnOnce(&[u8]) -> T) -> T {
        if range.is_empty() {
            return visitor(&[]);
        }
        let Self::File(disk) = self else {
            let Self::Owned { bytes, .. } = self else {
                unreachable!()
            };
            return visitor(slice(bytes, range));
        };
        if let Some(map) = disk.mapping_handle() {
            let raw = range.start + disk.skip..range.end + disk.skip;
            return visitor(slice(&map, raw));
        }
        let base = range.start / PAGE_BYTES * PAGE_BYTES;
        if range.end <= base + PAGE_BYTES + UTF8_LOOKAHEAD {
            let page = disk.page(base, range.end);
            return visitor(slice(&page, range.start - base..range.end - base));
        }
        let mut buffer = Vec::new();
        visitor(self.read(range.start, range.end, &mut buffer))
    }

    pub(super) fn byte(&self, offset: u64) -> Option<u8> {
        self.with_range(offset..offset.saturating_add(1), |bytes| {
            bytes.first().copied()
        })
    }

    // The indexer reads directly into its scratch buffer without populating
    // the small viewport cache.
    pub(super) fn read<'a>(&'a self, from: u64, to: u64, buffer: &'a mut Vec<u8>) -> &'a [u8] {
        match self {
            Self::Owned { bytes, .. } => slice(bytes, from..to),
            Self::File(disk) => {
                buffer.resize((to - from) as usize, 0);
                let filled = disk.read_at(buffer, from + disk.skip);
                &buffer[..filled]
            }
        }
    }
}

impl DiskBytes {
    fn mapping_handle(&self) -> Option<Arc<Mmap>> {
        if self.shared.load(Ordering::Acquire) {
            return None;
        }
        match &*self
            .mapping
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            Mapping::Active { map, .. } => Some(map.clone()),
            Mapping::Releasing { .. } | Mapping::SharedRead => None,
        }
    }

    fn read_at(&self, buffer: &mut [u8], offset: u64) -> usize {
        let retained = if self.shared.load(Ordering::Acquire) {
            None
        } else {
            match &*self
                .mapping
                .lock()
                .unwrap_or_else(|error| error.into_inner())
            {
                Mapping::Active { file, .. } => Some(file.clone()),
                Mapping::Releasing { .. } | Mapping::SharedRead => None,
            }
        };
        if let Some(file) = retained {
            return read_at(&file, buffer, offset);
        }
        // A persistent child handle can block rename of an ancestor on Windows
        // even when it shares deletion. Only keep the handle for an actual read.
        let Ok(file) = File::open(&self.path) else {
            return 0;
        };
        if let Some(expected) = self.identity {
            let Ok(metadata) = file.metadata() else {
                return 0;
            };
            if crate::save::tail::file_identity(&file, &metadata) != Some(expected) {
                return 0;
            }
        }
        read_at(&file, buffer, offset)
    }

    fn page(&self, base: u64, end: u64) -> Arc<[u8]> {
        let mut pages = self.pages.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((offset, bytes)) = pages.front() {
            if *offset == base && bytes.len() as u64 >= end - base {
                return bytes.clone();
            }
        }
        if let Some(index) = pages
            .iter()
            .position(|(offset, bytes)| *offset == base && (bytes.len() as u64) >= end - base)
        {
            let entry = pages.remove(index).unwrap();
            let bytes = entry.1.clone();
            pages.push_front(entry);
            return bytes;
        }
        // A cold page faults through the existing shared read handle. Read at
        // most one page and enough right context to keep UTF-8 chunks intact.
        let end = (base + PAGE_BYTES + UTF8_LOOKAHEAD).min(self.len);
        let mut bytes = vec![0; end.saturating_sub(base) as usize];
        let count = self.read_at(&mut bytes, base + self.skip);
        bytes.truncate(count);
        let bytes: Arc<[u8]> = bytes.into();
        pages.retain(|(offset, _)| *offset != base);
        pages.push_front((base, bytes.clone()));
        pages.truncate(CACHE_PAGES);
        bytes
    }
}

fn slice(bytes: &[u8], range: Range<u64>) -> &[u8] {
    let end = (range.end as usize).min(bytes.len());
    bytes.get(range.start as usize..end).unwrap_or_default()
}

fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> usize {
    let mut filled = 0;
    while filled < buffer.len() {
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(
            file,
            &mut buffer[filled..],
            offset + filled as u64,
        );
        #[cfg(unix)]
        let read = std::os::unix::fs::FileExt::read_at(
            file,
            &mut buffer[filled..],
            offset + filled as u64,
        );
        match read {
            Ok(0) | Err(_) => break,
            Ok(read) => filled += read,
        }
    }
    filled
}
