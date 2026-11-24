//! Windows file clipboard. The Shell receives the standard CF_HDROP file
//! list and its preferred copy or move effect.

use std::ffi::OsString;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr;
use windows::{
    core::{w, BOOL},
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE},
        },
        UI::Shell::{DragQueryFileW, DROPFILES, HDROP},
    },
};

const FILES: u32 = CF_HDROP.0 as u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Effect {
    Copy,
    Move,
}

pub(super) struct FileClipboard {
    pub paths: Vec<PathBuf>,
    pub effect: Effect,
    pub sequence: u32,
}

struct Open;

impl Open {
    fn begin(owner: Option<HWND>) -> io::Result<Self> {
        // SAFETY: the optional window handle belongs to the caller's window.
        unsafe { OpenClipboard(owner) }.map_err(io::Error::other)?;
        Ok(Self)
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: this guard balances a successful OpenClipboard.
        let _ = unsafe { CloseClipboard() };
    }
}

struct GlobalBlock(HGLOBAL);

impl GlobalBlock {
    fn new(size: usize, fill: impl FnOnce(*mut u8)) -> io::Result<Self> {
        // SAFETY: this function gives the initialized global block one owner.
        unsafe {
            let block = Self(GlobalAlloc(GMEM_MOVEABLE, size).map_err(io::Error::other)?);
            let pointer = GlobalLock(block.0).cast::<u8>();
            if pointer.is_null() {
                return Err(io::Error::last_os_error());
            }
            fill(pointer);
            let _ = GlobalUnlock(block.0);
            Ok(block)
        }
    }

    fn hand_off(mut self, format: u32) -> io::Result<()> {
        // SAFETY: the clipboard is open, and SetClipboardData owns this
        // moveable allocation after a successful call.
        unsafe { SetClipboardData(format, Some(HANDLE(self.0 .0))) }.map_err(io::Error::other)?;
        self.0 = HGLOBAL::default();
        Ok(())
    }
}

impl Drop for GlobalBlock {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            // SAFETY: this allocation never transferred ownership successfully.
            let _ = unsafe { GlobalFree(Some(self.0)) };
        }
    }
}

fn drop_effect_format() -> io::Result<u32> {
    // SAFETY: w! provides a static NUL-terminated string.
    let format = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) };
    if format == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(format)
}

fn encode_drop_list(paths: &[PathBuf]) -> io::Result<GlobalBlock> {
    if paths.is_empty() || paths.iter().any(|path| !path.is_absolute()) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut list = Vec::new();
    for path in paths {
        let start = list.len();
        list.extend(path.as_os_str().encode_wide());
        if list[start..].contains(&0) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        list.push(0);
    }
    list.push(0);
    let header_size = size_of::<DROPFILES>();
    GlobalBlock::new(header_size + list.len() * size_of::<u16>(), |pointer| {
        let header = DROPFILES {
            pFiles: header_size as u32,
            fWide: BOOL(1),
            ..Default::default()
        };
        // SAFETY: both ranges lie inside the allocated header plus UTF-16 list.
        unsafe {
            ptr::write_unaligned(pointer.cast::<DROPFILES>(), header);
            ptr::copy_nonoverlapping(list.as_ptr(), pointer.add(header_size).cast(), list.len());
        }
    })
}

fn query_drop_list(drop: HDROP) -> io::Result<Vec<PathBuf>> {
    // SAFETY: the HDROP is a live Shell data handle.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let mut paths = Vec::new();
    paths
        .try_reserve(count as usize)
        .map_err(io::Error::other)?;
    for index in 0..count {
        // SAFETY: the size query and read use the same live file list.
        let length = unsafe { DragQueryFileW(drop, index, None) } as usize;
        if !(1..=32767).contains(&length) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let mut path = vec![0; length + 1];
        let read = unsafe { DragQueryFileW(drop, index, Some(&mut path)) } as usize;
        if read != length {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let path = PathBuf::from(OsString::from_wide(&path[..read]));
        if !path.is_absolute() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        paths.push(path);
    }
    Ok(paths)
}

fn query_effect(format: u32) -> Effect {
    // SAFETY: the clipboard guard outlives the locked data. The DWORD is read
    // only from a block large enough to hold it and the lock is then released.
    unsafe {
        let Ok(data) = GetClipboardData(format) else {
            return Effect::Copy;
        };
        let block = HGLOBAL(data.0);
        if GlobalSize(block) < size_of::<u32>() {
            return Effect::Copy;
        }
        let pointer = GlobalLock(block).cast::<u32>();
        if pointer.is_null() {
            return Effect::Copy;
        }
        let effect = pointer.read_unaligned();
        let _ = GlobalUnlock(block);
        if effect == DROPEFFECT_MOVE.0 {
            Effect::Move
        } else {
            Effect::Copy
        }
    }
}

pub(super) fn sequence() -> u32 {
    // SAFETY: no inputs and only returns the clipboard revision.
    unsafe { GetClipboardSequenceNumber() }
}

pub(super) fn read() -> io::Result<Option<FileClipboard>> {
    let _open = Open::begin(None)?;
    let format = drop_effect_format()?;
    // SAFETY: the guard provides ownership for clipboard access, and only
    // the dedicated file list and transfer effect are read.
    if unsafe { IsClipboardFormatAvailable(FILES) }.is_err() {
        return Ok(None);
    }
    let handle = unsafe { GetClipboardData(FILES) }.map_err(io::Error::other)?;
    let paths = query_drop_list(HDROP(handle.0))?;
    let effect = query_effect(format);
    Ok(Some(FileClipboard {
        paths,
        effect,
        sequence: sequence(),
    }))
}

pub(super) fn write(owner: HWND, paths: &[PathBuf], effect: Effect) -> io::Result<u32> {
    let format = drop_effect_format()?;
    let list = encode_drop_list(paths)?;
    let effect = match effect {
        Effect::Copy => DROPEFFECT_COPY.0,
        Effect::Move => DROPEFFECT_MOVE.0,
    };
    let effect = GlobalBlock::new(size_of::<u32>(), |pointer| {
        // SAFETY: this block is exactly the preferred effect DWORD in size.
        unsafe { pointer.cast::<u32>().write_unaligned(effect) };
    })?;
    let _open = Open::begin(Some(owner))?;
    // SAFETY: the clipboard is open and will be closed by the guard.
    unsafe { EmptyClipboard() }.map_err(io::Error::other)?;
    list.hand_off(FILES)?;
    effect.hand_off(format)?;
    Ok(sequence())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_decodes_the_roves_file_list_without_changing_the_desktop_clipboard() {
        let paths = [
            PathBuf::from(r"C:\资料 空格\file,one.cs"),
            PathBuf::from(r"D:\Other\folder"),
        ];
        let block = encode_drop_list(&paths).unwrap();
        let decoded = query_drop_list(HDROP(block.0 .0)).unwrap();
        assert_eq!(decoded, paths);
    }
}

pub(super) fn clear_current(owner: usize, expected: u32) -> io::Result<()> {
    let _open = Open::begin(Some(HWND(owner as *mut _)))?;
    if sequence() == expected {
        // SAFETY: this lock still holds the exact clipboard content written
        // or consumed by this file operation.
        unsafe { EmptyClipboard() }.map_err(io::Error::other)?;
    }
    Ok(())
}
