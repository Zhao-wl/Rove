//! Staged copies and moves from Explorer or the native file clipboard.

use super::*;
use crate::path::relative_str;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Naming {
    Simple,
    Smart,
    Disabled,
}

/// VS Code's duplicate-name rules, applied only after a destination collides.
pub fn next_duplicate_name(name: &str, directory: bool, naming: Naming) -> Option<String> {
    let extension = (!directory)
        .then(|| name.rfind('.'))
        .flatten()
        .filter(|index| *index > 0);
    let (stem, suffix) = extension.map_or((name, ""), |index| name.split_at(index));
    match naming {
        Naming::Disabled => None,
        Naming::Simple => {
            if stem.ends_with(" copy") {
                return Some(format!("{stem} 2{suffix}"));
            }
            if let Some((base, previous)) = stem.rsplit_once(' ') {
                if base.ends_with(" copy") {
                    if let Ok(previous) = previous.parse::<u64>() {
                        if previous == 0 {
                            return Some(format!("{base}{suffix}"));
                        }
                        if let Some(next) = previous.checked_add(1) {
                            return Some(format!("{base} {next}{suffix}"));
                        }
                    }
                }
            }
            Some(format!("{stem} copy{suffix}"))
        }
        Naming::Smart => {
            let start = stem.trim_end_matches(|c: char| c.is_ascii_digit()).len();
            if let Some(next) = increment(&stem[start..]) {
                return Some(format!("{}{next}{suffix}", &stem[..start]));
            }
            if !directory && !suffix.is_empty() {
                let end = stem.bytes().take_while(u8::is_ascii_digit).count();
                if stem[end..].starts_with(['.', '-', '_']) {
                    if let Some(next) = increment(&stem[..end]) {
                        return Some(format!("{next}{}{suffix}", &stem[end..]));
                    }
                }
            }
            let separator = if directory || !suffix.is_empty() {
                "."
            } else {
                ""
            };
            Some(format!("{stem}{separator}1{suffix}"))
        }
    }
}

fn increment(number: &str) -> Option<String> {
    let next = number.parse::<u64>().ok()?.checked_add(1)?;
    Some(format!("{next:0width$}", width = number.len()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transfer {
    Copy,
    Move,
}

#[derive(Debug)]
pub enum TransferOutcome {
    Copied,
    Moved,
    Unchanged,
    /// A cross-volume copy completed, but its original changed or could not
    /// be removed. Keep the successful copy and surface the failure.
    SourceRetained(io::Error),
}

impl Folder {
    pub fn transfer_from(
        &self,
        source: &Path,
        target: &WorkspacePath,
        transfer: Transfer,
    ) -> Result<TransferOutcome> {
        let source = local_entry(source).map_err(|error| disk_error(target, error))?;
        let target_path = self.absolute(target);
        if transfer == Transfer::Move
            && identity_key(&source.to_string_lossy())
                == identity_key(&target_path.to_string_lossy())
        {
            return Ok(TransferOutcome::Unchanged);
        }
        let target_path = self.restore_target(target)?;
        if transfer == Transfer::Move {
            if let Some(relative) = relative_str(self.root(), &source) {
                let relative = WorkspacePath::new(&relative).expect("workspace-relative source");
                self.move_entry(&relative, target)?;
                return Ok(TransferOutcome::Moved);
            }
        }
        transfer_between(&source, &target_path, transfer).map_err(|error| disk_error(target, error))
    }

    /// Undo a move back to the original clipboard path. Refuses to replace an
    /// item that appeared there after the move.
    pub fn undo_transfer(
        &self,
        source: &WorkspacePath,
        original: &Path,
    ) -> Result<TransferOutcome> {
        let (current, _) = self.editable_entry(source)?;
        let parent = original
            .parent()
            .ok_or_else(|| disk_error(source, io::ErrorKind::InvalidInput.into()))?;
        let parent = local_entry(parent).map_err(|error| disk_error(source, error))?;
        let name = original
            .file_name()
            .ok_or_else(|| disk_error(source, io::ErrorKind::InvalidInput.into()))?;
        let target = parent.join(name);
        if let Some(relative) = relative_str(self.root(), &target) {
            let relative = WorkspacePath::new(&relative).expect("workspace-relative destination");
            self.move_entry(source, &relative)?;
            return Ok(TransferOutcome::Moved);
        }
        transfer_between(&current, &target, Transfer::Move)
            .map_err(|error| disk_error(source, error))
    }
}

pub fn inspect_source(path: &Path) -> io::Result<(PathBuf, EntryKind)> {
    let path = local_entry(path)?;
    let kind = if check_entry(&path)?.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::File
    };
    Ok((path, kind))
}

fn local_entry(path: &Path) -> io::Result<PathBuf> {
    use std::path::Component;
    if !path.is_absolute() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        use windows_sys::Win32::{
            Storage::FileSystem::GetDriveTypeW,
            System::WindowsProgramming::{DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOVABLE},
        };
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        let letter = match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
            _ => return Err(io::ErrorKind::Unsupported.into()),
        };
        let drive = [u16::from(letter), u16::from(b':'), u16::from(b'\\'), 0];
        // SAFETY: classify a NUL-terminated drive root before traversing it.
        if !matches!(
            unsafe { GetDriveTypeW(drive.as_ptr()) },
            DRIVE_FIXED | DRIVE_RAMDISK | DRIVE_REMOVABLE
        ) {
            return Err(io::ErrorKind::Unsupported.into());
        }
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => prefix.push(component),
            Component::Normal(part) => {
                prefix.push(part);
                check_entry(&prefix)?;
            }
            _ => return Err(io::ErrorKind::InvalidInput.into()),
        }
    }
    let canonical = fs::canonicalize(path)?;
    check_entry(&canonical)?;
    Ok(canonical)
}

fn transfer_between(
    source: &Path,
    target: &Path,
    transfer: Transfer,
) -> io::Result<TransferOutcome> {
    let source_meta = check_entry(source)?;
    if source_meta.is_dir() {
        // Both paths are absolute and have a canonical parent. Compare by
        // components, including workspace case-insensitivity on Windows.
        let mut target = target.components();
        if source.components().all(|part| {
            target.next().is_some_and(|actual| {
                identity_key(&part.as_os_str().to_string_lossy())
                    == identity_key(&actual.as_os_str().to_string_lossy())
            })
        }) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
    }
    match fs::symlink_metadata(target) {
        Ok(_) => return Err(io::ErrorKind::AlreadyExists.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if transfer == Transfer::Move {
        match rename_without_replacing(source, target) {
            Ok(()) => return Ok(TransferOutcome::Moved),
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {}
            Err(error) => return Err(error),
        }
    }
    let parent = target.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let staging = tempfile::Builder::new()
        .prefix(".rove-copy-")
        .tempdir_in(parent)?;
    let payload = staging.path().join("content");
    let mut observed = Vec::new();
    copy_to_staging(source, &payload, transfer, &mut observed)?;
    rename_without_replacing(&payload, target)?;
    if transfer == Transfer::Copy {
        return Ok(TransferOutcome::Copied);
    }
    let result = (|| {
        for (path, before) in observed {
            let current = EntryStamp::read(&path, &check_entry(&path)?)?;
            if current != before {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source changed during move",
                ));
            }
        }
        if source_meta.is_dir() {
            fs::remove_dir_all(source)
        } else {
            fs::remove_file(source)
        }
    })();
    match result {
        Ok(()) => Ok(TransferOutcome::Moved),
        Err(error) => Ok(TransferOutcome::SourceRetained(error)),
    }
}

fn copy_to_staging(
    source: &Path,
    target: &Path,
    transfer: Transfer,
    observed: &mut Vec<(PathBuf, EntryStamp)>,
) -> io::Result<()> {
    let mut pending = vec![(source.to_path_buf(), target.to_path_buf())];
    while let Some((source, target)) = pending.pop() {
        let metadata = check_entry(&source)?;
        if transfer == Transfer::Move {
            let stamp = EntryStamp::read(&source, &metadata)?;
            observed.push((source.clone(), stamp));
        }
        if metadata.is_dir() {
            fs::create_dir(&target)?;
            for child in fs::read_dir(&source)? {
                let child = child?;
                pending.push((child.path(), target.join(child.file_name())));
            }
        } else {
            fs::copy(source, target)?;
        }
    }
    Ok(())
}
