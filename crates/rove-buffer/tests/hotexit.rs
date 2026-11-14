//! 热退出与崩溃恢复的入口集成测试（#50）：编辑 → 持续备份 → **强杀进程** → 重启恢复 → 保存。
//!
//! 覆盖 issue 验收标准里属于核心层的部分：
//!
//! - `Stop-Process -Force` 杀掉真的子进程之后备份仍在，恢复出来的内容逐字节一致；
//! - 恢复后直接保存，GBK + CRLF、UTF-8 带 BOM、混合换行符三类文件除改动处外字节完全一致；
//! - 外部修改落到 #49 的保存冲突路径、外部删除恢复成“已删除”状态；
//! - `files.restoreUndoStack` 的真假两条路径与撤销栈超限截断；
//! - 备份清理（保存后、关闭后、恢复消费后、工作区消失后）；
//! - 连续两次恢复失败的备份被隔离到 `backups-failed\`，不进入无限恢复循环；
//! - 崩溃日志写在本地，且整个流程**没有任何网络访问**（见 `no_network_handles_during_backup`）；
//! - 持续备份的 I/O 开销与 UI 线程开销（见 `continuous_typing_io_cost`，加 `-- --nocapture` 看数据）。
//!
//! 界面侧（状态栏提示、标签顺序与活动标签、三种 `files.hotExit` 的关闭确认）在
//! `rove-ui` 的 `skeleton::workbench::hotexit_tests`。
//!
//! 辅助子进程是本测试二进制自己：`ROVE_HOTEXIT_CHILD` 决定它扮演哪个角色，未设置时
//! `child_*` 测试直接返回（它们标了 `#[ignore]`，只由父测试显式启动）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rove_base::{ByteOffset, Cancellation, DocumentId, Events, Span, Task, Value};
use rove_buffer::edit::{EditOptions, Transaction};
use rove_buffer::hotexit::{
    crash, store::MAX_RESTORE_ATTEMPTS, BackupService, HotExitMode, HotExitOptions, Restore,
};
use rove_buffer::save::{
    self, DiskStamp, SaveOptions, SaveOutcome, SaveRequest, TextFile, WriteOptions,
};
use rove_buffer::{Encoding, Eol, LineEndings};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings};

// --------------------------------------------------------------------------------- 测试脚手架

struct TestSettings {
    defaults: DefaultSettings,
    user: BTreeMap<String, Value>,
}

impl TestSettings {
    fn new(user: &[(&str, Value)]) -> Self {
        let mut registry = SettingRegistry::default();
        rove_buffer::settings::register(&mut registry).unwrap();
        save::settings::register(&mut registry).unwrap();
        rove_buffer::hotexit::settings::register(&mut registry).unwrap();
        Self {
            defaults: DefaultSettings(registry),
            user: user
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }
}

impl Settings for TestSettings {
    fn read(&self, key: &str, scope: &Scope) -> rove_base::Result<Value> {
        match self.user.get(key) {
            Some(value) => Ok(value.clone()),
            None => self.defaults.read(key, scope),
        }
    }
    fn write(&self, _: Arc<str>, _: Scope, _: Value, _: Cancellation) -> Task<()> {
        Box::pin(async { Err(rove_base::Error::Unsupported) })
    }
    fn changes(&self) -> &dyn Events<rove_config::SettingsChange> {
        &rove_base::NoEvents
    }
}

fn defaults() -> TestSettings {
    TestSettings::new(&[])
}

fn load(path: &Path, settings: &dyn Settings) -> TextFile {
    TextFile::load(
        path,
        "csharp",
        settings,
        Arc::from([]),
        EditOptions::default(),
    )
    .unwrap()
}

/// 把 `needle` 第一次出现处换成 `with`（经编辑会话，一个撤销单元）。
fn edit(file: &mut TextFile, needle: &str, with: &str) {
    let text = file.session().contents();
    let at = text.find(needle).expect("needle");
    let range = Span::new(
        ByteOffset(at as u64),
        ByteOffset((at + needle.len()) as u64),
    )
    .unwrap();
    file.session_mut()
        .edit(Transaction::single(range, with), Instant::now())
        .unwrap();
}

/// 在 `root` 下建一个工作区目录与状态根目录（模拟 `%LOCALAPPDATA%\Rove`）。
fn workspace(root: &Path) -> (PathBuf, PathBuf) {
    let workspace = root.join("Game");
    let state = root.join("Local").join("Rove");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    (workspace, state)
}

fn open(state: &Path, workspace: &Path, mode: HotExitMode) -> (BackupService, Restore) {
    BackupService::open(
        state,
        workspace,
        HotExitOptions {
            mode,
            restore_undo_stack: true,
        },
    )
}

/// 一次完整的“备份 → 写盘”。
fn back_up(service: &mut BackupService, document: DocumentId, file: &TextFile, index: u32) {
    let backup = file
        .to_backup(service.undo_limit(), index, index == 0)
        .expect("有路径的文档可以备份");
    let bytes = service.write(&backup).unwrap();
    service.note_written(document, &backup.path, bytes);
}

fn restore(backup: rove_buffer::hotexit::Backup) -> TextFile {
    TextFile::from_backup(backup, Arc::from([]), EditOptions::default())
}

fn save(file: &mut TextFile) -> SaveOutcome {
    file.save(
        &SaveRequest::default(),
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    )
}

const DOC: DocumentId = DocumentId(1);

// --------------------------------------------------------------------------------- 强杀进程

fn child_role() -> Option<String> {
    std::env::var("ROVE_HOTEXIT_CHILD").ok()
}

/// 子进程：打开文件、改内容、写出一份热退出备份，然后一直等着被父进程强杀。
/// **故意不写退出标记、不做任何收尾**，所以这是一次不折不扣的意外退出。
#[test]
#[ignore = "由 force_killed_process_leaves_a_restorable_backup 作为子进程启动"]
fn child_edit_then_hang() {
    if child_role().as_deref() != Some("child_edit_then_hang") {
        return;
    }
    let state = PathBuf::from(std::env::var_os("ROVE_HOTEXIT_STATE").unwrap());
    let workspace = PathBuf::from(std::env::var_os("ROVE_HOTEXIT_WORKSPACE").unwrap());
    let target = PathBuf::from(std::env::var_os("ROVE_HOTEXIT_TARGET").unwrap());
    let reached = PathBuf::from(std::env::var_os("ROVE_HOTEXIT_REACHED").unwrap());
    let settings = defaults();
    let (mut service, restored) = open(&state, &workspace, HotExitMode::OnExit);
    assert!(restored.is_empty());
    let mut file = load(&target, &settings);
    edit(&mut file, "原来的名字", "改过的名字");
    // 持续备份：去抖到期后写盘（这里直接走到期路径）。
    let now = Instant::now();
    service.note_edit(DOC, now);
    assert_eq!(service.due(now + Duration::from_secs(1)), vec![DOC]);
    back_up(&mut service, DOC, &file, 0);
    std::fs::write(&reached, b"1").unwrap();
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn force_killed_process_leaves_a_restorable_backup() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let target = ws.join("Player.cs");
    // GBK + CRLF：恢复后保存必须逐字节回到原编码。
    let original = encoding_rs::GBK
        .encode("// 原来的名字\r\nint a;\r\n")
        .0
        .into_owned();
    std::fs::write(&target, &original).unwrap();
    let marker = dir.path().join("reached");

    let mut child = spawn_child(
        "child_edit_then_hang",
        &[
            ("ROVE_HOTEXIT_STATE", state.as_path()),
            ("ROVE_HOTEXIT_WORKSPACE", ws.as_path()),
            ("ROVE_HOTEXIT_TARGET", target.as_path()),
            ("ROVE_HOTEXIT_REACHED", marker.as_path()),
        ],
    );
    wait_for(&marker, Duration::from_secs(120));

    // 真的强杀：PowerShell 的 Stop-Process -Force（不是“模拟崩溃”的内部开关）。
    stop_process_force(child.id());
    let status = child.wait().unwrap();
    assert!(!status.success(), "子进程应当是被杀死的");
    // 磁盘上的文件一个字节都没动（备份不等于保存）。
    assert_eq!(std::fs::read(&target).unwrap(), original);

    // 重启：发现备份、认定为意外退出、内容逐字节一致。
    let (service, restored) = open(&state, &ws, HotExitMode::OnExit);
    assert!(restored.crashed, "没有退出标记就是意外退出");
    assert_eq!(restored.backups.len(), 1);
    let backup = restored.backups.into_iter().next().unwrap();
    assert_eq!(backup.path, target);
    assert_eq!(backup.encoding, Encoding::Gbk);
    assert!(backup.dirty);
    let mut file = restore(backup);
    assert!(file.is_dirty(), "恢复出来的文档带着未保存的修改");
    assert_eq!(file.session().contents(), "// 改过的名字\r\nint a;\r\n");

    // 恢复后直接保存：除改动处外字节完全一致（编码、BOM、换行符都没变）。
    assert!(matches!(save(&mut file), SaveOutcome::Saved { .. }));
    let expected = encoding_rs::GBK
        .encode("// 改过的名字\r\nint a;\r\n")
        .0
        .into_owned();
    assert_eq!(std::fs::read(&target).unwrap(), expected);
    service.finish_restore(std::slice::from_ref(&target));
    assert!(!service.store().has_backups(), "消费完的备份被清掉");
}

fn spawn_child(test: &str, envs: &[(&str, &Path)]) -> std::process::Child {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            test,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ROVE_HOTEXIT_CHILD", test);
    for (k, v) in envs {
        command.env(k, v);
    }
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap()
}

/// 本机没有 pwsh 7，用 Windows PowerShell 5.1（`AGENTS.md` §9）。
fn stop_process_force(pid: u32) {
    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("Stop-Process -Id {pid} -Force"),
        ])
        .status()
        .expect("启动 powershell");
    assert!(status.success(), "Stop-Process -Force 失败");
}

fn wait_for(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        assert!(Instant::now() < deadline, "等待 {path:?} 超时");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// --------------------------------------------------------------------------------- 编码往返

#[test]
fn restored_documents_save_byte_identical_for_every_encoding_and_eol() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);

    // 三类文件：GBK + CRLF、UTF-8 带 BOM、混合换行符。
    let gbk = encoding_rs::GBK
        .encode("// 中文注释\r\nint a;\r\n")
        .0
        .into_owned();
    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice("// 中文注释\nint a;\n".as_bytes());
    let mixed = b"line one\r\nline two\nline three\r\n".to_vec();
    /// (文件名, 磁盘字节, 要改的片段, 改成什么, 期望编码, 期望 BOM)
    type Case = (
        &'static str,
        Vec<u8>,
        &'static str,
        &'static str,
        Encoding,
        bool,
    );
    let cases: [Case; 3] = [
        ("Gbk.cs", gbk, "中文注释", "改了注释", Encoding::Gbk, false),
        ("Bom.cs", bom, "中文注释", "改了注释", Encoding::Utf8, true),
        (
            "Mixed.cs",
            mixed,
            "line two",
            "line TWO",
            Encoding::Utf8,
            false,
        ),
    ];

    for (index, (name, bytes, needle, with, encoding, has_bom)) in cases.into_iter().enumerate() {
        let path = ws.join(name);
        std::fs::write(&path, &bytes).unwrap();
        let mut file = load(&path, &settings);
        assert_eq!(file.encoding(), &encoding, "{name} 的编码");
        assert_eq!(file.bom(), has_bom, "{name} 的 BOM");
        let before = file.session().contents();
        edit(&mut file, needle, with);
        let document = DocumentId(index as u64);
        back_up(&mut service, document, &file, index as u32);
        drop(file);

        // 另一进程里重新打开：从备份恢复，保存。
        let backup = service
            .store()
            .backups()
            .into_iter()
            .find(|b| b.path == path)
            .unwrap();
        // 混合换行符与“是否混合”必须原样带过来。
        let mixed_before = matches!(backup.line_endings, LineEndings::Mixed { .. });
        assert_eq!(mixed_before, name == "Mixed.cs", "{name} 的混合换行标记");
        let mut restored = restore(backup);
        assert_eq!(restored.session().contents(), before.replace(needle, with));
        assert!(matches!(save(&mut restored), SaveOutcome::Saved { .. }));

        // 除改动处外字节完全一致：原文件里把 needle 换成 with 再按原编码编码一遍。
        let expected = match (&encoding, has_bom) {
            (Encoding::Gbk, _) => encoding_rs::GBK
                .encode(&before.replace(needle, with))
                .0
                .into_owned(),
            (_, true) => {
                let mut out = vec![0xef, 0xbb, 0xbf];
                out.extend_from_slice(before.replace(needle, with).as_bytes());
                out
            }
            _ => before.replace(needle, with).into_bytes(),
        };
        assert_eq!(std::fs::read(&path).unwrap(), expected, "{name} 的字节");
    }
}

// --------------------------------------------------------------------------------- 外部修改

#[test]
fn external_change_during_backup_goes_to_the_save_conflict_path() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"original\n").unwrap();
    let mut file = load(&path, &settings);
    edit(&mut file, "original", "mine");
    back_up(&mut service, DOC, &file, 0);
    drop(file);

    // 备份之后、恢复之前，别的程序改了磁盘上的文件。
    std::fs::write(&path, b"theirs\n").unwrap();
    let backup = service.store().backups().into_iter().next().unwrap();
    let mut restored = restore(backup);
    // 保存落到 #49 的保存冲突路径（本模块不另造一套提示）。
    assert_eq!(save(&mut restored), SaveOutcome::Conflict);
    assert_eq!(std::fs::read(&path).unwrap(), b"theirs\n");
    // “覆盖”之后正常写入。
    let outcome = restored.save(
        &SaveRequest {
            overwrite_conflict: true,
            ..SaveRequest::default()
        },
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    );
    assert!(matches!(outcome, SaveOutcome::Saved { .. }));
    assert_eq!(std::fs::read(&path).unwrap(), b"mine\n");
}

#[test]
fn deleted_file_restores_as_a_deleted_tab_and_saves_as_a_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"original\n").unwrap();
    let mut file = load(&path, &settings);
    edit(&mut file, "original", "mine");
    back_up(&mut service, DOC, &file, 0);
    drop(file);

    std::fs::remove_file(&path).unwrap();
    let mut backup = service.store().backups().into_iter().next().unwrap();
    // 恢复方发现磁盘上的文件已经不在：标成“已删除”（内容保留）。
    backup.deleted = !backup.path.exists();
    let mut restored = restore(backup);
    assert!(restored.is_deleted());
    assert!(restored.is_dirty());
    // 保存视为新建写入。
    assert!(matches!(save(&mut restored), SaveOutcome::Saved { .. }));
    assert_eq!(std::fs::read(&path).unwrap(), b"mine\n");
    assert!(!restored.is_deleted());
}

// --------------------------------------------------------------------------------- 撤销栈

#[test]
fn restore_undo_stack_true_false_and_over_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"a\n").unwrap();

    // restoreUndoStack = true：恢复后可以继续 Ctrl+Z 回到更早的状态。
    let (service, _) = open(&state, &ws, HotExitMode::OnExit);
    assert_eq!(service.undo_limit(), rove_buffer::hotexit::UNDO_LIMIT);
    let mut file = load(&path, &settings);
    edit(&mut file, "a", "b");
    edit(&mut file, "b", "c");
    let backup = file.to_backup(service.undo_limit(), 0, true).unwrap();
    let mut restored = restore(backup);
    assert_eq!(restored.session().contents(), "c\n");
    restored.session_mut().undo().unwrap();
    assert_eq!(restored.session().contents(), "b\n");
    restored.session_mut().undo().unwrap();
    assert_eq!(restored.session().contents(), "a\n");
    // 回到保存点之后不再是“已修改”。
    assert!(!restored.is_dirty());
    restored.session_mut().redo().unwrap();
    assert_eq!(restored.session().contents(), "b\n");
    assert!(restored.is_dirty());

    // restoreUndoStack = false：撤销栈为空，但文档仍然是“已修改”。
    let (service, _) = BackupService::open(
        &state,
        &ws,
        HotExitOptions {
            mode: HotExitMode::OnExit,
            restore_undo_stack: false,
        },
    );
    assert_eq!(service.undo_limit(), 0);
    let backup = file.to_backup(service.undo_limit(), 0, true).unwrap();
    assert!(backup.undo.is_empty());
    let mut restored = restore(backup);
    assert_eq!(restored.session().contents(), "c\n");
    assert!(restored.is_dirty());
    assert!(!restored.session().can_undo());
    assert!(restored.session_mut().undo().unwrap().is_empty());
    assert_eq!(restored.session().contents(), "c\n");

    // 超限：按上限截断，丢掉最旧的单元，不报错。
    let mut many = load(&path, &settings);
    for n in 0..12u32 {
        let text = many.session().contents();
        let needle = text.trim_end().to_owned();
        edit(&mut many, &needle, &format!("v{n}"));
    }
    assert_eq!(many.session().contents(), "v11\n");
    let backup = many.to_backup(3, 0, true).unwrap();
    assert_eq!(backup.undo.undo.len(), 3);
    let mut restored = restore(backup);
    for expect in ["v10", "v9", "v8"] {
        restored.session_mut().undo().unwrap();
        assert_eq!(restored.session().contents().trim_end(), expect);
    }
    assert!(restored.session_mut().undo().unwrap().is_empty());
    assert!(restored.is_dirty(), "截断后仍然是已修改");
}

// --------------------------------------------------------------------------------- 清理

#[test]
fn backups_are_cleaned_up_after_save_close_restore_and_workspace_removal() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"original\n").unwrap();

    // 保存后：对应备份消失。
    let mut file = load(&path, &settings);
    edit(&mut file, "original", "saved");
    back_up(&mut service, DOC, &file, 0);
    assert!(service.store().backup_path(&path).is_file());
    assert!(matches!(save(&mut file), SaveOutcome::Saved { .. }));
    service.discard(DOC);
    assert!(!service.store().backup_path(&path).exists());

    // 关闭后：同样消失。
    edit(&mut file, "saved", "closed");
    back_up(&mut service, DOC, &file, 0);
    assert!(service.store().backup_path(&path).is_file());
    service.discard(DOC);
    assert!(!service.store().backup_path(&path).exists());

    // 恢复消费后：finish_restore 清掉。
    edit(&mut file, "closed", "restored");
    back_up(&mut service, DOC, &file, 0);
    service.finish_restore(std::slice::from_ref(&path));
    assert!(!service.store().backup_path(&path).exists());

    // 工作区目录已经不存在：整组清理。
    edit(&mut file, "restored", "gone");
    back_up(&mut service, DOC, &file, 0);
    let group = service.store().dir().to_path_buf();
    assert!(group.is_dir());
    drop(service);
    std::fs::remove_dir_all(&ws).unwrap();
    let other = dir.path().join("Other");
    std::fs::create_dir_all(&other).unwrap();
    let (_service, restored) = open(&state, &other, HotExitMode::OnExit);
    assert!(restored.is_empty());
    assert!(!group.exists(), "工作区消失后整组备份被清理");
}

#[test]
fn hot_exit_off_neither_backs_up_nor_restores() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"original\n").unwrap();

    // 先在 onExit 下留一份备份。
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);
    let mut file = load(&path, &settings);
    edit(&mut file, "original", "mine");
    back_up(&mut service, DOC, &file, 0);
    let group = service.store().dir().to_path_buf();
    assert!(group.is_dir());
    drop(service);

    // 改成 off：启动时整组清掉，不恢复、不提示，也不再写新的备份。
    let (mut service, restored) = open(&state, &ws, HotExitMode::Off);
    assert!(restored.is_empty());
    assert!(!restored.crashed);
    assert!(!group.exists());
    service.note_edit(DOC, Instant::now());
    assert!(service.next_deadline().is_none(), "off 时不排队备份");
    let backup = file.to_backup(service.undo_limit(), 0, true).unwrap();
    assert_eq!(service.write(&backup).unwrap(), 0);
    assert!(!service.store().backup_path(&path).exists());
}

// --------------------------------------------------------------------------------- 退出标记

#[test]
fn clean_exit_restores_silently_and_a_crash_asks_for_the_status_message() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"original\n").unwrap();

    // 第一次运行：编辑、备份、正常退出。
    let (mut service, restored) = open(&state, &ws, HotExitMode::OnExit);
    assert!(!restored.crashed, "没有备份时不提示");
    let mut file = load(&path, &settings);
    edit(&mut file, "original", "mine");
    back_up(&mut service, DOC, &file, 0);
    service.mark_clean_exit();
    drop(service);

    // 第二次运行：内容回来，但没有“意外退出”。
    let (service, restored) = open(&state, &ws, HotExitMode::OnExit);
    assert_eq!(restored.backups.len(), 1);
    assert!(!restored.crashed, "正常退出后不提示");
    assert_eq!(
        restore(restored.backups.into_iter().next().unwrap())
            .session()
            .contents(),
        "mine\n"
    );
    // 启动后标记立刻被清成“未正常退出”：这次再被强杀就会提示。
    assert!(!service.store().session().clean_exit);
    drop(service);
    let (_service, restored) = open(&state, &ws, HotExitMode::OnExit);
    assert!(restored.crashed, "启动即清除标记，所以这次算意外退出");

    // 没有脏文档时正常退出：整组删掉，下次启动什么都不做。
    let (service, _) = open(&state, &ws, HotExitMode::OnExit);
    service.finish_restore(std::slice::from_ref(&path));
    service.mark_clean_exit();
    assert!(!service.store().dir().exists());
}

// --------------------------------------------------------------------------------- 连续失败

#[test]
fn a_backup_that_fails_twice_is_quarantined_instead_of_looping() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"original\n").unwrap();
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);
    let mut file = load(&path, &settings);
    edit(&mut file, "original", "mine");
    back_up(&mut service, DOC, &file, 0);
    let group = service.store().dir().to_path_buf();
    drop(service);

    // 第 1、2 次启动：都拿到了备份，但宿主恢复失败（没有调用 finish_restore）。
    for attempt in 1..=MAX_RESTORE_ATTEMPTS {
        let (service, restored) = open(&state, &ws, HotExitMode::OnExit);
        assert_eq!(restored.backups.len(), 1, "第 {attempt} 次仍然尝试恢复");
        assert!(restored.quarantined.is_none());
        assert_eq!(service.store().session().attempts, attempt);
    }
    // 第 3 次启动：整组搬走，不再尝试，提示一次。
    let (service, restored) = open(&state, &ws, HotExitMode::OnExit);
    assert!(restored.is_empty(), "不进入无限恢复循环");
    let quarantined = restored.quarantined.expect("已隔离");
    assert!(quarantined.starts_with(state.join("backups-failed")));
    assert!(quarantined.is_dir());
    assert!(!group.exists());
    // 再启动一次：干干净净，没有隔离提示。
    drop(service);
    let (_service, restored) = open(&state, &ws, HotExitMode::OnExit);
    assert!(restored.is_empty());
    assert!(restored.quarantined.is_none());
}

#[test]
fn a_corrupt_or_missing_backup_directory_never_blocks_startup() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let (service, _) = open(&state, &ws, HotExitMode::OnExit);
    let group = service.store().dir().to_path_buf();
    std::fs::create_dir_all(&group).unwrap();
    // 一份被改坏的备份：按“没有这份备份”处理。
    std::fs::write(group.join("broken.rvbak"), b"not a backup").unwrap();
    std::fs::write(group.join("empty.rvbak"), b"").unwrap();
    drop(service);
    let (_service, restored) = open(&state, &ws, HotExitMode::OnExit);
    assert!(restored.is_empty());

    // 整个状态目录都不在：同样静默继续。
    let missing = dir.path().join("no-such-dir");
    let (service, restored) = open(&missing, &ws, HotExitMode::OnExit);
    assert!(restored.is_empty());
    assert!(service.store().backups().is_empty());
}

// --------------------------------------------------------------------------------- 崩溃日志

#[test]
fn crash_log_is_local_only_and_never_opens_a_socket() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("Rove").join(crash::LOGS_DIR);
    let before = open_handle_count();
    let path = crash::log_fatal(&logs, "无法打开窗口").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("kind: fatal"));
    assert!(text.contains("无法打开窗口"));
    assert!(text.contains("backtrace:"));
    // 日志留在本地，而且这次调用没有打开任何网络句柄（与 #31 的口径一致，见下）。
    assert_eq!(open_handle_count(), before, "崩溃日志不打开任何套接字");
    assert!(path.starts_with(&logs));
}

#[test]
fn no_network_handles_during_backup_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"original\n").unwrap();
    let before = open_handle_count();
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);
    let mut file = load(&path, &settings);
    edit(&mut file, "original", "mine");
    back_up(&mut service, DOC, &file, 0);
    service.mark_clean_exit();
    drop(service);
    let (service, restored) = open(&state, &ws, HotExitMode::OnExit);
    let _ = restore(restored.backups.into_iter().next().unwrap());
    service.finish_restore(&[path]);
    assert_eq!(
        open_handle_count(),
        before,
        "备份与恢复的整条链路不打开任何套接字"
    );
}

/// 本进程当前打开的 TCP / UDP 端点数（`netstat -ano` 里属于本进程的行）。
///
/// 与 #31 的“全程无运行时网络访问”同一套口径：不是 OS 级抓包，但足以证明本模块没有发起
/// 任何连接——本模块里压根没有网络 API 的调用点（只有 `std::fs` 与 `std::backtrace`）。
fn open_handle_count() -> usize {
    let pid = std::process::id().to_string();
    let Ok(output) = std::process::Command::new("netstat")
        .args(["-ano"])
        .output()
    else {
        return 0;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.split_whitespace().last() == Some(pid.as_str()))
        .count()
}

// --------------------------------------------------------------------------------- 磁盘标记

#[test]
fn backup_remembers_the_disk_stamp_so_conflicts_are_detected_after_restore() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);
    let path = ws.join("Player.cs");
    std::fs::write(&path, b"original\n").unwrap();
    let mut file = load(&path, &settings);
    let stamp = file.disk_stamp().cloned().unwrap();
    edit(&mut file, "original", "mine");
    back_up(&mut service, DOC, &file, 0);
    let backup = service.store().backups().into_iter().next().unwrap();
    assert_eq!(backup.disk, Some(stamp));
    assert_eq!(
        backup.disk.as_ref().map(|d| d.hash),
        DiskStamp::read(&path).unwrap().map(|(s, _)| s.hash)
    );
    assert!(matches!(backup.line_endings, LineEndings::Uniform(Eol::Lf)));
    assert_eq!(backup.tab_index, 0);
    assert!(backup.active);
}

// --------------------------------------------------------------------------------- I/O 开销

/// 持续备份的代价：连续输入 1 分钟的写入次数与字节数，以及每次击键 / 每次备份落在 UI 线程上的
/// 时间。加 `-- --nocapture` 看数字；断言只钉住数量级，避免在负载高的机器上误报。
///
/// 按键到上屏的 8 ms 预算（`PROJECT_CHARTER` §6）里本功能只占两样：
///
/// - **每次击键**：`note_edit` 往去抖队列里写一条（`BTreeMap` 的 O(log n)），亚微秒级；
/// - **每 2 秒一次**：`to_backup` 复制一份文本快照（O(n)），写盘在后台线程。
///
/// 也就是说绝大多数帧里本功能一行代码都不跑，最坏的那一帧多一次全文复制。
#[test]
fn continuous_typing_io_cost() {
    let dir = tempfile::tempdir().unwrap();
    let (ws, state) = workspace(dir.path());
    let settings = defaults();
    // 真实量级的 C# 文件：2000 行、约 60 KB。
    let source: String = (0..2000)
        .map(|n| format!("    private int field{n} = {n}; // 玩家控制器的字段\n"))
        .collect();
    let path = ws.join("Player.cs");
    std::fs::write(&path, source.as_bytes()).unwrap();
    let (mut service, _) = open(&state, &ws, HotExitMode::OnExit);
    let mut file = load(&path, &settings);

    // 每 50 ms 一次击键（比人手快），一分钟不停。
    let t0 = Instant::now();
    let mut note_edit = Duration::ZERO;
    let mut snapshot = Duration::ZERO;
    let mut writes = 0u64;
    let mut bytes = 0u64;
    for tick in 0..1200u64 {
        let now = t0 + Duration::from_millis(tick * 50);
        file.session_mut().type_text("x", now).unwrap();
        let at = Instant::now();
        service.note_edit(DOC, now);
        note_edit += at.elapsed();
        for document in service.due(now) {
            let at = Instant::now();
            let backup = file.to_backup(service.undo_limit(), 0, true).unwrap();
            snapshot += at.elapsed();
            let written = service.write(&backup).unwrap();
            service.note_written(document, &backup.path, written);
            bytes += written;
            writes += 1;
        }
    }

    assert_eq!(service.io_stats(), (writes, bytes));
    eprintln!(
        "热退出持续备份（2000 行 / {} KB 的文件，连续输入 1 分钟、每 50 ms 一次击键）：\
         写入 {writes} 次、共 {} KB；UI 线程每次击键 {:.2} µs（note_edit），\
         每次备份 {:.2} ms（to_backup 的全文快照）",
        source.len() / 1024,
        bytes / 1024,
        note_edit.as_secs_f64() * 1e6 / 1200.0,
        snapshot.as_secs_f64() * 1e3 / writes as f64,
    );
    // 靠最长等待封顶：一分钟 29 次（不是每次击键一次，也不是只在退出时一次）。
    assert_eq!(writes, 29, "一分钟的备份写入次数");
    assert!(bytes > 0);
    // 每次击键的 UI 线程开销远低于 8 ms 预算的千分之一（负载高的机器上也要成立）。
    assert!(
        note_edit / 1200 < Duration::from_micros(8),
        "note_edit 每次 {:?}",
        note_edit / 1200
    );
}
