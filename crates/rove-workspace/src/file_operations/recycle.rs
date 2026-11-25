//! Shell Recycle Bin adapter. Recycling and permanent deletion have separate
//! entry points. A failed recycle leaves the decision to the confirmation UI.

use std::io;
use std::path::Path;
use std::{ptr, slice, thread};

use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        System::Com::{
            CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{
            BHID_Transfer, Common::ITEMIDLIST, FOLDERID_RecycleBinFolder, ILGetSize, IShellItem,
            ITransferSource, SHCreateItemFromIDList, SHCreateItemFromParsingName,
            SHGetIDListFromObject, SHGetKnownFolderItem, KF_FLAG_DEFAULT,
        },
    },
};

/// Exact Shell identity returned by the recycle operation. Retains no COM
/// interface when moved between the filesystem worker and undo state.
#[derive(Debug)]
pub struct RecycledItem(Vec<u8>);

struct Apartment;

impl Apartment {
    fn initialize() -> windows::core::Result<Self> {
        // SAFETY: initialized and uninitialized on the same worker thread.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the one successful initialization on this thread.
        unsafe { CoUninitialize() };
    }
}

fn in_shell_apartment<T: Send>(operation: impl FnOnce() -> io::Result<T> + Send) -> io::Result<T> {
    match Apartment::initialize() {
        Ok(_apartment) => operation(),
        // Production filesystem workers may already belong to the MTA. Keep the caller's
        // apartment intact and create all Shell interfaces on a fresh STA thread.
        Err(error) if error.code() == RPC_E_CHANGED_MODE => thread::scope(|scope| {
            let worker = thread::Builder::new()
                .name("rove-shell-recycle".into())
                .spawn_scoped(scope, move || {
                    let _apartment = Apartment::initialize().map_err(io::Error::other)?;
                    operation()
                })?;
            worker
                .join()
                .map_err(|_| io::Error::other("Shell Recycle Bin worker panicked"))?
        }),
        Err(error) => Err(io::Error::other(error)),
    }
}

struct ShellId(*mut ITEMIDLIST);

impl Drop for ShellId {
    fn drop(&mut self) {
        // SAFETY: the owned pointer is from a Shell out parameter or CoTaskMemAlloc.
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

fn shell_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let source = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let unc = [92, 92, 63, 92, 85, 78, 67, 92]; // \\?\UNC\
    let verbatim = [92, 92, 63, 92]; // \\?\
    let mut path = if let Some(suffix) = source.strip_prefix(&unc) {
        [92, 92].into_iter().chain(suffix.iter().copied()).collect()
    } else if let Some(suffix) = source.strip_prefix(&verbatim) {
        suffix.to_vec()
    } else {
        source
    };
    path.push(0);
    path
}

fn shell_item(path: &Path) -> io::Result<IShellItem> {
    let path = shell_path(path);
    // SAFETY: the path is a live NUL-terminated UTF-16 string.
    unsafe { SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None) }.map_err(io::Error::other)
}

/// Checks known Shell limitations before the application asks for confirmation.
pub fn available(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
    use windows_sys::Win32::{
        Storage::FileSystem::GetDriveTypeW,
        System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
    };

    if shell_path(path).len() > 260 {
        return false;
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return false;
    };
    let letter = match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
        _ => return false,
    };
    let drive = [u16::from(letter), u16::from(b':'), u16::from(b'\\'), 0];
    // SAFETY: the path is a live, NUL-terminated drive root. The Shell query
    // receives an initialized buffer for the documented output type.
    unsafe {
        if !matches!(GetDriveTypeW(drive.as_ptr()), DRIVE_FIXED | DRIVE_REMOVABLE) {
            return false;
        }
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            ..Default::default()
        };
        SHQueryRecycleBinW(PCWSTR(drive.as_ptr()), &mut info).is_ok()
    }
}

pub fn unavailable(error: &io::Error) -> bool {
    use windows::Win32::UI::Shell::{
        COPYENGINE_E_RECYCLE_BIN_NOT_FOUND, COPYENGINE_E_RECYCLE_FORCE_NUKE,
        COPYENGINE_E_RECYCLE_PATH_TOO_LONG, COPYENGINE_E_RECYCLE_SIZE_TOO_BIG,
    };
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<windows::core::Error>())
        .is_some_and(|error| {
            matches!(
                error.code(),
                COPYENGINE_E_RECYCLE_BIN_NOT_FOUND
                    | COPYENGINE_E_RECYCLE_FORCE_NUKE
                    | COPYENGINE_E_RECYCLE_PATH_TOO_LONG
                    | COPYENGINE_E_RECYCLE_SIZE_TOO_BIG
            )
        })
}

/// Call from a filesystem worker. Shell calls can block while the file is transferred.
pub fn recycle(path: &Path) -> io::Result<RecycledItem> {
    in_shell_apartment(|| recycle_item(path))
}

fn recycle_item(path: &Path) -> io::Result<RecycledItem> {
    let item = shell_item(path)?;
    let result = (|| -> windows::core::Result<RecycledItem> {
        // SAFETY: COM is initialized. All items/handlers are used on this
        // apartment. The recycle method returns the new item in the Recycle Bin.
        unsafe {
            let parent = item.GetParent()?;
            let transfer: ITransferSource = parent.BindToHandler(None, &BHID_Transfer)?;
            let bin: IShellItem =
                SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)?;
            let recycled = transfer.RecycleItem(&item, &bin, 0)?;
            let id = ShellId(SHGetIDListFromObject(&recycled)?);
            if id.0.is_null() {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_POINTER,
                ));
            }
            let size = ILGetSize(Some(id.0)) as usize;
            let bytes = slice::from_raw_parts(id.0.cast::<u8>(), size).to_vec();
            Ok(RecycledItem(bytes))
        }
    })();
    result.map_err(io::Error::other)
}

/// Call from a filesystem worker. Only the PIDL bytes cross the caller's apartment.
pub fn restore(recycled: &RecycledItem, path: &Path) -> io::Result<()> {
    in_shell_apartment(|| restore_item(recycled, path))
}

fn restore_item(recycled: &RecycledItem, path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let name = shell_path(Path::new(name));
    let destination = shell_item(parent)?;
    let result = (|| -> windows::core::Result<()> {
        // SAFETY: the token contains bytes from an absolute Shell ID list.
        // Use COM-allocated aligned memory before passing it back to the API.
        unsafe {
            let id = ShellId(CoTaskMemAlloc(recycled.0.len()).cast());
            if id.0.is_null() {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_OUTOFMEMORY,
                ));
            }
            ptr::copy_nonoverlapping(recycled.0.as_ptr(), id.0.cast::<u8>(), recycled.0.len());
            let item: IShellItem = SHCreateItemFromIDList(id.0)?;
            let bin_parent = item.GetParent()?;
            let transfer: ITransferSource = bin_parent.BindToHandler(None, &BHID_Transfer)?;
            // Transfer flags 0 fail at a name collision instead of overwriting.
            transfer.MoveItem(&item, &destination, PCWSTR(name.as_ptr()), 0)?;
            Ok(())
        }
    })();
    result.map_err(io::Error::other)
}
