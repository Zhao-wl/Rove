//! 设置服务的入口集成测试：读设置 → 改一个键 → 写回（保留格式、先备份、原子写入）→ 热加载。
//! 全部在临时目录里进行，不触碰真实的 `%APPDATA%\Code\User`。

use rove_base::{Cancellation, Error, Result, Task, Value};
use rove_config::{
    builtin_registry, Key, Layer, ProblemKind, RetryPolicy, Scope, Settings, SettingsChange,
    SettingsOptions, SettingsService, WatchOptions,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

fn block_on<T>(mut task: Task<T>) -> Result<T> {
    let mut cx = Context::from_waker(Waker::noop());
    match task.as_mut().poll(&mut cx) {
        Poll::Ready(r) => r,
        Poll::Pending => panic!("settings tasks finish in one poll"),
    }
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new(settings: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        if let Some(text) = settings {
            fs::create_dir_all(dir.path().join("User")).unwrap();
            fs::write(dir.path().join("User").join("settings.json"), text).unwrap();
        }
        Self { dir }
    }

    fn user_dir(&self) -> PathBuf {
        self.dir.path().join("User")
    }

    fn backup_dir(&self) -> PathBuf {
        self.dir
            .path()
            .join("Local")
            .join("Rove")
            .join("settings-backup")
    }

    fn settings_file(&self) -> PathBuf {
        self.user_dir().join("settings.json")
    }

    fn options(&self, watch: bool) -> SettingsOptions {
        let mut options = SettingsOptions::new(self.user_dir(), self.backup_dir());
        options.watch = watch.then_some(WatchOptions {
            poll_interval: Duration::from_millis(10),
            debounce: Duration::from_millis(20),
        });
        options
    }

    fn open(&self, watch: bool) -> SettingsService {
        SettingsService::open(self.options(watch), builtin_registry())
    }

    fn text(&self) -> String {
        fs::read_to_string(self.settings_file()).unwrap()
    }

    fn backups(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(self.backup_dir())
            .map(|d| d.map(|e| e.unwrap().path()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
}

fn scope(language: &str) -> Scope {
    Scope {
        language: Some(language.into()),
    }
}

fn num(n: f64) -> Value {
    Value::Number(n)
}

fn string(s: &str) -> Value {
    Value::String(s.into())
}

fn write(s: &SettingsService, key: &str, scope: Scope, value: Value) -> Result<()> {
    block_on(s.write(key.into(), scope, value, Cancellation::default()))
}

fn record_changes(
    s: &SettingsService,
) -> (Arc<Mutex<Vec<SettingsChange>>>, rove_base::Subscription) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let sub = s
        .changes()
        .subscribe(Arc::new(move |c| sink.lock().unwrap().push(c)));
    (seen, sub)
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// 编辑器常见的保存方式：写临时文件再重命名覆盖。
fn save_like_an_editor(path: &Path, text: &str) {
    let tmp = path.with_extension("json.tmp~");
    fs::write(&tmp, text).unwrap();
    fs::rename(&tmp, path).unwrap();
}

const USER_FILE: &str = "\u{feff}// 我的 VS Code 设置\r\n{\r\n\t// ---- 外观 ----\r\n\t\"workbench.colorTheme\": \"Dark 2026\", /* 主题 */\r\n\r\n\t\"editor.fontSize\": 15,\r\n\t\"editor.tabSize\": 3,\r\n\t\"[csharp]\": {\r\n\t\t\"editor.tabSize\": 8, // C# 用 8\r\n\t},\r\n\t\"gitlens.codeLens.enabled\": false,\r\n\t\"search.useIgnoreFiles\": true,\r\n}\r\n";

#[test]
fn language_override_beats_global_beats_rove_default_beats_vscode_default() {
    let fx = Fixture::new(Some(USER_FILE));
    let s = fx.open(false);
    let tab = Key::<f64>::new("editor.tabSize");
    // 语言级用户覆盖 > 全局用户值
    assert_eq!(tab.read(&s, &scope("csharp")).unwrap(), 8.0);
    assert_eq!(
        s.layer("editor.tabSize", &scope("csharp")).unwrap(),
        Layer::UserLanguage
    );
    assert_eq!(tab.read(&s, &scope("plaintext")).unwrap(), 3.0);
    assert_eq!(tab.read(&s, &Scope::default()).unwrap(), 3.0);
    // VS Code 的默认语言覆盖（[yaml] tabSize = 2）高于全局用户值，与 VS Code 一致。
    assert_eq!(tab.read(&s, &scope("yaml")).unwrap(), 2.0);
    assert_eq!(
        s.layer("editor.tabSize", &scope("yaml")).unwrap(),
        Layer::LanguageDefault
    );
    // 未设置 → Rove 默认值 > VS Code 默认值
    let guess = Key::<bool>::new("files.autoGuessEncoding");
    assert!(guess.read(&s, &scope("csharp")).unwrap());
    assert_eq!(
        s.layer("files.autoGuessEncoding", &Scope::default())
            .unwrap(),
        Layer::RoveDefault
    );
    // 没有 Rove 默认值的设置项 → VS Code 默认值
    assert_eq!(
        Key::<f64>::new("editor.lineHeight")
            .read(&s, &Scope::default())
            .unwrap(),
        0.0
    );
    assert_eq!(
        s.layer("editor.lineHeight", &Scope::default()).unwrap(),
        Layer::Default
    );
    // 不可按语言覆盖的设置项忽略语言块
    assert_eq!(
        s.read("workbench.colorTheme", &scope("csharp")).unwrap(),
        string("Dark 2026")
    );
}

#[test]
fn rove_defaults_apply_only_while_unset_and_are_never_written() {
    let fx = Fixture::new(Some("{\n    \"editor.fontSize\": 15\n}\n"));
    let s = fx.open(false);
    let rove_defaults = [
        (
            "files.autoGuessEncoding",
            Value::Bool(true),
            Value::Bool(false),
        ),
        (
            "files.candidateGuessEncodings",
            Value::Array(Arc::from([string("gbk")])),
            Value::Array(Arc::from([string("big5")])),
        ),
        (
            "workbench.editorAssociations",
            Value::Object(Arc::new(
                [("*.md".to_string(), string("vscode.markdown.preview.editor"))].into(),
            )),
            Value::Object(Arc::new([("*.md".to_string(), string("default"))].into())),
        ),
        (
            "markdown.preview.doubleClickToSwitchToEditor",
            Value::Bool(true),
            Value::Bool(false),
        ),
    ];
    for (key, rove, _) in &rove_defaults {
        assert_eq!(
            &s.read(key, &Scope::default()).unwrap(),
            rove,
            "{key} 未设置时用 Rove 默认值"
        );
        assert!(!s.is_user_set(key, &Scope::default()).unwrap());
    }
    // 写入别的键：共用文件里不出现任何 Rove 默认值。
    write(&s, "editor.fontSize", Scope::default(), num(16.0)).unwrap();
    write(&s, "editor.tabSize", scope("markdown"), num(2.0)).unwrap();
    let text = fx.text();
    for (key, _, _) in &rove_defaults {
        assert!(!text.contains(key), "{key} 被写进了共用文件：\n{text}");
    }
    assert!(!text.contains("gbk") && !text.contains("preview.editor"));
    // 用户设置后让位，即使用户的值等于 VS Code 默认值。
    for (key, _, user) in &rove_defaults {
        write(&s, key, Scope::default(), user.clone()).unwrap();
        assert_eq!(&s.read(key, &Scope::default()).unwrap(), user, "{key}");
        assert!(s.is_user_set(key, &Scope::default()).unwrap());
        assert_eq!(s.layer(key, &Scope::default()).unwrap(), Layer::User);
    }
    // 只在 [csharp] 里设置：C# 用用户值，其他语言仍用 Rove 默认值。
    let fx = Fixture::new(Some("{\"[csharp]\": {\"files.autoGuessEncoding\": false}}"));
    let s = fx.open(false);
    assert_eq!(
        s.read("files.autoGuessEncoding", &scope("csharp")).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        s.read("files.autoGuessEncoding", &scope("xml")).unwrap(),
        Value::Bool(true)
    );
    assert!(s
        .is_user_set("files.autoGuessEncoding", &scope("csharp"))
        .unwrap());
    assert!(!s
        .is_user_set("files.autoGuessEncoding", &Scope::default())
        .unwrap());
}

#[test]
fn unsupported_keys_are_silently_ignored() {
    let fx = Fixture::new(Some(USER_FILE));
    let s = fx.open(false);
    assert_eq!(
        s.read("search.useIgnoreFiles", &Scope::default()),
        Err(Error::NotFound)
    );
    assert_eq!(
        s.read("gitlens.codeLens.enabled", &Scope::default()),
        Err(Error::NotFound)
    );
    assert!(s.problems().is_empty());
    let (seen, _sub) = record_changes(&s);
    let problems = Arc::new(Mutex::new(0));
    let sink = problems.clone();
    let _p = s
        .problem_changes()
        .subscribe(Arc::new(move |_| *sink.lock().unwrap() += 1));
    fs::write(
        fx.settings_file(),
        USER_FILE.replace(
            "\"gitlens.codeLens.enabled\": false",
            "\"gitlens.codeLens.enabled\": true, \"foo.bar\": 1",
        ),
    )
    .unwrap();
    s.reload();
    assert!(
        seen.lock().unwrap().is_empty(),
        "不支持的键变化不产生变更通知"
    );
    assert_eq!(*problems.lock().unwrap(), 0, "也不产生任何问题提示");
    assert!(s.problems().is_empty());
}

#[test]
fn writing_preserves_every_byte_except_the_value_and_backs_up_first() {
    let fx = Fixture::new(Some(USER_FILE));
    let s = fx.open(false);
    write(
        &s,
        "workbench.colorTheme",
        Scope::default(),
        string("Light 2026"),
    )
    .unwrap();
    assert_eq!(
        fx.text(),
        USER_FILE.replace("\"Dark 2026\"", "\"Light 2026\"")
    );
    write(&s, "editor.tabSize", scope("csharp"), num(4.0)).unwrap();
    assert_eq!(
        fx.text(),
        USER_FILE
            .replace("\"Dark 2026\"", "\"Light 2026\"")
            .replace("8, // C# 用 8", "4, // C# 用 8")
    );
    // 新增键：插在末尾，跟随制表符缩进、CRLF 与尾逗号风格。
    write(&s, "editor.lineHeight", Scope::default(), num(22.0)).unwrap();
    assert!(fx
        .text()
        .ends_with("\t\"search.useIgnoreFiles\": true,\r\n\t\"editor.lineHeight\": 22,\r\n}\r\n"));
    // 每次写入前都有一份备份，内容是写入前的文件。
    let backups = fx.backups();
    assert_eq!(backups.len(), 3);
    assert_eq!(fs::read_to_string(&backups[0]).unwrap(), USER_FILE);
    assert_eq!(
        s.read("workbench.colorTheme", &Scope::default()).unwrap(),
        string("Light 2026")
    );
    // 写入同样的值：不写文件，也不产生备份。
    write(&s, "editor.lineHeight", Scope::default(), num(22.0)).unwrap();
    assert_eq!(fx.backups().len(), 3);
    // 删除（重置为默认值）同样保留其余字节。
    block_on(s.remove(
        "editor.lineHeight".into(),
        Scope::default(),
        Cancellation::default(),
    ))
    .unwrap();
    assert_eq!(
        fx.text(),
        USER_FILE
            .replace("\"Dark 2026\"", "\"Light 2026\"")
            .replace("8, // C# 用 8", "4, // C# 用 8")
    );
    assert_eq!(
        s.layer("editor.lineHeight", &Scope::default()).unwrap(),
        Layer::Default
    );
}

#[test]
fn backups_are_capped_and_a_failed_backup_aborts_the_write() {
    let fx = Fixture::new(Some("{}"));
    let mut options = fx.options(false);
    options.backup_limit = 3;
    let s = SettingsService::open(options, builtin_registry());
    for size in 10..16 {
        write(&s, "editor.fontSize", Scope::default(), num(size as f64)).unwrap();
    }
    assert_eq!(fx.backups().len(), 3);
    assert!(fs::read_to_string(fx.backups().last().unwrap())
        .unwrap()
        .contains("14"));

    // 备份目录无法创建（同名文件挡住了路径）→ 不写入。
    let fx = Fixture::new(Some("{}"));
    fs::create_dir_all(fx.backup_dir().parent().unwrap()).unwrap();
    fs::write(fx.backup_dir(), "not a directory").unwrap();
    let s = fx.open(false);
    let err = write(&s, "editor.fontSize", Scope::default(), num(20.0)).unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "{err:?}");
    assert_eq!(fx.text(), "{}");
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(14.0)
    );
}

#[test]
fn missing_file_is_not_created_until_the_first_explicit_write() {
    let fx = Fixture::new(None);
    let s = fx.open(true);
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(14.0)
    );
    std::thread::sleep(Duration::from_millis(60));
    assert!(!fx.user_dir().exists(), "启动时不创建、不写入共用文件");
    write(
        &s,
        "workbench.colorTheme",
        Scope::default(),
        string("Light 2026"),
    )
    .unwrap();
    assert_eq!(
        fx.text(),
        "{\n    \"workbench.colorTheme\": \"Light 2026\"\n}\n"
    );
    assert!(fx.backups().is_empty(), "原来没有文件，无需备份");
}

#[test]
fn invalid_writes_and_cancellation_leave_the_file_untouched() {
    let fx = Fixture::new(Some(USER_FILE));
    let s = fx.open(false);
    assert_eq!(
        write(&s, "no.such.setting", Scope::default(), num(1.0)),
        Err(Error::NotFound)
    );
    assert_eq!(
        write(&s, "workbench.colorTheme", scope("csharp"), string("x")),
        Err(Error::InvalidInput),
        "不可按语言覆盖的键不能写进语言块"
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        block_on(s.write("editor.fontSize".into(), Scope::default(), num(1.0), cancel)),
        Err(Error::Cancelled)
    );
    assert_eq!(fx.text(), USER_FILE);
    assert!(fx.backups().is_empty());
}

#[test]
fn syntax_errors_keep_the_last_valid_settings_and_report_line_and_column() {
    let fx = Fixture::new(Some(USER_FILE));
    let s = fx.open(false);
    let broken = USER_FILE.replace(
        "\"editor.fontSize\": 15,",
        "\"editor.fontSize\": 99\r\n\t\"oops\"",
    );
    fs::write(fx.settings_file(), &broken).unwrap();
    s.reload();
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(15.0)
    );
    let problems = s.problems();
    assert_eq!(problems.len(), 1);
    let p = &problems[0];
    assert!(p.is_error());
    assert!(matches!(p.kind, ProblemKind::Syntax(_)), "{p:?}");
    assert_eq!((p.line, p.column), (7, 2));
    assert_eq!(&*p.path, fx.settings_file().as_path());
    // 文件有语法错误时拒绝写入，避免越改越坏。
    assert_eq!(
        write(&s, "editor.fontSize", Scope::default(), num(16.0)),
        Err(Error::InvalidInput)
    );
    assert_eq!(fx.text(), broken);
    // 修好后恢复，问题清空。
    fs::write(fx.settings_file(), USER_FILE.replace("15", "17")).unwrap();
    s.reload();
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(17.0)
    );
    assert!(s.problems().is_empty());

    // 第一次读取就失败：使用默认值并报告。
    let fx = Fixture::new(Some("{\n  \"editor.fontSize\": 15,,\n}"));
    let s = fx.open(false);
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(14.0)
    );
    assert_eq!((s.problems()[0].line, s.problems()[0].column), (2, 25));
}

#[test]
fn mismatched_types_fall_back_with_a_located_warning() {
    let fx = Fixture::new(Some(
        "{\n  \"editor.fontSize\": \"big\",\n  \"[csharp]\": { \"editor.tabSize\": \"wide\" },\n  \"editor.tabSize\": 6\n}",
    ));
    let s = fx.open(false);
    let font = Key::<f64>::new("editor.fontSize");
    assert_eq!(s.read_typed(&font, &Scope::default()).unwrap(), 14.0);
    // 语言块里的错误值跳过后回落到全局用户值。
    assert_eq!(
        s.read_typed(&Key::<f64>::new("editor.tabSize"), &scope("csharp"))
            .unwrap(),
        6.0
    );
    // 重复读取不会重复记录。
    s.read_typed(&font, &Scope::default()).unwrap();
    let problems = s.problems();
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems.iter().all(|p| !p.is_error()));
    assert_eq!(
        problems[0].kind,
        ProblemKind::TypeMismatch {
            key: "editor.fontSize".into()
        }
    );
    assert_eq!((problems[0].line, problems[0].column), (2, 22));
    assert_eq!((problems[1].line, problems[1].column), (3, 35));
    // 契约的 Key<T>::read 不做强制转换：类型不符返回 InvalidInput，由消费方决定回落。
    assert_eq!(font.read(&s, &Scope::default()), Err(Error::InvalidInput));
    // 消费方用错类型（默认值本身也解不出）→ InvalidInput。
    assert_eq!(
        s.read_typed(&Key::<bool>::new("editor.lineHeight"), &Scope::default()),
        Err(Error::InvalidInput)
    );
}

/// 入口集成测试：读设置 → 改一个键 → 写回 → 热加载（双向）。
#[test]
fn read_write_and_hot_reload_round_trip() {
    let fx = Fixture::new(Some(USER_FILE));
    let s = fx.open(true);
    let (seen, _sub) = record_changes(&s);
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(15.0)
    );

    // Rove 写入：立即生效、发一次通知；之后监听线程看到的是自己写的内容，不再重载。
    write(&s, "editor.fontSize", Scope::default(), num(18.0)).unwrap();
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(18.0)
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        &*seen.lock().unwrap()[0].keys,
        [Arc::<str>::from("editor.fontSize")]
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(seen.lock().unwrap().len(), 1, "自己的写入不应触发重载");

    // VS Code 修改（写临时文件再重命名）：Rove 实时生效。
    save_like_an_editor(&fx.settings_file(), &fx.text().replace("18", "21"));
    wait_until("external change", || {
        s.read("editor.fontSize", &Scope::default()).unwrap() == num(21.0)
    });
    let changes = seen.lock().unwrap().clone();
    assert_eq!(changes.len(), 2);
    assert!(changes[1].revision > changes[0].revision);

    // 短时间内多次写入被合并（去抖），最终值正确。
    let template = fx
        .text()
        .replace("\"editor.fontSize\": 21", "\"editor.fontSize\": SIZE");
    for size in [30, 31, 32] {
        fs::write(
            fx.settings_file(),
            template.replace("SIZE", &size.to_string()),
        )
        .unwrap();
    }
    wait_until("debounced change", || {
        s.read("editor.fontSize", &Scope::default()).unwrap() == num(32.0)
    });

    // 外部写坏：沿用上一份有效设置，报告问题；修好后恢复。
    fs::write(fx.settings_file(), "{ \"editor.fontSize\": }").unwrap();
    wait_until("syntax problem", || !s.problems().is_empty());
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(32.0)
    );
    save_like_an_editor(&fx.settings_file(), "{ \"editor.fontSize\": 12 }");
    wait_until("recovery", || s.problems().is_empty());
    assert_eq!(
        s.read("editor.fontSize", &Scope::default()).unwrap(),
        num(12.0)
    );
}

#[test]
fn keybindings_changes_are_forwarded() {
    let fx = Fixture::new(Some("{}"));
    let s = fx.open(true);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let _sub =
        s.keybindings_changes()
            .subscribe(Arc::new(move |c: rove_config::KeybindingsChange| {
                sink.lock().unwrap().push(c.text)
            }));
    let path = fx.user_dir().join("keybindings.json");
    assert_eq!(s.keybindings_path(), path.as_path());
    save_like_an_editor(&path, "[{\"key\": \"f12\", \"command\": \"x\"}]");
    wait_until("keybindings change", || !seen.lock().unwrap().is_empty());
    assert_eq!(
        seen.lock().unwrap()[0].as_deref(),
        Some("[{\"key\": \"f12\", \"command\": \"x\"}]")
    );
}

#[cfg(windows)]
#[test]
fn writes_retry_while_the_file_is_held_open_by_another_process() {
    use std::os::windows::fs::OpenOptionsExt;
    let fx = Fixture::new(Some("{\n    \"editor.fontSize\": 15\n}\n"));
    let s = Arc::new(fx.open(false));

    // 不共享任何访问：读、写、删除/重命名都会遇到共享冲突。
    let hold = || {
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(fx.settings_file())
            .unwrap()
    };
    let held = hold();
    let writer = {
        let s = s.clone();
        std::thread::spawn(move || write(&s, "editor.fontSize", Scope::default(), num(16.0)))
    };
    std::thread::sleep(Duration::from_millis(300));
    drop(held);
    writer.join().unwrap().unwrap();
    assert_eq!(fx.text(), "{\n    \"editor.fontSize\": 16\n}\n");

    // 一直被占用：重试用尽后报告 Io 错误，文件保持原样。
    let mut options = fx.options(false);
    options.retry = RetryPolicy {
        attempts: 3,
        delay: Duration::from_millis(10),
    };
    let s = SettingsService::open(options, builtin_registry());
    let held = hold();
    let err = write(&s, "editor.fontSize", Scope::default(), num(17.0)).unwrap_err();
    drop(held);
    assert!(matches!(err, Error::Io { .. }), "{err:?}");
    assert_eq!(fx.text(), "{\n    \"editor.fontSize\": 16\n}\n");
    let leftovers: Vec<_> = fs::read_dir(fx.user_dir()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "临时文件已清理");
}
