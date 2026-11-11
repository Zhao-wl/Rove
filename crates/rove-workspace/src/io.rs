//! Shared-lock tolerant file reads (#15).
//!
//! Unity and antivirus processes commonly hold files open. On Windows, `std::fs::File::open`
//! already requests `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` (documented on
//! `std::os::windows::fs::OpenOptionsExt::share_mode`), so a shared handle never blocks those
//! owners; the retry loop covers the short window where another process opened the file
//! exclusively. A file that stays exclusive returns a clear [`rove_base::Error::Io`] with
//! `ErrorKind::PermissionDenied` instead of panicking, which the UI turns into the "cannot open"
//! placeholder page (UX_SPEC §6.4).

use rove_base::{Error, Result};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

const ATTEMPTS: usize = 5;
const RETRY_DELAY: Duration = Duration::from_millis(10);

/// Reads a whole file, retrying transient sharing violations.
pub fn read_shared(path: &Path) -> Result<Vec<u8>> {
    let mut attempt = 0;
    loop {
        match open_and_read(path) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if is_transient(&error) && attempt + 1 < ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(RETRY_DELAY);
            }
            Err(error) => return Err(classify(error)),
        }
    }
}

/// Rounds a sharing/lock violation to the stable, caller-visible `PermissionDenied` category.
///
/// Windows maps `ERROR_SHARING_VIOLATION`/`ERROR_LOCK_VIOLATION` to `ErrorKind::Uncategorized` on
/// some Rust versions; the UI must be able to branch on "the file is locked" without parsing
/// localized OS text, so both become `PermissionDenied` here.
fn classify(error: std::io::Error) -> Error {
    if is_sharing_violation(&error) {
        Error::Io {
            kind: std::io::ErrorKind::PermissionDenied,
            message: error.to_string(),
        }
    } else {
        Error::from(error)
    }
}

fn open_and_read(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn is_sharing_violation(error: &std::io::Error) -> bool {
    // 32 = ERROR_SHARING_VIOLATION, 33 = ERROR_LOCK_VIOLATION.
    matches!(error.raw_os_error(), Some(32 | 33))
}

fn is_transient(error: &std::io::Error) -> bool {
    is_sharing_violation(error) || error.kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_a_normal_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("A.cs");
        std::fs::write(&path, b"class A {}\n").unwrap();
        assert_eq!(read_shared(&path).unwrap(), b"class A {}\n");
    }

    #[test]
    fn missing_files_report_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let error = read_shared(&dir.path().join("missing.cs")).unwrap_err();
        assert!(matches!(
            error,
            Error::Io {
                kind: std::io::ErrorKind::NotFound,
                ..
            }
        ));
    }

    #[cfg(windows)]
    #[test]
    fn an_exclusively_opened_file_returns_a_clear_error() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Locked.cs");
        {
            let mut lock = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .share_mode(0)
                .open(&path)
                .unwrap();
            lock.write_all(b"locked").unwrap();
            let error = read_shared(&path).unwrap_err();
            assert!(
                matches!(
                    error,
                    Error::Io {
                        kind: std::io::ErrorKind::PermissionDenied,
                        ..
                    }
                ),
                "unexpected error: {error:?}"
            );
        }
        // Once the exclusive owner closes, the shared reader succeeds.
        assert_eq!(read_shared(&path).unwrap(), b"locked");
    }
}
