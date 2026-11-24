use rove_base::WorkspacePath;
use rove_workspace::file_operations::{entry_path, rename_selection, NameError, OperationError};
use rove_workspace::{EntryKind, Folder, WorkspaceConfig};
use std::fs;
use std::io;

fn path(value: &str) -> WorkspacePath {
    WorkspacePath::new(value).unwrap()
}

#[test]
fn inline_names_validate_each_component_and_select_the_basename() {
    for (input, expected) in [
        ("", NameError::Empty),
        ("src/", NameError::Empty),
        ("src/../outside.cs", NameError::DotComponent),
        ("src/bad?.cs", NameError::InvalidCharacter),
        ("src\\file.cs", NameError::InvalidCharacter),
        ("src/last.", NameError::TrailingDotOrSpace),
        ("src/PrN.txt", NameError::ReservedDevice),
        ("src/LPT¹.json", NameError::ReservedDevice),
    ] {
        assert_eq!(entry_path(None, input, true), Err(expected), "{input}");
    }
    assert_eq!(
        entry_path(None, &"字".repeat(256), true),
        Err(NameError::ComponentTooLong)
    );
    assert_eq!(
        entry_path(None, "src/file.cs", false),
        Err(NameError::Nested)
    );
    assert_eq!(
        entry_path(Some(&path("Assets")), "src/脚本.cs", true).unwrap(),
        path("Assets/src/脚本.cs")
    );
    assert_eq!(rename_selection("脚本.cs", EntryKind::File), 0..6);
    assert_eq!(rename_selection(".gitignore", EntryKind::File), 0..10);
    assert_eq!(rename_selection("folder.name", EntryKind::Directory), 0..11);
}

#[test]
fn creates_nested_entries_and_never_replaces_existing_data() {
    let temp = tempfile::tempdir().unwrap();
    let folder = Folder::scan_root(temp.path(), WorkspaceConfig::default().without_watching())
        .expect("temporary workspace");
    let created = folder
        .create_entry(None, "Assets/Scripts/First.cs", EntryKind::File)
        .unwrap();
    assert_eq!(created.path, path("Assets/Scripts/First.cs"));
    assert_eq!(
        created.directories,
        vec![path("Assets"), path("Assets/Scripts")]
    );
    let file = folder.absolute(&created.path);
    fs::write(&file, b"original data").unwrap();
    let duplicate = folder.create_entry(Some(&path("Assets")), "Scripts/First.cs", EntryKind::File);
    assert!(matches!(
        duplicate,
        Err(OperationError::Io { source, .. }) if source.kind() == io::ErrorKind::AlreadyExists
    ));
    assert_eq!(fs::read(&file).unwrap(), b"original data");

    let subfolder = folder
        .create_entry(Some(&path("Assets")), "Shaders/New", EntryKind::Directory)
        .unwrap();
    assert!(folder.absolute(&subfolder.path).is_dir());
}

#[test]
fn readonly_workspace_rules_reject_creation_and_rename_before_disk_changes() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("Library/PackageCache/package");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("Package.cs"), b"managed by Unity").unwrap();
    let folder = Folder::scan_root(temp.path(), WorkspaceConfig::default().without_watching())
        .expect("temporary workspace");
    let parent = path("Library/PackageCache/package");
    assert!(matches!(
        folder.create_entry(Some(&parent), "New.cs", EntryKind::File),
        Err(OperationError::ReadOnly(_))
    ));
    assert!(matches!(
        folder.rename_entry(
            &path("Library/PackageCache/package/Package.cs"),
            "Changed.cs"
        ),
        Err(OperationError::ReadOnly(_))
    ));
    assert!(!cache.join("New.cs").exists());
    assert!(!cache.join("Changed.cs").exists());
    assert_eq!(
        fs::read(cache.join("Package.cs")).unwrap(),
        b"managed by Unity"
    );
}

#[test]
fn renames_without_overwrite_and_preserves_case_on_disk() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Foo.cs"), b"source data").unwrap();
    fs::write(temp.path().join("Existing.cs"), b"destination data").unwrap();
    let folder = Folder::scan_root(temp.path(), WorkspaceConfig::default().without_watching())
        .expect("temporary workspace");
    let original = path("Foo.cs");
    assert!(matches!(
        folder.rename_entry(&original, "Existing.cs"),
        Err(OperationError::Io { source, .. }) if source.kind() == io::ErrorKind::AlreadyExists
    ));
    assert_eq!(
        fs::read(temp.path().join("Existing.cs")).unwrap(),
        b"destination data"
    );
    let renamed = folder.rename_entry(&original, "foo.cs").unwrap();
    assert_eq!(renamed, path("foo.cs"));
    assert_eq!(fs::read(folder.absolute(&renamed)).unwrap(), b"source data");
    let names = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 2);
    assert!(names.iter().any(|name| name == "foo.cs"));
}

#[cfg(windows)]
#[test]
fn handles_extended_paths_and_reports_a_locked_source() {
    use std::os::windows::fs::OpenOptionsExt;

    let temp = tempfile::tempdir().unwrap();
    let folder = Folder::scan_root(temp.path(), WorkspaceConfig::default().without_watching())
        .expect("temporary workspace");
    let nested = format!("{}/{}/{}", "a".repeat(90), "b".repeat(90), "c".repeat(90));
    let created = folder
        .create_entry(None, &format!("{nested}/Busy.txt"), EntryKind::File)
        .unwrap();
    let absolute = folder.absolute(&created.path);
    assert!(absolute.as_os_str().len() > 260);
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&absolute)
        .unwrap();
    assert!(matches!(
        folder.rename_entry(&created.path, "Available.txt"),
        Err(OperationError::Io { .. })
    ));
    drop(lock);
    assert!(folder.rename_entry(&created.path, "Available.txt").is_ok());
}

#[test]
fn moves_between_directories_and_never_moves_into_a_descendant() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("Assets/Scripts")).unwrap();
    fs::create_dir(temp.path().join("Target")).unwrap();
    fs::write(temp.path().join("Assets/Scripts/Test.cs"), b"moved data").unwrap();
    let folder = Folder::scan_root(temp.path(), WorkspaceConfig::default().without_watching())
        .expect("temporary workspace");
    assert!(matches!(
        folder.move_entry(&path("Assets"), &path("Assets/Scripts/New")),
        Err(OperationError::Io { source, .. }) if source.kind() == io::ErrorKind::InvalidInput
    ));

    let from = path("Assets/Scripts");
    let to = path("Target/Scripts");
    folder.move_entry(&from, &to).unwrap();
    assert!(!folder.absolute(&from).exists());
    assert_eq!(
        fs::read(folder.absolute(&path("Target/Scripts/Test.cs"))).unwrap(),
        b"moved data"
    );
    folder.move_entry(&to, &from).unwrap();
    assert!(folder.absolute(&from).is_dir());
    let stamp = folder.entry_stamp(&from).unwrap();
    folder.remove_permanently(&from, &stamp).unwrap();
    assert!(!folder.absolute(&from).exists());
}

#[test]
fn maps_directory_descendants_by_component_and_keeps_display_case() {
    use rove_workspace::file_operations::path_after_rename;

    let old = path("Assets/Scripts");
    let new = path("Code");
    assert_eq!(
        path_after_rename(&path("Assets/Scripts/UI/Player.cs"), &old, &new),
        Some(path("Code/UI/Player.cs"))
    );
    assert_eq!(path_after_rename(&old, &old, &new), Some(new.clone()));
    assert_eq!(
        path_after_rename(&path("Assets/ScriptsBase/Other.cs"), &old, &new),
        None
    );
    #[cfg(windows)]
    assert_eq!(
        path_after_rename(&path("assets/SCRIPTS/Player.cs"), &old, &new),
        Some(path("Code/Player.cs"))
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires a Windows desktop with the Recycle Bin enabled for the test volume"]
fn shell_recycles_and_restores_the_exact_temporary_file() {
    use rove_workspace::file_operations::recycle;

    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("独立验证, recycle.txt");
    fs::write(&file, b"temporary recycle test").unwrap();
    assert!(recycle::available(&file));
    let item = recycle::recycle(&file).expect("Windows Shell moves the temporary file to its bin");
    let disappeared = !file.exists();
    recycle::restore(&item, &file).expect("Windows Shell restores the captured item");
    assert!(disappeared);
    assert_eq!(fs::read(&file).unwrap(), b"temporary recycle test");
}

#[cfg(windows)]
#[test]
fn workspace_events_track_case_only_rename_during_other_disk_changes() {
    use rove_workspace::file_operations::path_after_rename;
    use rove_workspace::{wait_until_complete, FileEvent, Workspace};
    use std::{
        sync::{mpsc, Arc},
        thread,
        time::{Duration, Instant},
    };

    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("Folder")).unwrap();
    fs::write(temp.path().join("Folder/Example.cs"), b"retain identity").unwrap();
    let folder = Folder::start(temp.path(), WorkspaceConfig::default()).unwrap();
    let (send, receive) = mpsc::channel();
    let _subscription = folder.changes().subscribe(Arc::new(move |event| {
        let _ = send.send(event);
    }));
    assert!(wait_until_complete(&folder, Duration::from_secs(12)));
    let ready = Instant::now() + Duration::from_secs(12);
    while !folder.watching() && Instant::now() < ready {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(folder.watching());

    let external_root = temp.path().to_path_buf();
    let external = thread::spawn(move || {
        for index in 0..200 {
            let file = external_root.join(format!("external-{index:03}.txt"));
            fs::write(&file, b"external").unwrap();
            if index % 2 == 0 {
                fs::remove_file(&file).unwrap();
            }
        }
    });
    folder.rename_entry(&path("Folder"), "folder").unwrap();
    external.join().unwrap();

    let mut document = path("Folder/Example.cs");
    let expected = path("folder/Example.cs");
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if let Ok(batch) = receive.recv_timeout(Duration::from_millis(50)) {
            for event in batch.events.iter() {
                if let FileEvent::Renamed { from, to } = event {
                    if let Some(mapped) = path_after_rename(&document, from, to) {
                        document = mapped;
                    }
                }
            }
        }
        let items = folder.files(0..usize::MAX).unwrap();
        if document == expected
            && items.items.len() == 101
            && items.items.iter().any(|entry| entry.path == expected)
            && items
                .items
                .iter()
                .all(|entry| folder.absolute(&entry.path).exists())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "document: {document:?}, files: {}",
            items.items.len()
        );
    }
}

#[test]
fn permanent_delete_rechecks_the_confirmed_item() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("ToDelete.txt");
    fs::write(&file, b"old").unwrap();
    let folder = Folder::scan_root(temp.path(), WorkspaceConfig::default().without_watching())
        .expect("temporary workspace");
    let selected = path("ToDelete.txt");
    let stamp = folder.entry_stamp(&selected).unwrap();
    fs::remove_file(&file).unwrap();
    fs::write(&file, b"external replacement").unwrap();
    assert!(matches!(
        folder.remove_permanently(&selected, &stamp),
        Err(OperationError::Io { source, .. }) if source.kind() == io::ErrorKind::InvalidData
    ));
    assert_eq!(fs::read(&file).unwrap(), b"external replacement");
}

#[test]
fn incremental_paste_names_preserve_extensions_and_padding() {
    use rove_workspace::file_operations::transfer::{next_duplicate_name, Naming};
    assert_eq!(
        next_duplicate_name("Game.cs", false, Naming::Simple).as_deref(),
        Some("Game copy.cs")
    );
    assert_eq!(
        next_duplicate_name("Game copy.cs", false, Naming::Simple).as_deref(),
        Some("Game copy 2.cs")
    );
    assert_eq!(
        next_duplicate_name("Game.007.cs", false, Naming::Smart).as_deref(),
        Some("Game.008.cs")
    );
    assert_eq!(
        next_duplicate_name("006-Game.cs", false, Naming::Smart).as_deref(),
        Some("007-Game.cs")
    );
    assert_eq!(
        next_duplicate_name("Scripts", true, Naming::Smart).as_deref(),
        Some("Scripts.1")
    );
    assert_eq!(
        next_duplicate_name("Game.cs", false, Naming::Disabled),
        None
    );
}

#[test]
fn stages_copies_and_moves_without_replacing_destinations() {
    use rove_workspace::file_operations::transfer::{Transfer, TransferOutcome};
    let temp = tempfile::tempdir().unwrap();
    // Pass a directory on another volume to exercise the real cross-volume
    // path with exactly the same no-overwrite, move and restore assertions.
    let external = std::env::var_os("ROVE_TEST_FILE_OPERATIONS_VOLUME")
        .map_or_else(tempfile::tempdir, tempfile::tempdir_in)
        .unwrap();
    let project = temp.path();
    let source = external.path().join("From Explorer");
    fs::create_dir_all(source.join("Nested")).unwrap();
    fs::write(source.join("Nested/Original.cs"), b"native clipboard data").unwrap();
    let folder = Folder::scan_root(project, WorkspaceConfig::default().without_watching()).unwrap();
    let copied = path("Copied");
    assert!(matches!(
        folder
            .transfer_from(&source, &copied, Transfer::Copy)
            .unwrap(),
        TransferOutcome::Copied
    ));
    assert_eq!(
        fs::read(project.join("Copied/Nested/Original.cs")).unwrap(),
        b"native clipboard data"
    );
    assert!(source.is_dir());
    assert!(matches!(
        folder.transfer_from(&source, &copied, Transfer::Move),
        Err(OperationError::Io { source, .. }) if source.kind() == io::ErrorKind::AlreadyExists
    ));
    let moved = path("Moved");
    assert!(matches!(
        folder
            .transfer_from(&source, &moved, Transfer::Move)
            .unwrap(),
        TransferOutcome::Moved
    ));
    assert!(!source.exists());
    assert!(matches!(
        folder.undo_transfer(&moved, &source).unwrap(),
        TransferOutcome::Moved
    ));
    assert_eq!(
        fs::read(source.join("Nested/Original.cs")).unwrap(),
        b"native clipboard data"
    );
}
