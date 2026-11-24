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
        self.verify_transfer_tree(&source, target, transfer)?;
        transfer_between(&source, &target_path, transfer).map_err(|error| disk_error(target, error))
    }

    /// Check a transfer's source and projected destination before showing its confirmation.
    pub fn verify_transfer_tree(
        &self,
        source: &Path,
        target: &WorkspacePath,
        transfer: Transfer,
    ) -> Result<()> {
        let from = (transfer == Transfer::Move)
            .then(|| {
                relative_str(self.root(), source).and_then(|path| WorkspacePath::new(&path).ok())
            })
            .flatten();
        if let Some(from) = &from {
            self.editable_entry(from)?;
        }
        self.verify_writable(target, false)?;
        if check_entry(source)
            .map_err(|error| disk_error(target, error))?
            .is_dir()
        {
            if enters_directory(source, &self.absolute(target)) {
                return Err(disk_error(target, io::ErrorKind::InvalidInput.into()));
            }
            self.verify_writable_descendants(source, from.as_ref(), Some(target))?;
        }
        Ok(())
    }

    /// Undo a move back to the original clipboard path. Refuses to replace an
    /// item that appeared there after the move.
    pub fn undo_transfer(
        &self,
        source: &WorkspacePath,
        original: &Path,
    ) -> Result<TransferOutcome> {
        let (current, _) = self.editable_tree(source)?;
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

pub struct TransferSource {
    pub path: PathBuf,
    pub kind: EntryKind,
    stamp: EntryStamp,
}

impl TransferSource {
    pub fn verify(&self) -> io::Result<()> {
        let path = local_entry(&self.path)?;
        let live = EntryStamp::read(&path, &check_entry(&path)?)?;
        if live != self.stamp {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "transfer source changed since selection",
            ));
        }
        Ok(())
    }
}

pub fn inspect_source(path: &Path) -> io::Result<TransferSource> {
    let path = local_entry(path)?;
    let metadata = check_entry(&path)?;
    let kind = if metadata.is_dir() {
        EntryKind::Directory
    } else {
        EntryKind::File
    };
    let stamp = EntryStamp::read(&path, &metadata)?;
    Ok(TransferSource { path, kind, stamp })
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

fn enters_directory(source: &Path, target: &Path) -> bool {
    let mut target = target.components();
    source.components().all(|part| {
        target.next().is_some_and(|actual| {
            identity_key(&part.as_os_str().to_string_lossy())
                == identity_key(&actual.as_os_str().to_string_lossy())
        })
    })
}

fn transfer_between(
    source: &Path,
    target: &Path,
    transfer: Transfer,
) -> io::Result<TransferOutcome> {
    let source_meta = check_entry(source)?;
    if source_meta.is_dir() && enters_directory(source, target) {
        return Err(io::ErrorKind::InvalidInput.into());
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
        for (path, before) in &observed {
            let current = EntryStamp::read(path, &check_entry(path)?)?;
            if current != *before {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source changed during move",
                ));
            }
        }
        remove_copied_entries(observed)
    })();
    match result {
        Ok(()) => Ok(TransferOutcome::Moved),
        Err(error) => Ok(TransferOutcome::SourceRetained(error)),
    }
}

fn remove_copied_entries(observed: Vec<(PathBuf, EntryStamp)>) -> io::Result<()> {
    // Only remove items in the copy manifest. Removing a directory recursively
    // could discard new files that arrived after the copy and validation.
    for (path, before) in observed.into_iter().rev() {
        let current = EntryStamp::read(&path, &check_entry(&path)?)?;
        if before.directory {
            if !current.same_object(&before) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source directory was replaced",
                ));
            }
            fs::remove_dir(path)?;
        } else {
            if current != before {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source file changed during move",
                ));
            }
            fs::remove_file(path)?;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_cleanup_preserves_files_created_after_copy() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let target = directory.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("copied.txt"), "first").unwrap();
        let mut observed = Vec::new();
        copy_to_staging(&source, &target, Transfer::Move, &mut observed).unwrap();

        fs::write(source.join("arrived-later.txt"), "keep me").unwrap();
        assert!(remove_copied_entries(observed).is_err());
        assert!(!source.join("copied.txt").exists());
        assert_eq!(
            fs::read_to_string(target.join("copied.txt")).unwrap(),
            "first"
        );
        assert_eq!(
            fs::read_to_string(source.join("arrived-later.txt")).unwrap(),
            "keep me"
        );
    }
}
