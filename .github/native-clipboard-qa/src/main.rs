#[path = "native_clipboard.rs"]
mod rove;

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HANDLE, HGLOBAL},
        System::{
            Com::{
                CoInitializeEx, CoUninitialize, IDataObject, COINIT_APARTMENTTHREADED,
                DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL,
            },
            DataExchange::{
                CloseClipboard, CountClipboardFormats, EmptyClipboard, GetClipboardData,
                OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{ReleaseStgMedium, CF_HDROP, DROPEFFECT_MOVE},
        },
        UI::{
            Shell::{
                BHID_DataObject, DragQueryFileW, IShellItem, SHCreateItemFromParsingName, HDROP,
            },
            WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED},
        },
    },
};

fn assert_success(log: &mut File, fixture: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
    {
        return Err("not a GitHub-hosted disposable Windows runner".into());
    }
    let root = PathBuf::from(std::env::var_os("RUNNER_TEMP").ok_or("missing RUNNER_TEMP")?)
        .canonicalize()?;
    if !fixture.canonicalize()?.starts_with(&root) {
        return Err("fixture is outside RUNNER_TEMP; no clipboard call issued".into());
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("isolated file clipboard"),
            WS_OVERLAPPED,
            0,
            0,
            100,
            100,
            None,
            None,
            None,
            None,
        )?
    };
    writeln!(log, "Hidden test HWND created")?;
    let attempt = unsafe { OpenClipboard(Some(hwnd)) };
    writeln!(
        log,
        "OpenClipboard(hidden HWND): {attempt:?}; win32 last error {}",
        io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or_default()
    )?;
    if attempt.is_err() {
        let no_owner = unsafe { OpenClipboard(None) };
        writeln!(
            log,
            "OpenClipboard(None) on disposable runner: {no_owner:?}; win32 last error {}",
            io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or_default()
        )?;
        if no_owner.is_ok() {
            unsafe { CloseClipboard()? };
        }
        return Err("could not open the disposable runner clipboard with its owner window".into());
    }
    let format_count = unsafe { CountClipboardFormats() };
    unsafe { CloseClipboard()? };
    if format_count != 0 || rove::read()?.is_some() {
        return Err("initial isolated clipboard is not empty; no write attempted".into());
    }
    let local = fixture.join("资料 空格, Shell source.txt");
    let second = fixture.join("second file.txt");
    fs::write(&local, b"shell producer")?;
    fs::write(&second, b"rove producer")?;

    // The independent producer asks the Shell itself to emit CF_HDROP data.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let path: Vec<u16> = local.as_os_str().encode_wide().chain([0]).collect();
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None)?;
        let obj: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..FORMATETC::default()
        };
        let mut shell = obj.GetData(&format)?;
        let block = shell.u.hGlobal;
        let len = GlobalSize(block);
        if len == 0 {
            return Err("Shell returned an empty HGLOBAL".into());
        }
        let copy = GlobalAlloc(GMEM_MOVEABLE, len)?;
        let from = GlobalLock(block);
        let to = GlobalLock(copy);
        if from.is_null() || to.is_null() {
            return Err("could not lock Shell data".into());
        }
        ptr::copy_nonoverlapping(from.cast::<u8>(), to.cast::<u8>(), len);
        let _ = GlobalUnlock(block);
        let _ = GlobalUnlock(copy);
        ReleaseStgMedium(&mut shell);
        OpenClipboard(Some(hwnd))?;
        EmptyClipboard()?;
        SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(copy.0)))?;
        CloseClipboard()?;
        drop(obj);
        drop(item);
        CoUninitialize();
    }
    let read = rove::read()?.ok_or("Rove did not read Shell's clipboard")?;
    assert_eq!(read.paths, [local.clone()]);
    assert_eq!(read.effect, rove::Effect::Copy);
    assert_eq!(read.sequence, rove::sequence());
    writeln!(
        log,
        "Shell IDataObject -> actual disposable runner clipboard -> precise Rove read: PASS"
    )?;

    // The independent consumer uses only raw Windows and Shell APIs.
    let next = rove::write(hwnd, &[local.clone(), second.clone()], rove::Effect::Move)?;
    unsafe {
        OpenClipboard(Some(hwnd))?;
        let files = GetClipboardData(CF_HDROP.0 as u32)?;
        let hd = HDROP(files.0);
        assert_eq!(DragQueryFileW(hd, u32::MAX, None), 2);
        for (index, expected) in [local, second].iter().enumerate() {
            let len = DragQueryFileW(hd, index as u32, None) as usize;
            let mut words = vec![0u16; len + 1];
            let read = DragQueryFileW(hd, index as u32, Some(&mut words)) as usize;
            assert_eq!(read, len);
            assert_eq!(&PathBuf::from(OsString::from_wide(&words[..len])), expected);
        }
        let effect = RegisterClipboardFormatW(w!("Preferred DropEffect"));
        let data = HGLOBAL(GetClipboardData(effect)?.0);
        let effect_ptr = GlobalLock(data).cast::<u32>();
        if effect_ptr.is_null() {
            return Err("no preferred drop effect".into());
        }
        assert_eq!(effect_ptr.read_unaligned(), DROPEFFECT_MOVE.0);
        let _ = GlobalUnlock(data);
        CloseClipboard()?;
    }
    assert_eq!(rove::sequence(), next);
    writeln!(log, "Precise Rove write -> actual disposable runner clipboard -> Win32 Shell DragQueryFileW + DWORD MOVE: PASS")?;
    rove::clear_current(hwnd.0 as usize, next)?;
    assert!(rove::read()?.is_none());
    unsafe { DestroyWindow(hwnd)? };
    writeln!(log, "Disposable runner clipboard cleared after testing")?;
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 2 {
        std::process::exit(2);
    }

    let fixture = PathBuf::from(&args[1]);
    let mut log = File::create(fixture.join("worker.log")).expect("fixture log");
    if let Err(error) = assert_success(&mut log, &fixture) {
        let _ = writeln!(log, "FAIL: {error}");
        std::process::exit(1);
    }
}
