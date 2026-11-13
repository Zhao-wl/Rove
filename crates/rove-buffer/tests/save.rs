//! 保存与外部修改的入口集成测试（#49）：打开 → 编辑 → 保存 → 外部修改 → 重载。
//!
//! 覆盖 issue 的验收标准中属于核心层的部分：编码往返字节级零差异（GBK + CRLF、UTF-8 BOM、
//! UTF-16 LE/BE、混合换行）、换行符统一可一次撤销、无法编码字符的三条路径、原子写入（故障注入
//! 与强杀子进程）、占用重试（持有独占句柄的辅助进程）、只读属性、保存冲突、外部修改（静默重载 /
//! 提示 / 删除）、外部修改风暴、连续保存零误报、保存时变换与用户改动是同一个撤销单元、未命名文档。
//!
//! 辅助子进程是本测试二进制自己：`ROVE_SAVE_CHILD` 环境变量决定它扮演哪个角色，未设置时
//! `child_*` 测试直接返回（它们标了 `#[ignore]`，只由父测试显式启动）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rove_base::{ByteOffset, Cancellation, Events, Line, Task, Value};
use rove_buffer::edit::{EditOptions, Selection, Selections};
use rove_buffer::save::{
    self, ChangeCoalescer, ConflictResolution, DiskChange, DiskStamp, SaveOptions, SaveOutcome,
    SaveRequest, SelfWrites, TextFile, UnencodableChoice, WriteOptions,
};
use rove_buffer::{Encoding, Eol, LineEndings, TextSource};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings};

/// VS Code 默认值 + 测试里的用户值。
struct TestSettings {
    defaults: DefaultSettings,
    user: BTreeMap<String, Value>,
}

impl TestSettings {
    fn new(user: &[(&str, Value)]) -> Self {
        let mut registry = SettingRegistry::default();
        rove_buffer::settings::register(&mut registry).unwrap();
        save::settings::register(&mut registry).unwrap();
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

fn contents(file: &TextFile) -> String {
    file.session().contents()
}

/// 把 `needle` 第一次出现处换成 `with`（经编辑会话，一个撤销单元）。
fn replace_first(file: &mut TextFile, needle: &str, with: &str, now: Instant) {
    let text = contents(file);
    let at = text.find(needle).expect("needle") as u64;
    let session = file.session_mut();
    session.set_selections(Selections::single(Selection {
        anchor: ByteOffset(at),
        active: ByteOffset(at + needle.len() as u64),
        ..Selection::cursor(ByteOffset(at))
    }));
    session.type_text(with, now).unwrap();
}

fn type_at(file: &mut TextFile, at: u64, text: &str, now: Instant) {
    let session = file.session_mut();
    session.set_selections(Selections::single(Selection::cursor(ByteOffset(at))));
    session.type_text(text, now).unwrap();
}

fn save_default(file: &mut TextFile) -> SaveOutcome {
    file.save(
        &SaveRequest::default(),
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    )
}

fn encode_text(encoding: &Encoding, bom: bool, text: &str) -> Vec<u8> {
    let rope = rove_buffer::RopeText::from_text(text);
    save::encode(&rope, encoding, bom, save::Unencodable::Reject).unwrap()
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

fn tmp_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// 编码往返：五类文件各编辑一处后保存，除改动处外字节完全一致。

#[test]
fn encoding_round_trip_is_byte_identical_except_the_edit() {
    let settings = defaults();
    let cases: &[(&str, Encoding, bool, &str)] = &[
        (
            "gbk_crlf.cs",
            Encoding::Gbk,
            false,
            "// 中文注释：玩家控制器\r\nclass 玩家 {\r\n    int 生命值 = 100;\r\n}\r\n",
        ),
        (
            "utf8_bom.cs",
            Encoding::Utf8,
            true,
            "// 带 BOM 的 UTF-8\nclass Player {\n    int hp = 100;\n}\n",
        ),
        (
            "utf16le.cs",
            Encoding::Utf16Le,
            true,
            "// UTF-16 LE 😀\r\nclass Player {\r\n    int hp = 100;\r\n}\r\n",
        ),
        (
            "utf16be.cs",
            Encoding::Utf16Be,
            true,
            "// UTF-16 BE\nclass Player {\n    int hp = 100;\n}",
        ),
        (
            "mixed.cs",
            Encoding::Utf8,
            false,
            "line1\r\nline2\nint hp = 100;\rline4\r\nline5\n",
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (name, encoding, bom, text) in cases {
        let path = dir.path().join(name);
        let original = encode_text(encoding, *bom, text);
        std::fs::write(&path, &original).unwrap();
        let mut file = load(&path, &settings);
        assert_eq!(file.encoding(), encoding, "{name}");
        assert_eq!(file.bom(), *bom, "{name}");
        assert!(!file.is_dirty());
        // 无改动保存：什么都不写。
        assert_eq!(save_default(&mut file), SaveOutcome::NotDirty);

        let needle = if text.contains("生命值") {
            "生命值"
        } else {
            "hp"
        };
        replace_first(&mut file, needle, "health", Instant::now());
        assert!(file.is_dirty());
        assert!(matches!(save_default(&mut file), SaveOutcome::Saved { .. }));
        assert!(!file.is_dirty());

        let saved = std::fs::read(&path).unwrap();
        let expected_text = text.replacen(needle, "health", 1);
        assert_eq!(saved, encode_text(encoding, *bom, &expected_text), "{name}");
        // 只有改动处不同：公共前缀 + 公共后缀覆盖了其余全部字节。
        let changed_new = encode_text(encoding, false, "health").len();
        let changed_old = encode_text(encoding, false, needle).len();
        let prefix = common_prefix(&original, &saved);
        let suffix = common_suffix(&original, &saved);
        assert!(
            prefix + suffix >= saved.len() - changed_new
                && prefix + suffix >= original.len() - changed_old,
            "{name}: 前缀 {prefix} 后缀 {suffix}"
        );
        assert!(tmp_files(dir.path()).is_empty());
    }
}

#[test]
fn mixed_line_endings_stay_mixed_and_change_eol_is_one_undo() {
    let settings = defaults();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.txt");
    let text = "a\r\nb\nc\rd\r\n";
    std::fs::write(&path, text).unwrap();
    let mut file = load(&path, &settings);
    assert!(matches!(file.line_endings(), LineEndings::Mixed { .. }));
    replace_first(&mut file, "b", "B", Instant::now());
    assert!(matches!(save_default(&mut file), SaveOutcome::Saved { .. }));
    assert_eq!(std::fs::read(&path).unwrap(), b"a\r\nB\nc\rd\r\n");
    assert!(matches!(file.line_endings(), LineEndings::Mixed { .. }));

    // 统一为 LF：全文一致，一次撤销复原。
    file.change_eol(Eol::Lf, Instant::now()).unwrap();
    assert_eq!(contents(&file), "a\nB\nc\nd\n");
    assert!(matches!(file.line_endings(), LineEndings::Uniform(Eol::Lf)));
    assert!(matches!(save_default(&mut file), SaveOutcome::Saved { .. }));
    assert_eq!(std::fs::read(&path).unwrap(), b"a\nB\nc\nd\n");
    file.session_mut().undo().unwrap();
    assert_eq!(contents(&file), "a\r\nB\nc\rd\r\n");
    assert!(matches!(file.line_endings(), LineEndings::Mixed { .. }));
    // 撤销回到了上一次保存之前的内容：未保存。
    assert!(file.is_dirty());
    // 统一为 CRLF。
    file.change_eol(Eol::CrLf, Instant::now()).unwrap();
    assert_eq!(contents(&file), "a\r\nB\r\nc\r\nd\r\n");
}

#[test]
fn undo_back_to_the_save_point_is_clean() {
    let settings = defaults();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.cs");
    std::fs::write(&path, "abc").unwrap();
    let mut file = load(&path, &settings);
    type_at(&mut file, 3, "d", Instant::now());
    assert!(file.is_dirty());
    file.session_mut().undo().unwrap();
    assert!(!file.is_dirty());
    file.session_mut().redo().unwrap();
    assert!(file.is_dirty());
    assert!(matches!(save_default(&mut file), SaveOutcome::Saved { .. }));
    type_at(&mut file, 4, "e", Instant::now());
    file.session_mut().undo().unwrap();
    assert!(!file.is_dirty(), "回到保存点");
    file.session_mut().undo().unwrap();
    assert!(file.is_dirty(), "越过保存点");
}

// ---------------------------------------------------------------------------------------------
// 无法编码的字符。

fn gbk_file_with_emoji(dir: &Path) -> (PathBuf, TextFile, Vec<u8>) {
    let path = dir.join("gbk.cs");
    let original = encode_text(
        &Encoding::Gbk,
        false,
        "// 玩家控制器，中文注释\r\nint a;\r\n",
    );
    std::fs::write(&path, &original).unwrap();
    let settings = defaults();
    let mut file = load(&path, &settings);
    assert_eq!(file.encoding(), &Encoding::Gbk);
    let end = contents(&file).len() as u64;
    type_at(&mut file, end, "😀😀", Instant::now());
    (path, file, original)
}

#[test]
fn unencodable_characters_ask_first() {
    let dir = tempfile::tempdir().unwrap();
    let (path, mut file, original) = gbk_file_with_emoji(dir.path());
    // 保存前询问；“取消”就是不再保存：文件不变、仍未保存。
    assert_eq!(
        save_default(&mut file),
        SaveOutcome::NeedsEncodingDecision {
            count: 2,
            encoding: Encoding::Gbk
        }
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(file.is_dirty());
    assert!(tmp_files(dir.path()).is_empty());
}

#[test]
fn unencodable_characters_replaced_with_question_marks() {
    let dir = tempfile::tempdir().unwrap();
    let (path, mut file, original) = gbk_file_with_emoji(dir.path());
    let request = SaveRequest {
        unencodable: UnencodableChoice::Replace,
        ..SaveRequest::default()
    };
    let outcome = file.save(
        &request,
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    );
    assert!(matches!(
        outcome,
        SaveOutcome::Saved {
            encoding_changed: false,
            ..
        }
    ));
    let mut expected = original;
    expected.extend_from_slice(b"??");
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    assert_eq!(file.encoding(), &Encoding::Gbk);
}

#[test]
fn unencodable_characters_switch_to_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let (path, mut file, _) = gbk_file_with_emoji(dir.path());
    let request = SaveRequest {
        encoding: Some((Encoding::Utf8, false)),
        ..SaveRequest::default()
    };
    let outcome = file.save(
        &request,
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    );
    assert!(matches!(
        outcome,
        SaveOutcome::Saved {
            encoding_changed: true,
            ..
        }
    ));
    // 文档的编码随之改变（状态栏读它）。
    assert_eq!(file.encoding(), &Encoding::Utf8);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        "// 玩家控制器，中文注释\r\nint a;\r\n😀😀".as_bytes()
    );
    // 再次打开识别为 UTF-8。
    let reopened = load(&path, &defaults());
    assert_eq!(reopened.encoding(), &Encoding::Utf8);
}

// ---------------------------------------------------------------------------------------------
// 原子写入：强杀写入中的进程。

fn child_role() -> Option<String> {
    std::env::var("ROVE_SAVE_CHILD").ok()
}

fn spawn_child(
    test: &str,
    envs: &[(&str, &Path)],
    extra: &[(&str, String)],
) -> std::process::Child {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            test,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ROVE_SAVE_CHILD", test);
    for (k, v) in envs {
        command.env(k, v);
    }
    for (k, v) in extra {
        command.env(k, v);
    }
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        assert!(Instant::now() < deadline, "等待 {path:?} 超时");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 子进程：写入到一半（临时文件已写满、还没替换）时停住，等父进程强杀。
#[test]
#[ignore = "由 killed_writer_leaves_original_intact 作为子进程启动"]
fn child_kill_during_write() {
    if child_role().as_deref() != Some("child_kill_during_write") {
        return;
    }
    let target = PathBuf::from(std::env::var_os("ROVE_SAVE_TARGET").unwrap());
    let reached = PathBuf::from(std::env::var_os("ROVE_SAVE_REACHED").unwrap());
    let options = WriteOptions {
        fail_point: Some(save::FailPoint(Arc::new(move |_| {
            std::fs::write(&reached, b"1").unwrap();
            std::thread::sleep(Duration::from_secs(60));
            Ok(())
        }))),
        ..WriteOptions::default()
    };
    let big = vec![b'x'; 4 << 20];
    let _ = save::write_atomic(&target, &big, &options);
}

#[test]
fn killed_writer_leaves_original_intact_and_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Player.cs");
    std::fs::write(&target, b"original content").unwrap();
    let marker_dir = tempfile::tempdir().unwrap();
    let reached = marker_dir.path().join("reached");
    let mut child = spawn_child(
        "child_kill_during_write",
        &[
            ("ROVE_SAVE_TARGET", &target),
            ("ROVE_SAVE_REACHED", &reached),
        ],
        &[],
    );
    wait_for(&reached, Duration::from_secs(60));
    // 临时文件确实已经写出。
    assert_eq!(tmp_files(dir.path()).len(), 1);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"original content");
    // 系统按“关闭即删除”清掉了临时文件（删除是异步完成的，稍等）。
    let deadline = Instant::now() + Duration::from_secs(5);
    while !tmp_files(dir.path()).is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        tmp_files(dir.path()).is_empty(),
        "{:?}",
        tmp_files(dir.path())
    );
}

// ---------------------------------------------------------------------------------------------
// 占用：持有独占句柄的辅助进程。

/// 子进程：以不共享的方式打开目标，写出标记，保持 `ROVE_SAVE_HOLD_MS` 毫秒后退出。
#[test]
#[ignore = "由 busy_file_retries_once_then_reports 作为子进程启动"]
fn child_hold_exclusive() {
    if child_role().as_deref() != Some("child_hold_exclusive") {
        return;
    }
    let target = PathBuf::from(std::env::var_os("ROVE_SAVE_TARGET").unwrap());
    let reached = PathBuf::from(std::env::var_os("ROVE_SAVE_REACHED").unwrap());
    let hold: u64 = std::env::var("ROVE_SAVE_HOLD_MS").unwrap().parse().unwrap();
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options.open(&target).unwrap();
    std::fs::write(&reached, b"1").unwrap();
    std::thread::sleep(Duration::from_millis(hold));
    drop(file);
}

#[cfg(windows)]
#[test]
fn busy_file_retries_once_then_reports_and_retry_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Player.cs");
    std::fs::write(&target, b"abc").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    type_at(&mut file, 3, "d", Instant::now());

    let marker = tempfile::tempdir().unwrap();
    let reached = marker.path().join("locked");
    let mut child = spawn_child(
        "child_hold_exclusive",
        &[
            ("ROVE_SAVE_TARGET", &target),
            ("ROVE_SAVE_REACHED", &reached),
        ],
        &[("ROVE_SAVE_HOLD_MS", "4000".into())],
    );
    wait_for(&reached, Duration::from_secs(60));
    let write = WriteOptions {
        busy_retry_delay: Duration::from_millis(200),
        ..WriteOptions::default()
    };
    let started = Instant::now();
    let outcome = file.save(
        &SaveRequest::default(),
        &SaveOptions::default(),
        None,
        &write,
        Instant::now(),
    );
    // 先自动重试一次（共两次尝试、中间等了重试间隔），仍失败才报告。
    assert_eq!(outcome, SaveOutcome::Busy { attempts: 2 });
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert!(file.is_dirty());
    assert!(tmp_files(dir.path()).is_empty());
    child.wait().unwrap();
    // 通知里的“重试”：占用解除后再次保存成功。
    assert!(matches!(
        file.save(
            &SaveRequest::default(),
            &SaveOptions::default(),
            None,
            &write,
            Instant::now()
        ),
        SaveOutcome::Saved { attempts: 1, .. }
    ));
    assert_eq!(std::fs::read(&target).unwrap(), b"abcd");
}

#[cfg(windows)]
#[test]
fn busy_file_released_before_the_automatic_retry_saves_silently() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Player.cs");
    std::fs::write(&target, b"abc").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    type_at(&mut file, 3, "d", Instant::now());
    let marker = tempfile::tempdir().unwrap();
    let reached = marker.path().join("locked");
    let mut child = spawn_child(
        "child_hold_exclusive",
        &[
            ("ROVE_SAVE_TARGET", &target),
            ("ROVE_SAVE_REACHED", &reached),
        ],
        &[("ROVE_SAVE_HOLD_MS", "300".into())],
    );
    wait_for(&reached, Duration::from_secs(60));
    let write = WriteOptions {
        busy_retry_delay: Duration::from_millis(1500),
        ..WriteOptions::default()
    };
    let outcome = file.save(
        &SaveRequest::default(),
        &SaveOptions::default(),
        None,
        &write,
        Instant::now(),
    );
    child.wait().unwrap();
    assert!(
        matches!(outcome, SaveOutcome::Saved { attempts: 2, .. }),
        "{outcome:?}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"abcd");
}

// ---------------------------------------------------------------------------------------------
// 只读属性与保存冲突。

#[test]
fn read_only_attribute_then_overwrite_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Player.cs.meta");
    std::fs::write(&target, b"guid: 1").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    let mut permissions = std::fs::metadata(&target).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&target, permissions).unwrap();
    type_at(&mut file, 7, "2", Instant::now());
    assert_eq!(save_default(&mut file), SaveOutcome::ReadOnly);
    assert_eq!(std::fs::read(&target).unwrap(), b"guid: 1");
    let request = SaveRequest {
        clear_read_only: true,
        ..SaveRequest::default()
    };
    assert!(matches!(
        file.save(
            &request,
            &SaveOptions::default(),
            None,
            &WriteOptions::default(),
            Instant::now()
        ),
        SaveOutcome::Saved { .. }
    ));
    assert_eq!(std::fs::read(&target).unwrap(), b"guid: 12");
    assert!(!std::fs::metadata(&target).unwrap().permissions().readonly());
}

fn conflicted(dir: &Path) -> (PathBuf, TextFile) {
    let target = dir.join("Player.cs");
    std::fs::write(&target, b"base").unwrap();
    let mut file = load(&target, &defaults());
    type_at(&mut file, 4, " local", Instant::now());
    // 外部修改（长度也变了，修改时间精度低的文件系统上同样能识别）。
    std::fs::write(&target, b"base external").unwrap();
    (target, file)
}

#[test]
fn save_conflict_is_reported_and_overwrite_wins() {
    let dir = tempfile::tempdir().unwrap();
    let (target, mut file) = conflicted(dir.path());
    assert_eq!(save_default(&mut file), SaveOutcome::Conflict);
    assert_eq!(std::fs::read(&target).unwrap(), b"base external");
    assert!(file.is_dirty());
    let request = SaveRequest {
        overwrite_conflict: true,
        ..SaveRequest::default()
    };
    assert!(matches!(
        file.save(
            &request,
            &SaveOptions::default(),
            None,
            &WriteOptions::default(),
            Instant::now()
        ),
        SaveOutcome::Saved { .. }
    ));
    assert_eq!(std::fs::read(&target).unwrap(), b"base local");
}

#[test]
fn save_conflict_discard_local_changes_reverts_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let (target, mut file) = conflicted(dir.path());
    assert_eq!(save_default(&mut file), SaveOutcome::Conflict);
    let change = file.revert(&defaults(), Instant::now()).unwrap();
    assert!(matches!(change, DiskChange::Reloaded { .. }));
    assert_eq!(contents(&file), "base external");
    assert!(!file.is_dirty());
    assert_eq!(std::fs::read(&target).unwrap(), b"base external");
    // 还原可以撤销（回到本地修改）。
    file.session_mut().undo().unwrap();
    assert_eq!(contents(&file), "base local");
}

#[test]
fn overwrite_file_on_disk_setting_skips_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let (target, mut file) = conflicted(dir.path());
    let settings = TestSettings::new(&[(
        "files.saveConflictResolution",
        Value::String("overwriteFileOnDisk".into()),
    )]);
    let options = SaveOptions::read(&settings, "csharp");
    assert_eq!(
        options.conflict_resolution,
        ConflictResolution::OverwriteFileOnDisk
    );
    assert!(matches!(
        file.save(
            &SaveRequest::default(),
            &options,
            None,
            &WriteOptions::default(),
            Instant::now()
        ),
        SaveOutcome::Saved { .. }
    ));
    assert_eq!(std::fs::read(&target).unwrap(), b"base local");
}

/// 模拟“第一次替换时目标被占用，等待重试期间外部程序写入新内容并释放”（#139）：
/// 替换故障注入点在第一次尝试时写入 `content` 并报占用。
fn busy_once_then_external_write(target: &Path, content: &'static [u8]) -> WriteOptions {
    let target = target.to_path_buf();
    WriteOptions {
        busy_retry_delay: Duration::from_millis(1),
        replace_fail_point: Some(save::ReplaceFailPoint(Arc::new(move |attempt| {
            (attempt == 1).then(|| {
                std::fs::write(&target, content).unwrap();
                save::WriteError::Busy { attempts: 1 }
            })
        }))),
        ..WriteOptions::default()
    }
}

#[test]
fn external_write_during_the_busy_retry_is_reported_as_a_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Player.cs");
    std::fs::write(&target, b"base").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    type_at(&mut file, 4, " local", Instant::now());
    let write = busy_once_then_external_write(&target, b"base external");
    let outcome = file.save(
        &SaveRequest::default(),
        &SaveOptions::default(),
        None,
        &write,
        Instant::now(),
    );
    // 重试之前重新做了冲突检测：外部内容保住，没有静悄悄覆盖。
    assert_eq!(outcome, SaveOutcome::Conflict);
    assert_eq!(std::fs::read(&target).unwrap(), b"base external");
    assert!(file.is_dirty());
    assert!(tmp_files(dir.path()).is_empty());
}

#[test]
fn overwrite_conflict_still_writes_on_the_busy_retry() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Player.cs");
    std::fs::write(&target, b"base").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    type_at(&mut file, 4, " local", Instant::now());
    let write = busy_once_then_external_write(&target, b"base external");
    let request = SaveRequest {
        overwrite_conflict: true,
        ..SaveRequest::default()
    };
    let outcome = file.save(
        &request,
        &SaveOptions::default(),
        None,
        &write,
        Instant::now(),
    );
    assert!(
        matches!(outcome, SaveOutcome::Saved { attempts: 2, .. }),
        "{outcome:?}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"base local");
    assert!(tmp_files(dir.path()).is_empty());
}

#[test]
fn overwrite_file_on_disk_setting_still_writes_on_the_busy_retry() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("Player.cs");
    std::fs::write(&target, b"base").unwrap();
    let settings = TestSettings::new(&[(
        "files.saveConflictResolution",
        Value::String("overwriteFileOnDisk".into()),
    )]);
    let mut file = load(&target, &settings);
    type_at(&mut file, 4, " local", Instant::now());
    let write = busy_once_then_external_write(&target, b"base external");
    let outcome = file.save(
        &SaveRequest::default(),
        &SaveOptions::read(&settings, "csharp"),
        None,
        &write,
        Instant::now(),
    );
    assert!(
        matches!(outcome, SaveOutcome::Saved { attempts: 2, .. }),
        "{outcome:?}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"base local");
}

// ---------------------------------------------------------------------------------------------
// 外部修改。

fn disk(path: &Path) -> Option<(DiskStamp, Vec<u8>)> {
    DiskStamp::read(path).unwrap()
}

#[test]
fn clean_document_reloads_silently_keeping_the_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.cs");
    std::fs::write(&target, "line0\nline1\nline2\n").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    file.session_mut()
        .set_selections(Selections::single(Selection::cursor(ByteOffset(8))));
    // 外部在后面追加一行、改了第三行。
    std::fs::write(&target, "line0\nline1\nLINE2\nline3\n").unwrap();
    let change = file
        .disk_changed(disk(&target), &settings, Instant::now())
        .unwrap();
    assert!(
        matches!(
            change,
            DiskChange::Reloaded {
                encoding_changed: false,
                line_endings_changed: false,
                ..
            }
        ),
        "{change:?}"
    );
    assert_eq!(contents(&file), "line0\nline1\nLINE2\nline3\n");
    assert!(!file.is_dirty());
    assert_eq!(
        file.session().selections().primary().active,
        ByteOffset(8),
        "光标不变"
    );
    // 换行符与编码重新识别：变化时报告给状态栏。
    std::fs::write(&target, "line0\r\nline1\r\n").unwrap();
    let change = file
        .disk_changed(disk(&target), &settings, Instant::now())
        .unwrap();
    assert!(matches!(
        change,
        DiskChange::Reloaded {
            line_endings_changed: true,
            ..
        }
    ));
    assert!(matches!(
        file.line_endings(),
        LineEndings::Uniform(Eol::CrLf)
    ));
    std::fs::write(&target, b"\xEF\xBB\xBFline0\r\n").unwrap();
    let change = file
        .disk_changed(disk(&target), &settings, Instant::now())
        .unwrap();
    assert!(matches!(
        change,
        DiskChange::Reloaded {
            encoding_changed: true,
            ..
        }
    ));
    assert!(file.bom());
}

#[test]
fn dirty_document_asks_once_and_both_choices_work() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.cs");
    std::fs::write(&target, "abc").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    type_at(&mut file, 3, "!", Instant::now());
    std::fs::write(&target, "external 1").unwrap();
    let first = file
        .disk_changed(disk(&target), &settings, Instant::now())
        .unwrap();
    assert!(matches!(first, DiskChange::NeedsDecision { first: true }));
    std::fs::write(&target, "external 2").unwrap();
    let again = file
        .disk_changed(disk(&target), &settings, Instant::now())
        .unwrap();
    assert!(matches!(again, DiskChange::NeedsDecision { first: false }));
    assert_eq!(contents(&file), "abc!", "提示期间本地内容不变");

    // “保留本地修改”：之后保存直接写入，不报冲突。
    file.keep_local_changes();
    assert!(!file.has_pending_external_change());
    assert!(matches!(save_default(&mut file), SaveOutcome::Saved { .. }));
    assert_eq!(std::fs::read(&target).unwrap(), b"abc!");

    // “重新加载（丢弃本地修改）”。
    type_at(&mut file, 4, "?", Instant::now());
    std::fs::write(&target, "external 3").unwrap();
    assert!(matches!(
        file.disk_changed(disk(&target), &settings, Instant::now())
            .unwrap(),
        DiskChange::NeedsDecision { first: true }
    ));
    file.revert(&settings, Instant::now()).unwrap();
    assert_eq!(contents(&file), "external 3");
    assert!(!file.is_dirty());
    assert!(!file.has_pending_external_change());
}

#[test]
fn deleted_file_keeps_content_and_next_save_recreates_it() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.cs");
    std::fs::write(&target, "abc").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    std::fs::remove_file(&target).unwrap();
    assert!(matches!(
        file.disk_changed(None, &settings, Instant::now()).unwrap(),
        DiskChange::Deleted { first: true }
    ));
    assert!(matches!(
        file.disk_changed(None, &settings, Instant::now()).unwrap(),
        DiskChange::Deleted { first: false }
    ));
    assert!(file.is_deleted());
    assert_eq!(contents(&file), "abc");
    // 内容未修改也能保存：视为新建写入。
    assert!(matches!(save_default(&mut file), SaveOutcome::Saved { .. }));
    assert_eq!(std::fs::read(&target).unwrap(), b"abc");
    assert!(!file.is_deleted());
}

/// 监听方的最小模型：自写去重 + 按路径合并 + 交给文档。界面装配（rove-ui 的 `save` 模块）同样如此。
struct Watcher {
    own: SelfWrites,
    coalescer: ChangeCoalescer,
    prompts: usize,
    reloads: usize,
    processed: usize,
}

impl Watcher {
    fn new() -> Self {
        Self {
            own: SelfWrites::default(),
            coalescer: ChangeCoalescer::new(Duration::from_millis(30), Duration::from_millis(300)),
            prompts: 0,
            reloads: 0,
            processed: 0,
        }
    }

    fn flush(&mut self, file: &mut TextFile, settings: &dyn Settings, now: Instant) {
        for path in self.coalescer.due(now) {
            let disk = disk(&path);
            if self
                .own
                .is_own(&path, disk.as_ref().map(|(s, _)| s.hash), now)
            {
                continue;
            }
            self.processed += 1;
            match file.disk_changed(disk, settings, now).unwrap() {
                DiskChange::NeedsDecision { first: true } => self.prompts += 1,
                DiskChange::Reloaded { .. } => self.reloads += 1,
                _ => {}
            }
        }
    }
}

#[test]
fn external_storm_of_100_writes_ends_consistent_without_stacking() {
    for dirty in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("gen.cs");
        std::fs::write(&target, "v0").unwrap();
        let settings = defaults();
        let mut file = load(&target, &settings);
        if dirty {
            type_at(&mut file, 2, "*", Instant::now());
        }
        let mut watcher = Watcher::new();
        for i in 1..=100 {
            std::fs::write(&target, format!("v{i}")).unwrap();
            // 每次写入一个事件（真实监听可能更多或更少，合并后都一样）。
            watcher.coalescer.note(&target, Instant::now());
            watcher.flush(&mut file, &settings, Instant::now());
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !watcher.coalescer.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            watcher.flush(&mut file, &settings, Instant::now());
        }
        assert!(watcher.coalescer.is_empty());
        if dirty {
            assert_eq!(watcher.prompts, 1, "提示条不堆积");
            assert_eq!(contents(&file), "v0*");
            file.revert(&settings, Instant::now()).unwrap();
        } else {
            assert!(watcher.reloads >= 1);
            assert!(watcher.processed < 100, "被合并：{}", watcher.processed);
        }
        assert_eq!(contents(&file), "v100", "最终与磁盘一致");
    }
}

#[test]
fn twenty_consecutive_saves_cause_zero_external_change_reports() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.cs");
    std::fs::write(&target, "x").unwrap();
    let settings = defaults();
    let mut file = load(&target, &settings);
    let mut watcher = Watcher::new();
    for i in 0..20 {
        let end = contents(&file).len() as u64;
        type_at(&mut file, end, &i.to_string(), Instant::now());
        let SaveOutcome::Saved { path, hash, .. } = save_default(&mut file) else {
            panic!("保存失败")
        };
        watcher.own.record(&path, hash, Instant::now());
        // 自己的写入引起的事件：有的在时间窗内到达（被去重），有的“迟到”（交给内容比较）。
        watcher.coalescer.note(&target, Instant::now());
        if i % 2 == 0 {
            std::thread::sleep(Duration::from_millis(40));
            watcher.flush(&mut file, &settings, Instant::now());
        }
        // 迟到的事件：直接交给文档，内容与已知一致，应为 Unchanged。
        let change = file
            .disk_changed(disk(&target), &settings, Instant::now())
            .unwrap();
        assert!(matches!(change, DiskChange::Unchanged), "{change:?}");
    }
    std::thread::sleep(Duration::from_millis(40));
    watcher.flush(&mut file, &settings, Instant::now());
    assert_eq!(watcher.prompts, 0);
    assert_eq!(watcher.reloads, 0);
    assert!(!file.is_dirty());
}

// ---------------------------------------------------------------------------------------------
// 保存时的变换：参数化用例 + 与用户改动是同一个撤销单元。

#[test]
fn save_transforms_are_parameterized_and_join_the_last_undo_unit() {
    struct Case {
        settings: &'static [(&'static str, bool)],
        input: &'static str,
        expected: &'static str,
    }
    let cases = [
        Case {
            settings: &[],
            input: "a  \n\n\n",
            expected: "Xa  \n\n\n",
        },
        Case {
            settings: &[("files.trimTrailingWhitespace", true)],
            input: "a  \nb\t\n",
            expected: "Xa\nb\n",
        },
        Case {
            settings: &[("files.insertFinalNewline", true)],
            input: "a\r\nb",
            expected: "Xa\r\nb\r\n",
        },
        Case {
            settings: &[("files.trimFinalNewlines", true)],
            input: "a\n\n\n",
            expected: "Xa\n",
        },
        Case {
            settings: &[
                ("files.trimTrailingWhitespace", true),
                ("files.trimFinalNewlines", true),
                ("files.insertFinalNewline", true),
            ],
            input: "a  \n  \n\n",
            expected: "Xa\n",
        },
    ];
    for (index, case) in cases.iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        std::fs::write(&target, case.input).unwrap();
        let user: Vec<(&str, Value)> = case
            .settings
            .iter()
            .map(|(k, v)| (*k, Value::Bool(*v)))
            .collect();
        let settings = TestSettings::new(&user);
        let options = SaveOptions::read(&settings, "plaintext");
        let mut file = load(&target, &settings);
        type_at(&mut file, 0, "X", Instant::now());
        let outcome = file.save(
            &SaveRequest::default(),
            &options,
            None,
            &WriteOptions::default(),
            Instant::now(),
        );
        assert!(matches!(outcome, SaveOutcome::Saved { .. }), "case {index}");
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            case.expected,
            "case {index}"
        );
        // 一次撤销同时撤掉变换与用户的输入。
        file.session_mut().undo().unwrap();
        assert_eq!(contents(&file), case.input, "case {index}");
    }
}

#[test]
fn trailing_whitespace_inside_strings_is_kept_when_configured() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.cs");
    std::fs::write(&target, "var s = @\"a  \nb\";  \n").unwrap();
    let settings = TestSettings::new(&[
        ("files.trimTrailingWhitespace", Value::Bool(true)),
        (
            "files.trimTrailingWhitespaceInRegexAndStrings",
            Value::Bool(false),
        ),
    ]);
    let options = SaveOptions::read(&settings, "csharp");
    let mut file = load(&target, &settings);
    type_at(&mut file, 0, "/", Instant::now());
    // 语法层的判断：第 0 行的行尾在逐字字符串里。
    let in_string = |line: Line, _: rove_base::Utf8Column| line == Line(0);
    file.save(
        &SaveRequest::default(),
        &options,
        Some(&in_string),
        &WriteOptions::default(),
        Instant::now(),
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "/var s = @\"a  \nb\";\n"
    );
}

#[test]
fn auto_save_policy_follows_vscode() {
    use save::{AutoSave, AutoSaveTrigger};
    let settings = TestSettings::new(&[
        ("files.autoSave", Value::String("afterDelay".into())),
        ("files.autoSaveDelay", Value::Number(250.0)),
        ("files.autoSaveWorkspaceFilesOnly", Value::Bool(true)),
        ("files.autoSaveWhenNoErrors", Value::Bool(true)),
    ]);
    let options = SaveOptions::read(&settings, "csharp");
    assert_eq!(
        options.auto_save,
        AutoSave::AfterDelay(Duration::from_millis(250))
    );
    assert!(options.auto_save_due(AutoSaveTrigger::Delay, false, true, false));
    assert!(!options.auto_save_due(AutoSaveTrigger::FocusChange, false, true, false));
    assert!(
        !options.auto_save_due(AutoSaveTrigger::Delay, true, true, false),
        "未命名"
    );
    assert!(
        !options.auto_save_due(AutoSaveTrigger::Delay, false, false, false),
        "工作区外"
    );
    assert!(
        !options.auto_save_due(AutoSaveTrigger::Delay, false, true, true),
        "有错误"
    );
    for (mode, focus, window) in [
        ("off", false, false),
        ("onFocusChange", true, true),
        ("onWindowChange", false, true),
    ] {
        let settings = TestSettings::new(&[("files.autoSave", Value::String(mode.into()))]);
        let options = SaveOptions::read(&settings, "");
        assert_eq!(
            options.auto_save_due(AutoSaveTrigger::FocusChange, false, true, false),
            focus,
            "{mode}"
        );
        assert_eq!(
            options.auto_save_due(AutoSaveTrigger::WindowChange, false, true, false),
            window,
            "{mode}"
        );
        assert!(!options.auto_save_due(AutoSaveTrigger::Delay, false, true, false));
    }
}

// ---------------------------------------------------------------------------------------------
// 未命名文档。

#[test]
fn untitled_document_uses_files_encoding_and_eol_and_needs_a_path() {
    let settings = TestSettings::new(&[
        ("files.encoding", Value::String("gbk".into())),
        ("files.eol", Value::String("\n".into())),
    ]);
    let mut file = TextFile::untitled("Untitled-1", "plaintext", &settings, EditOptions::default());
    assert!(file.is_untitled());
    assert!(!file.is_dirty(), "空的未命名文档不需要确认关闭");
    type_at(&mut file, 0, "中文\n第二行", Instant::now());
    assert!(file.is_dirty());
    assert!(file.needs_close_confirmation());
    assert_eq!(save_default(&mut file), SaveOutcome::NeedsPath);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("new.txt");
    let request = SaveRequest {
        save_as: Some(target.clone()),
        ..SaveRequest::default()
    };
    assert!(matches!(
        file.save(
            &request,
            &SaveOptions::read(&settings, "plaintext"),
            None,
            &WriteOptions::default(),
            Instant::now()
        ),
        SaveOutcome::Saved { .. }
    ));
    assert_eq!(
        std::fs::read(&target).unwrap(),
        encode_text(&Encoding::Gbk, false, "中文\n第二行")
    );
    assert_eq!(file.path(), Some(target.as_path()));
    assert!(!file.is_untitled());
    assert!(!file.is_dirty());

    let crlf = TestSettings::new(&[("files.encoding", Value::String("utf8bom".into()))]);
    let file = TextFile::untitled("Untitled-2", "plaintext", &crlf, EditOptions::default());
    assert!(file.bom());
    assert_eq!(file.encoding(), &Encoding::Utf8);
    assert!(matches!(
        file.line_endings(),
        LineEndings::Uniform(e) if *e == SaveOptions::default().new_file_eol()
    ));
}

#[test]
fn save_as_keeps_encoding_and_moves_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.cs");
    std::fs::write(&from, b"\xFF\xFEa\0").unwrap();
    let settings = defaults();
    let mut file = load(&from, &settings);
    let to = dir.path().join("b.cs");
    let request = SaveRequest {
        save_as: Some(to.clone()),
        ..SaveRequest::default()
    };
    assert!(matches!(
        file.save(
            &request,
            &SaveOptions::default(),
            None,
            &WriteOptions::default(),
            Instant::now()
        ),
        SaveOutcome::Saved { .. }
    ));
    assert_eq!(std::fs::read(&to).unwrap(), b"\xFF\xFEa\0");
    assert_eq!(file.path(), Some(to.as_path()));
    assert_eq!(file.snapshot().byte_len(), 1);
}
