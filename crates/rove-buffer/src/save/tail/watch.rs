//! Windows extended directory notifications retain the size seen at truncation.
use std::{
    fs::{File, OpenOptions},
    io,
    mem::offset_of,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
};

use windows_sys::Win32::{
    Foundation::{ERROR_IO_INCOMPLETE, ERROR_NOTIFY_ENUM_DIR, HANDLE, WAIT_OBJECT_0},
    Globalization::{CompareStringOrdinal, CSTR_EQUAL},
    Storage::FileSystem::{
        GetLongPathNameW, GetShortPathNameW, ReadDirectoryNotifyExtendedInformation,
        FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME,
        FILE_ACTION_RENAMED_OLD_NAME, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
        FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
        FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_EXTENDED_INFORMATION, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
    System::{
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        Threading::{CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, INFINITE},
        IO::{CancelIoEx, GetOverlappedResult, LPOVERLAPPED_COMPLETION_ROUTINE, OVERLAPPED},
    },
};

type ReadChanges = unsafe extern "system" fn(
    HANDLE,
    *mut std::ffi::c_void,
    u32,
    i32,
    u32,
    *mut u32,
    *mut OVERLAPPED,
    LPOVERLAPPED_COMPLETION_ROUTINE,
    i32,
) -> i32;

fn read_changes() -> io::Result<ReadChanges> {
    static FUNCTION: OnceLock<Option<ReadChanges>> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            let name = "kernel32.dll\0".encode_utf16().collect::<Vec<_>>();
            // SAFETY: the module name and the symbol name are NUL-terminated.
            let module = unsafe { GetModuleHandleW(name.as_ptr()) };
            if module.is_null() {
                return None;
            }
            // SAFETY: the loaded kernel module stays loaded for the process lifetime.
            let address =
                unsafe { GetProcAddress(module, c"ReadDirectoryChangesExW".as_ptr().cast()) }?;
            // SAFETY: ReadChanges matches the documented ABI of this exact symbol.
            Some(unsafe {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, ReadChanges>(address)
            })
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "extended file notifications unavailable",
            )
        })
}

const BUFFER_BYTES: usize = 64 * 1024;
const NAME: usize = offset_of!(FILE_NOTIFY_EXTENDED_INFORMATION, FileName);

struct State {
    generation: AtomicU64,
    live: AtomicBool,
    target: Mutex<Target>,
}

struct Target {
    filenames: Vec<Vec<u16>>,
    last_len: Option<u64>,
}

/// Watches one followed path for a transient decrease in on-disk size.
///
/// Supported on NTFS. If notifications are lost, the generation also advances
/// so callers can rebuild instead of reusing an invalid append baseline.
pub struct TailSizeWatch {
    stop: Arc<OwnedHandle>,
    state: Arc<State>,
    parent_identity: (u64, u64),
}

impl TailSizeWatch {
    /// Opens the parent directory and arms the first read. Do this off the UI thread.
    pub fn start(path: &Path) -> io::Result<Self> {
        let filenames = filename_aliases(path)?;
        let directory = open_parent(path)?;
        let parent_identity = directory_identity(&directory)?;
        let stop = Arc::new(event()?);
        let ready = event()?;
        let state = Arc::new(State {
            generation: AtomicU64::new(0),
            live: AtomicBool::new(false),
            target: Mutex::new(Target {
                filenames,
                last_len: None,
            }),
        });
        let (armed, wait_armed) = mpsc::sync_channel(1);
        let worker_stop = Arc::clone(&stop);
        let worker_state = Arc::clone(&state);
        thread::Builder::new()
            .name("rove-tail-size-watch".into())
            .spawn(move || {
                watch(directory, &worker_stop, &ready, &worker_state, armed);
            })?;
        wait_armed
            .recv()
            .map_err(|_| io::Error::other("file size watcher exited"))??;
        Ok(Self {
            stop,
            state,
            parent_identity,
        })
    }

    /// Synchronize the physical length and aliases from a baseline read after the
    /// monitor was armed. Leave the watcher state untouched if the baseline raced a rotation.
    pub fn accept_baseline(&self, path: &Path, generation: u64, raw_len: u64) -> io::Result<bool> {
        let filenames = filename_aliases(path)?;
        let mut target = self
            .state
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.is_live() || self.generation() != generation {
            return Ok(false);
        }
        *target = Target {
            filenames,
            last_len: Some(target.last_len.map_or(raw_len, |seen| seen.max(raw_len))),
        };
        Ok(true)
    }

    /// Detect replacement of the directory object the original monitor is watching.
    pub fn still_watches_parent(&self, path: &Path) -> io::Result<bool> {
        Ok(directory_identity(&open_parent(path)?)? == self.parent_identity)
    }

    pub fn generation(&self) -> u64 {
        self.state.generation.load(Ordering::Acquire)
    }

    pub fn is_live(&self) -> bool {
        self.state.live.load(Ordering::Acquire)
    }
}

impl Drop for TailSizeWatch {
    fn drop(&mut self) {
        // SAFETY: the Arc keeps the event valid until this call returns.
        unsafe { SetEvent(self.stop.as_raw_handle()) };
    }
}

fn open_parent(path: &Path) -> io::Result<File> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    OpenOptions::new()
        .read(true)
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED)
        .open(parent)
}

fn directory_identity(directory: &File) -> io::Result<(u64, u64)> {
    let metadata = directory.metadata()?;
    super::file_identity(directory, &metadata)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "directory identity unavailable"))
}

fn event() -> io::Result<OwnedHandle> {
    // SAFETY: null security attributes/name request an unnamed, non-inherited event.
    let handle = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful CreateEventW transfers ownership of the new handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn arm(
    directory: &File,
    ready: &OwnedHandle,
    buffer: &mut [u64],
    pending: &mut OVERLAPPED,
) -> io::Result<()> {
    *pending = OVERLAPPED {
        hEvent: ready.as_raw_handle(),
        ..Default::default()
    };
    // SAFETY: all handles are live. The caller keeps buffer and pending stationary
    // until GetOverlappedResult reports completion or the operation is cancelled and drained.
    unsafe {
        ResetEvent(ready.as_raw_handle());
        if read_changes()?(
            directory.as_raw_handle(),
            buffer.as_mut_ptr().cast(),
            BUFFER_BYTES as u32,
            0,
            FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
            ptr::null_mut(),
            pending,
            None,
            ReadDirectoryNotifyExtendedInformation,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn watch(
    directory: File,
    stop: &OwnedHandle,
    ready: &OwnedHandle,
    state: &State,
    armed: mpsc::SyncSender<io::Result<()>>,
) {
    let mut buffer = vec![0u64; BUFFER_BYTES / size_of::<u64>()];
    let mut pending = OVERLAPPED::default();
    let initial = arm(&directory, ready, &mut buffer, &mut pending);
    if initial.is_err() {
        let _ = armed.send(initial);
        return;
    }
    state.live.store(true, Ordering::Release);
    let _ = armed.send(Ok(()));
    let handles = [stop.as_raw_handle(), ready.as_raw_handle()];

    loop {
        // SAFETY: both event handles remain valid for the duration of this wait.
        let result = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        let mut bytes = 0;
        if result != WAIT_OBJECT_0 + 1 {
            // SAFETY: cancel the pending operation and do not release its buffer
            // or OVERLAPPED until the kernel has completed the cancellation.
            unsafe {
                CancelIoEx(directory.as_raw_handle(), &pending);
                GetOverlappedResult(directory.as_raw_handle(), &pending, &mut bytes, 1);
            }
            break;
        }
        // SAFETY: ready was signalled for the active request and buffers are live.
        let completed =
            unsafe { GetOverlappedResult(directory.as_raw_handle(), &pending, &mut bytes, 0) };
        if completed == 0 {
            let code = io::Error::last_os_error().raw_os_error();
            if code == Some(ERROR_IO_INCOMPLETE as i32) {
                // SAFETY: retain the request's backing storage through cancellation.
                unsafe {
                    CancelIoEx(directory.as_raw_handle(), &pending);
                    GetOverlappedResult(directory.as_raw_handle(), &pending, &mut bytes, 1);
                }
            }
            if code != Some(ERROR_NOTIFY_ENUM_DIR as i32) {
                break;
            }
            bytes = 0;
        }
        let mut target = state
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Target {
            filenames,
            last_len,
        } = &mut *target;
        let changed = if bytes == 0 || bytes as usize > BUFFER_BYTES {
            None
        } else {
            // SAFETY: the aligned byte storage is initialized and the OS is done
            // writing it. Only the reported range is exposed.
            let records =
                unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), bytes as usize) };
            invalidated(records, filenames, last_len)
        };
        if changed.is_none() {
            // An overflow provides no trustworthy length for the new generation.
            *last_len = None;
        }
        if changed != Some(false) {
            state.generation.fetch_add(1, Ordering::Release);
        }
        drop(target);
        if arm(&directory, ready, &mut buffer, &mut pending).is_err() {
            break;
        }
    }
    state.generation.fetch_add(1, Ordering::Release);
    state.live.store(false, Ordering::Release);
}

fn invalidated(
    mut records: &[u8],
    filenames: &[Vec<u16>],
    last_len: &mut Option<u64>,
) -> Option<bool> {
    let mut changed = false;
    loop {
        let offset = read_u32(records, 0)? as usize;
        let record = if offset == 0 {
            records
        } else {
            records.get(..offset)?
        };
        let name_bytes = read_u32(
            record,
            offset_of!(FILE_NOTIFY_EXTENDED_INFORMATION, FileNameLength),
        )? as usize;
        if name_bytes == 0 || !name_bytes.is_multiple_of(2) {
            return None;
        }
        let raw_name = record.get(NAME..NAME.checked_add(name_bytes)?)?;
        if filenames
            .iter()
            .any(|filename| filename.len() == raw_name.len() / 2)
        {
            let name = raw_name
                .chunks_exact(2)
                .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
                .collect::<Vec<_>>();
            let same = filenames.iter().any(|filename| {
                // SAFETY: both slices hold the exact number of UTF-16 units provided.
                unsafe {
                    CompareStringOrdinal(
                        name.as_ptr(),
                        name.len() as i32,
                        filename.as_ptr(),
                        filename.len() as i32,
                        1,
                    ) == CSTR_EQUAL
                }
            });
            if same {
                let action =
                    read_u32(record, offset_of!(FILE_NOTIFY_EXTENDED_INFORMATION, Action))?;
                if matches!(action, FILE_ACTION_REMOVED | FILE_ACTION_RENAMED_OLD_NAME) {
                    changed = true;
                    *last_len = None;
                } else {
                    let field = offset_of!(FILE_NOTIFY_EXTENDED_INFORMATION, FileSize);
                    let size = u64::try_from(i64::from_le_bytes(
                        record.get(field..field + 8)?.try_into().ok()?,
                    ))
                    .ok()?;
                    match action {
                        FILE_ACTION_MODIFIED => {
                            changed |= size == 0 || last_len.is_some_and(|previous| size < previous)
                        }
                        FILE_ACTION_ADDED | FILE_ACTION_RENAMED_NEW_NAME => changed = true,
                        _ => return None,
                    }
                    *last_len = Some(size);
                }
            }
        }
        if offset == 0 {
            return Some(changed);
        }
        records = records.get(offset..)?;
    }
}

fn read_u32(record: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        record.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn filename_aliases(path: &Path) -> io::Result<Vec<Vec<u16>>> {
    let filename = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
    let mut aliases = vec![filename.encode_wide().collect()];
    let path = path
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    for query in [GetLongPathNameW, GetShortPathNameW] {
        if let Some(alias) = query_basename(&path, query) {
            if !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
    }
    Ok(aliases)
}

fn query_basename(
    path: &[u16],
    query: unsafe extern "system" fn(*const u16, *mut u16, u32) -> u32,
) -> Option<Vec<u16>> {
    // SAFETY: path is NUL-terminated; zero size queries the required length.
    let capacity = unsafe { query(path.as_ptr(), ptr::null_mut(), 0) };
    if capacity == 0 {
        return None;
    }
    let mut wide = vec![0; capacity as usize];
    // SAFETY: wide is writable for capacity UTF-16 units.
    let len = unsafe { query(path.as_ptr(), wide.as_mut_ptr(), capacity) };
    if len == 0 || len >= capacity {
        return None;
    }
    wide.truncate(len as usize);
    wide.rsplit(|&unit| matches!(unit, 47 | 92))
        .next()
        .filter(|name| !name.is_empty())
        .map(<[u16]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    #[test]
    fn zero_size_notification_invalidates_even_a_zero_sampled_baseline() {
        let name: Vec<u16> = "Editor.log".encode_utf16().collect();
        let mut bytes = vec![0; NAME + name.len() * 2];
        let action = offset_of!(FILE_NOTIFY_EXTENDED_INFORMATION, Action);
        let count = offset_of!(FILE_NOTIFY_EXTENDED_INFORMATION, FileNameLength);
        bytes[action..action + 4].copy_from_slice(&FILE_ACTION_MODIFIED.to_le_bytes());
        bytes[count..count + 4].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
        for (target, code) in bytes[NAME..].chunks_exact_mut(2).zip(&name) {
            target.copy_from_slice(&code.to_le_bytes());
        }
        assert_eq!(invalidated(&bytes, &[name], &mut Some(0)), Some(true));
    }

    #[test]
    fn accepting_an_older_read_does_not_lower_the_observed_size() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Editor.log");
        std::fs::write(&path, vec![b'a'; 8_192]).unwrap();
        let Ok(watch) = TailSizeWatch::start(&path) else {
            eprintln!("已跳过：扩展文件尺寸通知不可用");
            return;
        };
        let generation = watch.generation();
        assert!(watch.accept_baseline(&path, generation, 8_192).unwrap());
        let mut append = OpenOptions::new().append(true).open(&path).unwrap();
        append.write_all(&[b'b'; 8_192]).unwrap();
        append.sync_all().unwrap();
        drop(append);

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let observed = watch.state.target.lock().unwrap().last_len;
            if observed == Some(16_384) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "append size notification did not arrive"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(watch.generation(), generation);
        assert!(watch.accept_baseline(&path, generation, 8_192).unwrap());

        let mut writer = OpenOptions::new().write(true).open(&path).unwrap();
        writer.set_len(12_288).unwrap();
        writer.sync_all().unwrap();
        writer.write_all(&[b'c'; 20_480]).unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        let deadline = Instant::now() + Duration::from_secs(5);
        while watch.generation() == generation && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(watch.generation() > generation);
    }

    #[test]
    fn accepted_baseline_tracks_subsequent_in_place_shrinks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Editor.log");
        std::fs::write(&path, vec![b'a'; 12_288]).unwrap();
        let watch = match TailSizeWatch::start(&path) {
            Ok(watch) => watch,
            Err(error) => {
                eprintln!("已跳过：扩展文件尺寸通知不可用：{error}");
                return;
            }
        };
        let before = watch.generation();
        assert!(watch.accept_baseline(&path, before, 12_288).unwrap());
        let mut writer = OpenOptions::new().write(true).open(&path).unwrap();
        writer.set_len(512).unwrap();
        writer.sync_all().unwrap();
        writer.write_all(&[b'b'; 16_384]).unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        let deadline = Instant::now() + Duration::from_secs(5);
        while watch.generation() == before && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(watch.generation() > before);
        assert!(watch.is_live());
        assert!(!watch.accept_baseline(&path, before, 16_384).unwrap());
    }
}
