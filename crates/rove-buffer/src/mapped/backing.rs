use memmap2::Mmap;
use std::{
    collections::VecDeque,
    fs::File,
    ops::Range,
    sync::{Arc, Mutex, Weak},
};

pub(super) const PAGE_BYTES: u64 = 64 * 1024;
const CACHE_PAGES: usize = 8;
const UTF8_LOOKAHEAD: u64 = 3;

pub(super) enum Storage {
    File(Box<DiskBytes>),
    Owned(Box<[u8]>),
}

pub(super) struct DiskBytes {
    file: File,
    skip: u64,
    len: u64,
    // Existing snapshots can begin using the same shared handle without
    // leaving their old mapping alive in other editor or worker references.
    mapping: Mutex<Mapping>,
    pages: Mutex<VecDeque<(u64, Arc<[u8]>)>>,
}

enum Mapping {
    Active(Arc<Mmap>),
    Releasing(Weak<Mmap>),
    SharedRead,
}

impl Storage {
    pub(super) fn disk(file: File, skip: u64, len: u64, map: Option<Mmap>) -> Self {
        Self::File(Box::new(DiskBytes {
            file,
            skip,
            len,
            mapping: Mutex::new(
                map.map_or(Mapping::SharedRead, |map| Mapping::Active(Arc::new(map))),
            ),
            pages: Mutex::default(),
        }))
    }

    pub(super) fn len(&self) -> u64 {
        match self {
            Self::File(disk) => disk.len,
            Self::Owned(bytes) => bytes.len() as u64,
        }
    }

    pub(super) fn has_disk_bytes(&self) -> bool {
        matches!(self, Self::File(_))
    }

    pub(super) fn is_mapped(&self) -> bool {
        let Self::File(disk) = self else { return false };
        match &*disk.mapping.lock().unwrap_or_else(|e| e.into_inner()) {
            Mapping::Active(_) => true,
            Mapping::Releasing(old) => old.strong_count() > 0,
            Mapping::SharedRead => false,
        }
    }

    pub(super) fn enable_shared_reads(&self) -> bool {
        let Self::File(disk) = self else { return true };
        let mut state = disk.mapping.lock().unwrap_or_else(|e| e.into_inner());
        if let Mapping::Active(map) = &*state {
            *state = Mapping::Releasing(Arc::downgrade(map));
        }
        if matches!(&*state, Mapping::Releasing(old) if old.strong_count() != 0) {
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
            let Self::Owned(bytes) = self else {
                unreachable!()
            };
            return visitor(slice(bytes, range));
        };
        let map = match &*disk.mapping.lock().unwrap_or_else(|e| e.into_inner()) {
            Mapping::Active(map) => Some(map.clone()),
            Mapping::Releasing(_) | Mapping::SharedRead => None,
        };
        if let Some(map) = map {
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

    // Indexing always reads directly from the shared handle using its own
    // scratch buffer. It does not populate the small viewport cache.
    pub(super) fn read<'a>(&'a self, from: u64, to: u64, buffer: &'a mut Vec<u8>) -> &'a [u8] {
        match self {
            Self::Owned(bytes) => slice(bytes, from..to),
            Self::File(disk) => {
                buffer.resize((to - from) as usize, 0);
                let filled = read_at(&disk.file, buffer, from + disk.skip);
                &buffer[..filled]
            }
        }
    }
}

impl DiskBytes {
    fn page(&self, base: u64, end: u64) -> Arc<[u8]> {
        let mut pages = self.pages.lock().unwrap_or_else(|e| e.into_inner());
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
        let count = read_at(&self.file, &mut bytes, base + self.skip);
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
