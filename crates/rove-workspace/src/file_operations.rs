//! File operations for Explorer (#63). Mutations use the live filesystem and leave
//! the workspace snapshot to the existing watcher.

use crate::{path::identity_key, EntryKind, Folder};
use rove_base::WorkspacePath;
use std::fs::{self, Metadata, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameError {
    Empty,
    Nested,
    DotComponent,
    InvalidCharacter,
    TrailingDotOrSpace,
    ReservedDevice,
    ComponentTooLong,
}

#[derive(Debug)]
pub enum OperationError {
    Name(NameError),
    ReadOnly(WorkspacePath),
    Io {
        path: WorkspacePath,
        source: io::Error,
    },
    /// A concurrent change blocked the recovery of a two-step rename. The
    /// original item is still available at the reported temporary path.
    RenameRecovery {
        temporary: PathBuf,
        source: io::Error,
    },
}

type Result<T> = std::result::Result<T, OperationError>;

#[derive(Debug)]
pub struct CreatedEntry {
    pub path: WorkspacePath,
    pub kind: EntryKind,
    /// Directories created by this operation, from ancestor to descendant.
    pub directories: Vec<WorkspacePath>,
}

/// The item selected before a confirmation. Revalidate it immediately
/// before an operation so a replacement at the same path is left untouched.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryStamp {
    directory: bool,
    bytes: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    #[cfg(windows)]
    file_id: (u32, u64),
}

impl EntryStamp {
    fn same_object(&self, other: &Self) -> bool {
        #[cfg(windows)]
        {
            self.directory == other.directory && self.file_id == other.file_id
        }
        #[cfg(not(windows))]
        {
            self.directory == other.directory && self.created == other.created
        }
    }

    fn read(path: &Path, metadata: &Metadata) -> io::Result<Self> {
        #[cfg(not(windows))]
        let _ = path;
        Ok(Self {
            directory: metadata.is_dir(),
            bytes: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(windows)]
            file_id: windows_file_id(path)?,
        })
    }
}

#[cfg(windows)]
fn windows_file_id(path: &Path) -> io::Result<(u32, u64)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let file = OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the file and initialized output buffer are live for this call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok((info.dwVolumeSerialNumber, id))
}

/// Validates input before constructing a workspace-relative path. Only New
/// File/New Folder accept additional subdirectories in the entered name.
pub fn entry_path(
    parent: Option<&WorkspacePath>,
    input: &str,
    allow_nested: bool,
) -> std::result::Result<WorkspacePath, NameError> {
    if !allow_nested && input.contains('/') {
        return Err(NameError::Nested);
    }
    for component in input.split('/') {
        validate_component(component)?;
    }
    let path = match parent {
        Some(parent) => format!("{}/{input}", parent.as_str()),
        None => input.to_owned(),
    };
    WorkspacePath::new(&path).map_err(|_| NameError::InvalidCharacter)
}

fn validate_component(name: &str) -> std::result::Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if matches!(name, "." | "..") {
        return Err(NameError::DotComponent);
    }
    if name
        .chars()
        .any(|c| c < '\u{20}' || matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*'))
    {
        return Err(NameError::InvalidCharacter);
    }
    if name.ends_with(['.', ' ']) {
        return Err(NameError::TrailingDotOrSpace);
    }
    if name.encode_utf16().count() > 255 {
        return Err(NameError::ComponentTooLong);
    }
    let base = name
        .split_once('.')
        .map_or(name, |(base, _)| base)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || base
        .strip_prefix("COM")
        .or_else(|| base.strip_prefix("LPT"))
        .is_some_and(|index| {
            matches!(
                index,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    {
        return Err(NameError::ReservedDevice);
    }
    Ok(())
}

/// Range in UTF-8 bytes, as expected by GPUI's inline input. Dotfiles with
/// no basename and directory names select the entire name.
pub fn rename_selection(name: &str, kind: EntryKind) -> std::ops::Range<usize> {
    let end = if matches!(kind, EntryKind::File) {
        name.rfind('.').filter(|end| *end > 0).unwrap_or(name.len())
    } else {
        name.len()
    };
    0..end
}

/// Maps a resource through a workspace rename without confusing a sibling
/// with a shared string prefix for a descendant. Suffix casing is preserved.
pub fn path_after_rename(
    path: &WorkspacePath,
    from: &WorkspacePath,
    to: &WorkspacePath,
) -> Option<WorkspacePath> {
    let mut old = path.as_str().split('/');
    for component in from.as_str().split('/') {
        if identity_key(old.next()?) != identity_key(component) {
            return None;
        }
    }
    let mut target = to.as_str().to_owned();
    for component in old {
        target.push('/');
        target.push_str(component);
    }
    WorkspacePath::new(&target).ok()
}

impl Folder {
    pub fn create_entry(
        &self,
        parent: Option<&WorkspacePath>,
        input: &str,
        kind: EntryKind,
    ) -> Result<CreatedEntry> {
        let target = entry_path(parent, input, true).map_err(OperationError::Name)?;
        self.verify_parent(parent)?;
        // Validate all new paths before making the first directory. The
        // watcher may not have indexed them, so evaluate its same rule set.
        let mut prefix = parent.map_or_else(String::new, |parent| parent.as_str().to_owned());
        let paths = input
            .split('/')
            .map(|component| {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(component);
                WorkspacePath::new(&prefix).expect("validated workspace path")
            })
            .collect::<Vec<_>>();
        for path in &paths {
            self.verify_writable(path, false)?;
        }
        let mut directories = Vec::new();
        if let Err(error) = self.create_paths(&paths, &kind, &mut directories) {
            // Remove only directories created by this call, and only if an
            // outside process has not populated them in the meantime.
            for path in directories.iter().rev() {
                let _ = fs::remove_dir(self.absolute(path));
            }
            return Err(error);
        }
        Ok(CreatedEntry {
            path: target,
            kind,
            directories,
        })
    }

    fn create_paths(
        &self,
        paths: &[WorkspacePath],
        kind: &EntryKind,
        directories: &mut Vec<WorkspacePath>,
    ) -> Result<()> {
        for (index, path) in paths.iter().enumerate() {
            let absolute = self.absolute(path);
            let is_last = index + 1 == paths.len();
            if is_last && matches!(kind, EntryKind::File) {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(absolute)
                    .map_err(|source| disk_error(path, source))?;
                continue;
            }
            match fs::create_dir(&absolute) {
                Ok(()) => directories.push(path.clone()),
                Err(error) if !is_last && error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = check_directory(&absolute).map_err(|e| disk_error(path, e))?;
                    self.verify_writable(path, metadata.permissions().readonly())?;
                }
                Err(source) => return Err(disk_error(path, source)),
            }
        }
        Ok(())
    }

    pub fn rename_entry(&self, source: &WorkspacePath, input: &str) -> Result<WorkspacePath> {
        let parent = parent_path(source);
        let target = entry_path(parent.as_ref(), input, false).map_err(OperationError::Name)?;
        self.move_entry(source, &target)?;
        Ok(target)
    }

    /// Validates the source and destination against the current workspace before
    /// moving it. The watcher publishes the rename and updates the tree.
    pub fn move_entry(&self, source: &WorkspacePath, target: &WorkspacePath) -> Result<()> {
        let (source_path, metadata) = self.editable_entry(source)?;
        self.verify_parent(parent_path(target).as_ref())?;
        self.verify_writable(target, false)?;
        if source == target {
            return Ok(());
        }
        let source_key = identity_key(source.as_str());
        let target_key = identity_key(target.as_str());
        if target_key
            .strip_prefix(&source_key)
            .is_some_and(|suffix| suffix.starts_with('/'))
        {
            return Err(disk_error(target, io::ErrorKind::InvalidInput.into()));
        }
        if metadata.is_dir() {
            self.verify_writable_descendants(&source_path, Some(source), Some(target))?;
        }
        let target_path = self.absolute(target);
        if source_key == target_key {
            return rename_case_only(&source_path, &target_path, target);
        }
        rename_without_replacing(&source_path, &target_path)
            .map_err(|error| disk_error(target, error))
    }

    /// Also used by the Windows Recycle Bin adapter before it queues shell items.
    pub fn editable_path(&self, path: &WorkspacePath) -> Result<PathBuf> {
        self.editable_tree(path).map(|(path, _)| path)
    }

    pub fn entry_stamp(&self, path: &WorkspacePath) -> Result<EntryStamp> {
        let (absolute, metadata) = self.editable_tree(path)?;
        EntryStamp::read(&absolute, &metadata).map_err(|error| disk_error(path, error))
    }

    pub fn checked_entry_path(
        &self,
        path: &WorkspacePath,
        expected: &EntryStamp,
    ) -> Result<PathBuf> {
        self.editable_entry_matching(path, expected)
            .map(|(absolute, _)| absolute)
    }

    pub fn checked_entry_identity(
        &self,
        path: &WorkspacePath,
        expected: &EntryStamp,
    ) -> Result<PathBuf> {
        let (absolute, metadata) = self.editable_tree(path)?;
        let live =
            EntryStamp::read(&absolute, &metadata).map_err(|error| disk_error(path, error))?;
        if !live.same_object(expected) {
            return Err(disk_error(path, io::ErrorKind::InvalidData.into()));
        }
        Ok(absolute)
    }

    pub fn remove_empty_directory(
        &self,
        path: &WorkspacePath,
        expected: &EntryStamp,
    ) -> Result<()> {
        let absolute = self.checked_entry_identity(path, expected)?;
        fs::remove_dir(absolute).map_err(|error| disk_error(path, error))
    }

    pub fn restore_target(&self, path: &WorkspacePath) -> Result<PathBuf> {
        self.verify_parent(parent_path(path).as_ref())?;
        self.verify_writable(path, false)?;
        let absolute = self.absolute(path);
        match fs::symlink_metadata(&absolute) {
            Ok(_) => Err(disk_error(path, io::ErrorKind::AlreadyExists.into())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(absolute),
            Err(error) => Err(disk_error(path, error)),
        }
    }

    /// Permanently deletes one validated item. The caller obtains confirmation.
    pub fn remove_permanently(&self, path: &WorkspacePath, expected: &EntryStamp) -> Result<()> {
        let (absolute, metadata) = self.editable_entry_matching(path, expected)?;
        let result = if metadata.is_dir() {
            fs::remove_dir_all(absolute)
        } else {
            fs::remove_file(absolute)
        };
        result.map_err(|error| disk_error(path, error))
    }

    fn editable_entry_matching(
        &self,
        path: &WorkspacePath,
        expected: &EntryStamp,
    ) -> Result<(PathBuf, Metadata)> {
        let (absolute, metadata) = self.editable_tree(path)?;
        let live =
            EntryStamp::read(&absolute, &metadata).map_err(|error| disk_error(path, error))?;
        if &live != expected {
            return Err(disk_error(
                path,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "entry was replaced or modified while awaiting the operation",
                ),
            ));
        }
        Ok((absolute, metadata))
    }

    fn editable_tree(&self, path: &WorkspacePath) -> Result<(PathBuf, Metadata)> {
        let (absolute, metadata) = self.editable_entry(path)?;
        if metadata.is_dir() {
            self.verify_writable_descendants(&absolute, Some(path), None)?;
        }
        Ok((absolute, metadata))
    }

    fn verify_writable_descendants(
        &self,
        absolute: &Path,
        source: Option<&WorkspacePath>,
        target: Option<&WorkspacePath>,
    ) -> Result<()> {
        let root = source.or(target).expect("one workspace side");
        let mut pending = vec![(absolute.to_path_buf(), source.cloned(), target.cloned())];
        while let Some((directory, source, target)) = pending.pop() {
            let label = source.as_ref().or(target.as_ref()).unwrap_or(root);
            for entry in fs::read_dir(&directory).map_err(|error| disk_error(label, error))? {
                let entry = entry.map_err(|error| disk_error(label, error))?;
                let absolute = entry.path();
                let metadata =
                    fs::symlink_metadata(&absolute).map_err(|error| disk_error(label, error))?;
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| disk_error(label, io::ErrorKind::InvalidData.into()))?;
                let project = |parent: Option<&WorkspacePath>| {
                    parent
                        .map(|parent| {
                            WorkspacePath::new(&format!("{}/{name}", parent.as_str()))
                                .map_err(|_| disk_error(parent, io::ErrorKind::InvalidData.into()))
                        })
                        .transpose()
                };
                let source = project(source.as_ref())?;
                let target = project(target.as_ref())?;
                if let Some(path) = &source {
                    self.verify_writable(path, metadata.permissions().readonly())?;
                }
                if let Some(path) = &target {
                    self.verify_writable(path, false)?;
                }
                if metadata.is_dir() && !is_link(&metadata) {
                    pending.push((absolute, source, target));
                }
            }
        }
        Ok(())
    }

    fn editable_entry(&self, path: &WorkspacePath) -> Result<(PathBuf, Metadata)> {
        self.verify_parent(parent_path(path).as_ref())?;
        let absolute = self.absolute(path);
        let metadata = check_entry(&absolute).map_err(|e| disk_error(path, e))?;
        self.verify_writable(path, metadata.permissions().readonly())?;
        Ok((absolute, metadata))
    }

    fn verify_parent(&self, parent: Option<&WorkspacePath>) -> Result<()> {
        if let Some(parent) = parent {
            let mut relative = String::new();
            let mut absolute = self.root().to_path_buf();
            for component in parent.as_str().split('/') {
                if !relative.is_empty() {
                    relative.push('/');
                }
                relative.push_str(component);
                absolute.push(component);
                let relative = WorkspacePath::new(&relative).expect("valid parent component");
                let metadata = check_directory(&absolute).map_err(|e| disk_error(&relative, e))?;
                self.verify_writable(&relative, metadata.permissions().readonly())?;
            }
        }
        Ok(())
    }

    fn verify_writable(&self, path: &WorkspacePath, permission_read_only: bool) -> Result<()> {
        if !self
            .config()
            .readonly
            .evaluate(path.as_str(), permission_read_only)
            .is_empty()
        {
            return Err(OperationError::ReadOnly(path.clone()));
        }
        Ok(())
    }
}

fn parent_path(path: &WorkspacePath) -> Option<WorkspacePath> {
    path.as_str()
        .rsplit_once('/')
        .map(|(parent, _)| WorkspacePath::new(parent).expect("valid parent path"))
}

fn disk_error(path: &WorkspacePath, source: io::Error) -> OperationError {
    OperationError::Io {
        path: path.clone(),
        source,
    }
}

fn check_directory(path: &Path) -> io::Result<Metadata> {
    let metadata = check_entry(path)?;
    if !metadata.is_dir() {
        return Err(io::ErrorKind::NotADirectory.into());
    }
    Ok(metadata)
}

fn check_entry(path: &Path) -> io::Result<Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if is_link(&metadata) || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(metadata)
}

#[cfg(windows)]
fn is_link(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link(metadata: &Metadata) -> bool {
    metadata.is_symlink()
}

fn rename_case_only(source: &Path, target: &Path, label: &WorkspacePath) -> Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = source.parent().expect("entry under workspace root");
    let temporary = loop {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".rove-rename-{}-{id:x}", std::process::id()));
        match rename_without_replacing(source, &candidate) {
            Ok(()) => break candidate,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(disk_error(label, error)),
        }
    };
    match rename_without_replacing(&temporary, target) {
        Ok(()) => Ok(()),
        Err(error) => match rename_without_replacing(&temporary, source) {
            Ok(()) => Err(disk_error(label, error)),
            Err(source) => Err(OperationError::RenameRecovery { temporary, source }),
        },
    }
}

#[cfg(windows)]
fn rename_without_replacing(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let path = |p: &Path| p.as_os_str().encode_wide().chain([0]).collect::<Vec<_>>();
    let source = path(source);
    let target = path(target);
    // SAFETY: both UTF-16 path buffers are NUL-terminated and live through
    // the call. Zero flags require an atomic same-volume move without overwrite.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn rename_without_replacing(source: &Path, target: &Path) -> io::Result<()> {
    match fs::symlink_metadata(target) {
        Ok(_) => Err(io::ErrorKind::AlreadyExists.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(source, target),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
pub mod recycle;

pub mod transfer;
