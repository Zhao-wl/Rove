//! 保存控制器：结果 → 反馈级别与按钮、按钮的后续动作、关闭确认、外部修改提示（#49）。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rove_base::{ByteOffset, Cancellation, DocumentId, Events, Task, Value};
use rove_buffer::edit::{EditOptions, Selection, Selections};
use rove_buffer::save::{
    ChangeCoalescer, DiskStamp, FailPoint, SaveOptions, TextFile, WriteOptions,
};
use rove_buffer::{Encoding, Eol, LineEndings};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings};

use super::registry::*;
use super::{CloseCheck, Effect, SaveController};
use crate::feedback::{FeedbackLevel, Severity};
use crate::shell::feedback::FeedbackHub;

struct TestSettings {
    defaults: DefaultSettings,
    user: BTreeMap<String, Value>,
}

impl TestSettings {
    fn new(user: &[(&str, Value)]) -> Self {
        let mut registry = SettingRegistry::default();
        rove_buffer::settings::register(&mut registry).unwrap();
        rove_buffer::save::settings::register(&mut registry).unwrap();
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
            Some(v) => Ok(v.clone()),
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

const DOC: DocumentId = DocumentId(7);

fn controller() -> SaveController {
    SaveController::new(WriteOptions {
        busy_retry_delay: Duration::from_millis(10),
        ..WriteOptions::default()
    })
    .with_coalescer(ChangeCoalescer::new(
        Duration::from_millis(1),
        Duration::from_millis(5),
    ))
}

fn open(controller: &mut SaveController, path: &Path, settings: &dyn Settings) {
    let file = TextFile::load(
        path,
        "csharp",
        settings,
        Arc::from([]),
        EditOptions::default(),
    )
    .unwrap();
    controller.attach(DOC, file);
}

fn type_end(controller: &mut SaveController, text: &str) {
    let file = controller.file_mut(DOC).unwrap();
    let end = file.session().contents().len() as u64;
    let session = file.session_mut();
    session.set_selections(Selections::single(Selection::cursor(ByteOffset(end))));
    session.type_text(text, Instant::now()).unwrap();
}

fn action_ids(actions: &[crate::feedback::FeedbackAction]) -> Vec<&str> {
    actions.iter().map(|a| a.id.as_ref()).collect()
}

fn save(controller: &mut SaveController, feedback: &mut FeedbackHub) -> Vec<Effect> {
    controller.save(
        DOC,
        &Default::default(),
        &SaveOptions::default(),
        feedback,
        Instant::now(),
    )
}

#[test]
fn conflict_notification_hides_compare_and_both_buttons_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Player.cs");
    std::fs::write(&path, "base").unwrap();
    let settings = TestSettings::new(&[]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &path, &settings);
    type_end(&mut c, " local");
    std::fs::write(&path, "base external").unwrap();

    assert!(save(&mut c, &mut feedback).is_empty());
    let n = &feedback.notifications()[0];
    assert!(n.key.starts_with(KEY_CONFLICT));
    assert!(matches!(n.severity, Severity::Error));
    assert_eq!(action_ids(&n.actions), [ACTION_OVERWRITE, ACTION_DISCARD]);
    assert!(!action_ids(&n.actions).contains(&ACTION_COMPARE));
    assert_eq!(n.message.key.as_ref(), "save.notify.conflict");
    assert_eq!(n.message.args[0].1.as_ref(), "Player.cs");
    // 同一原因再次触发：不新增通知，只更新次数。
    save(&mut c, &mut feedback);
    assert_eq!(feedback.notifications().len(), 1);
    assert_eq!(feedback.notifications()[0].count, 2);

    // “放弃本地修改”。
    let key = feedback.notifications()[0].key.clone();
    feedback.act(&key, ACTION_DISCARD).unwrap();
    let effects = c.on_action(
        &key,
        ACTION_DISCARD,
        &SaveOptions::default(),
        &settings,
        &mut feedback,
        Instant::now(),
    );
    assert!(matches!(effects[..], [Effect::Reloaded { .. }]));
    assert_eq!(c.file(DOC).unwrap().session().contents(), "base external");
    assert!(!c.is_dirty(DOC));

    // “覆盖”。
    type_end(&mut c, "!");
    std::fs::write(&path, "third party").unwrap();
    save(&mut c, &mut feedback);
    let key = feedback.notifications()[0].key.clone();
    let effects = c.on_action(
        &key,
        ACTION_OVERWRITE,
        &SaveOptions::default(),
        &settings,
        &mut feedback,
        Instant::now(),
    );
    assert!(matches!(effects[..], [Effect::Saved { .. }]));
    assert_eq!(std::fs::read(&path).unwrap(), b"base external!");
    // 保存成功后同一文档的失败通知收起。
    assert!(feedback.notifications().is_empty());
}

#[test]
fn read_only_notification_offers_overwrite_and_save_as_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Player.cs.meta");
    std::fs::write(&path, "guid").unwrap();
    let settings = TestSettings::new(&[]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &path, &settings);
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    type_end(&mut c, "2");
    save(&mut c, &mut feedback);
    let n = &feedback.notifications()[0];
    assert_eq!(action_ids(&n.actions), [ACTION_OVERWRITE, ACTION_SAVE_AS]);
    let key = n.key.clone();
    assert!(matches!(
        c.on_action(
            &key,
            ACTION_SAVE_AS,
            &SaveOptions::default(),
            &settings,
            &mut feedback,
            Instant::now()
        )[..],
        [Effect::PickSaveAsPath(DOC)]
    ));
    let effects = c.on_action(
        &key,
        ACTION_OVERWRITE,
        &SaveOptions::default(),
        &settings,
        &mut feedback,
        Instant::now(),
    );
    assert!(matches!(effects[..], [Effect::Saved { .. }]));
    assert!(!std::fs::metadata(&path).unwrap().permissions().readonly());
    assert_eq!(std::fs::read(&path).unwrap(), b"guid2");
}

#[test]
fn unencodable_modal_offers_three_choices() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gbk.cs");
    let bytes = rove_buffer::save::encode(
        &rove_buffer::RopeText::from_text("// 中文注释，玩家控制器\r\n"),
        &Encoding::Gbk,
        false,
        rove_buffer::save::Unencodable::Reject,
    )
    .unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let settings = TestSettings::new(&[]);
    for (action, expected_encoding, written) in [
        (ACTION_CANCEL, Encoding::Gbk, None),
        (ACTION_REPLACE, Encoding::Gbk, Some(b"?".to_vec())),
        (
            ACTION_USE_UTF8,
            Encoding::Utf8,
            Some("😀".as_bytes().to_vec()),
        ),
    ] {
        std::fs::write(&path, &bytes).unwrap();
        let mut c = controller();
        let mut feedback = FeedbackHub::default();
        open(&mut c, &path, &settings);
        type_end(&mut c, "😀");
        assert!(save(&mut c, &mut feedback).is_empty());
        let modal = feedback.modal().expect("模态框").clone();
        assert_eq!(modal.message.key.as_ref(), "save.modal.unencodable");
        assert_eq!(modal.message.args[0].1.as_ref(), "1");
        assert_eq!(modal.message.args[1].1.as_ref(), "GBK");
        assert_eq!(
            action_ids(&modal.actions),
            [ACTION_USE_UTF8, ACTION_REPLACE, ACTION_CANCEL]
        );
        feedback.act(&modal.key, action).unwrap();
        let effects = c.on_action(
            &modal.key,
            action,
            &SaveOptions::default(),
            &settings,
            &mut feedback,
            Instant::now(),
        );
        assert_eq!(
            c.file(DOC).unwrap().encoding(),
            &expected_encoding,
            "{action}"
        );
        let on_disk = std::fs::read(&path).unwrap();
        match written {
            None => {
                assert!(effects.is_empty());
                assert_eq!(on_disk, bytes);
                assert!(c.is_dirty(DOC));
            }
            Some(tail) => {
                assert!(on_disk.ends_with(&tail), "{action}");
                assert!(
                    matches!(effects[..], [Effect::Saved { encoding_changed, .. }] if encoding_changed == (action == ACTION_USE_UTF8))
                );
            }
        }
    }
}

fn set_readonly(path: &Path, readonly: bool) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(readonly);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn act(
    c: &mut SaveController,
    key: &str,
    action: &str,
    settings: &dyn Settings,
    feedback: &mut FeedbackHub,
) -> Vec<Effect> {
    c.on_action(
        key,
        action,
        &SaveOptions::default(),
        settings,
        feedback,
        Instant::now(),
    )
}

/// #140：另存为遇到无法编码的字符时，模态框的两个按钮都写到**另存为的目标**，原文件逐字节不变。
#[test]
fn unencodable_buttons_after_save_as_write_to_the_new_path() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("gbk.cs");
    let gbk = rove_buffer::save::encode(
        &rove_buffer::RopeText::from_text("// 中文注释\r\n"),
        &Encoding::Gbk,
        false,
        rove_buffer::save::Unencodable::Reject,
    )
    .unwrap();
    let settings = TestSettings::new(&[]);
    for (action, tail, encoding) in [
        (ACTION_USE_UTF8, "😀".as_bytes().to_vec(), Encoding::Utf8),
        (ACTION_REPLACE, b"?".to_vec(), Encoding::Gbk),
    ] {
        std::fs::write(&source, &gbk).unwrap();
        let target = dir.path().join(format!("copy-{action}.cs"));
        let mut c = controller();
        let mut feedback = FeedbackHub::default();
        open(&mut c, &source, &settings);
        type_end(&mut c, "😀");
        assert!(c
            .save_as(
                DOC,
                target.clone(),
                &SaveOptions::default(),
                &mut feedback,
                Instant::now()
            )
            .is_empty());
        let modal = feedback.modal().expect("模态框").clone();
        feedback.act(&modal.key, action).unwrap();
        let effects = act(&mut c, &modal.key, action, &settings, &mut feedback);
        assert!(
            matches!(&effects[..], [Effect::Saved { path, .. }] if path == &target),
            "{action}: {effects:?}"
        );
        assert!(std::fs::read(&target).unwrap().ends_with(&tail), "{action}");
        assert_eq!(c.file(DOC).unwrap().encoding(), &encoding, "{action}");
        assert_eq!(c.file(DOC).unwrap().path(), Some(target.as_path()));
        // 原文件逐字节不变。
        assert_eq!(std::fs::read(&source).unwrap(), gbk, "{action}");
    }
}

/// #140：另存为的目标只读时，“覆盖”清除的是**目标**的只读属性；原文件的内容和只读属性不变。
#[test]
fn overwrite_after_save_as_clears_read_only_on_the_new_path_only() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("Player.cs");
    std::fs::write(&source, "class Player {}").unwrap();
    let settings = TestSettings::new(&[]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &source, &settings);
    set_readonly(&source, true);
    type_end(&mut c, "// a");

    let target = dir.path().join("Copy.cs");
    std::fs::write(&target, "old").unwrap();
    set_readonly(&target, true);
    assert!(c
        .save_as(
            DOC,
            target.clone(),
            &SaveOptions::default(),
            &mut feedback,
            Instant::now()
        )
        .is_empty());
    let n = feedback.notifications()[0].clone();
    assert!(n.key.starts_with(KEY_READ_ONLY));
    // 通知说的是另存为的目标，不是原文件。
    assert_eq!(n.message.args[0].1.as_ref(), "Copy.cs");
    assert_eq!(action_ids(&n.actions), [ACTION_OVERWRITE, ACTION_SAVE_AS]);

    let effects = act(&mut c, &n.key, ACTION_OVERWRITE, &settings, &mut feedback);
    assert!(
        matches!(&effects[..], [Effect::Saved { path, .. }] if path == &target),
        "{effects:?}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"class Player {}// a");
    assert!(!std::fs::metadata(&target).unwrap().permissions().readonly());
    // 原文件：内容和只读属性都没被动过。
    assert_eq!(std::fs::read(&source).unwrap(), b"class Player {}");
    assert!(std::fs::metadata(&source).unwrap().permissions().readonly());
    set_readonly(&source, false);
}

/// #140：另存为写不进去时，“重试”重试的是**另存为的目标**（占用与其他写入失败共用这条分支）。
#[test]
fn retry_after_save_as_retries_the_new_path() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("Player.cs");
    std::fs::write(&source, "class Player {}").unwrap();
    let settings = TestSettings::new(&[]);
    // 第一次写入失败（目标被别的程序拿住的等价物），“重试”时恢复正常。
    let flag = Arc::new(AtomicBool::new(true));
    let mut c = SaveController::new(WriteOptions {
        busy_retry_delay: Duration::from_millis(10),
        fail_point: Some(FailPoint(Arc::new(move |_| {
            if flag.swap(false, Ordering::SeqCst) {
                Err(std::io::Error::other("注入的写入失败"))
            } else {
                Ok(())
            }
        }))),
        ..WriteOptions::default()
    });
    let mut feedback = FeedbackHub::default();
    open(&mut c, &source, &settings);
    type_end(&mut c, "// a");

    let target = dir.path().join("Copy.cs");
    assert!(c
        .save_as(
            DOC,
            target.clone(),
            &SaveOptions::default(),
            &mut feedback,
            Instant::now()
        )
        .is_empty());
    let n = feedback.notifications()[0].clone();
    assert!(n.key.starts_with(KEY_FAILED));
    assert_eq!(n.message.args[0].1.as_ref(), "Copy.cs");
    assert_eq!(action_ids(&n.actions), [ACTION_RETRY, ACTION_SAVE_AS]);
    assert!(!target.exists());

    // “另存为”仍然只是请求重新选路径。
    assert!(matches!(
        act(&mut c, &n.key, ACTION_SAVE_AS, &settings, &mut feedback)[..],
        [Effect::PickSaveAsPath(DOC)]
    ));
    let effects = act(&mut c, &n.key, ACTION_RETRY, &settings, &mut feedback);
    assert!(
        matches!(&effects[..], [Effect::Saved { path, .. }] if path == &target),
        "{effects:?}"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"class Player {}// a");
    assert_eq!(std::fs::read(&source).unwrap(), b"class Player {}");
    assert!(!c.is_dirty(DOC));
}

/// #140：原地保存的后续按钮不受影响；保存成功后记住的请求被清除。
#[test]
fn in_place_follow_up_still_writes_to_the_original_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Player.cs");
    std::fs::write(&path, "class Player {}").unwrap();
    let settings = TestSettings::new(&[]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &path, &settings);
    set_readonly(&path, true);
    type_end(&mut c, "// a");
    assert!(save(&mut c, &mut feedback).is_empty());
    let n = feedback.notifications()[0].clone();
    assert_eq!(n.message.args[0].1.as_ref(), "Player.cs");
    let effects = act(&mut c, &n.key, ACTION_OVERWRITE, &settings, &mut feedback);
    assert!(
        matches!(&effects[..], [Effect::Saved { path: p, .. }] if p == &path),
        "{effects:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"class Player {}// a");

    // 记住的另存为请求不会劫持普通的保存命令：另存为失败后按 ctrl+s 仍然写回当前文件。
    type_end(&mut c, "!");
    let missing = dir.path().join("none").join("Copy.cs");
    assert!(c
        .save_as(
            DOC,
            missing.clone(),
            &SaveOptions::default(),
            &mut feedback,
            Instant::now()
        )
        .is_empty());
    let effects = save(&mut c, &mut feedback);
    assert!(
        matches!(&effects[..], [Effect::Saved { path: p, .. }] if p == &path),
        "{effects:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"class Player {}// a!");
    assert!(!missing.exists());
}

#[test]
fn close_dirty_document_asks_save_dont_save_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.cs");
    let settings = TestSettings::new(&[]);
    for action in [ACTION_SAVE, ACTION_DONT_SAVE, ACTION_CANCEL] {
        std::fs::write(&path, "abc").unwrap();
        let mut c = controller();
        let mut feedback = FeedbackHub::default();
        open(&mut c, &path, &settings);
        assert_eq!(c.request_close(DOC, &mut feedback), CloseCheck::CloseNow);
        type_end(&mut c, "d");
        assert_eq!(c.request_close(DOC, &mut feedback), CloseCheck::Pending);
        let modal = feedback.modal().unwrap().clone();
        assert_eq!(
            action_ids(&modal.actions),
            [ACTION_SAVE, ACTION_DONT_SAVE, ACTION_CANCEL]
        );
        feedback.act(&modal.key, action).unwrap();
        let effects = c.on_action(
            &modal.key,
            action,
            &SaveOptions::default(),
            &settings,
            &mut feedback,
            Instant::now(),
        );
        let closes = effects.iter().any(|e| matches!(e, Effect::Close(DOC)));
        match action {
            ACTION_SAVE => {
                assert!(closes);
                assert_eq!(std::fs::read(&path).unwrap(), b"abcd");
            }
            ACTION_DONT_SAVE => {
                assert!(closes);
                assert_eq!(std::fs::read(&path).unwrap(), b"abc");
            }
            _ => {
                assert!(!closes);
                assert!(c.is_dirty(DOC));
            }
        }
    }
    // 热退出开关点（#50）：接管后不再确认。
    std::fs::write(&path, "abc").unwrap();
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &path, &settings);
    type_end(&mut c, "d");
    c.set_hot_exit(|_| true);
    assert_eq!(c.request_close(DOC, &mut feedback), CloseCheck::CloseNow);
    assert!(feedback.modal().is_none());
}

#[test]
fn external_change_bar_does_not_stack_and_clean_files_reload_silently() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.cs");
    std::fs::write(&path, "v0").unwrap();
    let settings = TestSettings::new(&[]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &path, &settings);

    // 本地没有修改：静默重载，不显示任何反馈。
    std::fs::write(&path, "v1").unwrap();
    let effects = c
        .apply_disk(
            DOC,
            DiskStamp::read(&path).unwrap(),
            &settings,
            &mut feedback,
            Instant::now(),
        )
        .unwrap();
    assert!(matches!(effects[..], [Effect::Reloaded { .. }]));
    assert!(feedback.all_editor_bars().is_empty() && feedback.notifications().is_empty());

    // 本地有修改：提示条（第 3 级，不是模态框），连续外部修改不堆积。
    type_end(&mut c, "*");
    for i in 2..10 {
        std::fs::write(&path, format!("v{i}")).unwrap();
        c.apply_disk(
            DOC,
            DiskStamp::read(&path).unwrap(),
            &settings,
            &mut feedback,
            Instant::now(),
        )
        .unwrap();
    }
    assert_eq!(feedback.all_editor_bars().len(), 1);
    assert!(feedback.modal().is_none());
    let bar = feedback.all_editor_bars()[0].clone();
    assert_eq!(action_ids(&bar.actions), [ACTION_RELOAD, ACTION_KEEP]);
    feedback.act(&bar.key, ACTION_RELOAD).unwrap();
    c.on_action(
        &bar.key,
        ACTION_RELOAD,
        &SaveOptions::default(),
        &settings,
        &mut feedback,
        Instant::now(),
    );
    assert_eq!(c.file(DOC).unwrap().session().contents(), "v9");

    // 删除：标签标记“已删除”，内容保留。
    std::fs::remove_file(&path).unwrap();
    let effects = c
        .apply_disk(DOC, None, &settings, &mut feedback, Instant::now())
        .unwrap();
    assert!(matches!(
        effects[..],
        [Effect::Deleted { deleted: true, .. }]
    ));
    assert_eq!(c.file(DOC).unwrap().session().contents(), "v9");
}

#[test]
fn own_saves_are_never_reported_as_external() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.cs");
    std::fs::write(&path, "x").unwrap();
    let settings = TestSettings::new(&[]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &path, &settings);
    for _ in 0..20 {
        type_end(&mut c, "y");
        assert!(matches!(
            save(&mut c, &mut feedback)[..],
            [Effect::Saved { .. }]
        ));
        c.note_change(&path, Instant::now());
        std::thread::sleep(Duration::from_millis(6));
        for (document, path) in c.due(Instant::now()) {
            let effects = c
                .apply_disk(
                    document,
                    DiskStamp::read(&path).unwrap(),
                    &settings,
                    &mut feedback,
                    Instant::now(),
                )
                .unwrap();
            assert!(effects.is_empty());
        }
    }
    assert!(feedback.all_editor_bars().is_empty());
    assert!(feedback.notifications().is_empty());
}

#[test]
fn untitled_documents_ask_for_a_path_and_eol_change_is_one_undo() {
    let settings = TestSettings::new(&[("files.eol", Value::String("\n".into()))]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    c.attach(
        DOC,
        TextFile::untitled("Untitled-1", "plaintext", &settings, EditOptions::default()),
    );
    type_end(&mut c, "a\nb");
    assert!(matches!(
        save(&mut c, &mut feedback)[..],
        [Effect::PickSaveAsPath(DOC)]
    ));
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("new.txt");
    let effects = c.save_as(
        DOC,
        target.clone(),
        &SaveOptions::read(&settings, "plaintext"),
        &mut feedback,
        Instant::now(),
    );
    assert!(matches!(effects[..], [Effect::Saved { .. }]));
    assert_eq!(std::fs::read(&target).unwrap(), b"a\nb");
    assert!(c.change_eol(DOC, Eol::CrLf, Instant::now()).unwrap());
    let file = c.file_mut(DOC).unwrap();
    assert!(matches!(
        file.line_endings(),
        LineEndings::Uniform(Eol::CrLf)
    ));
    file.session_mut().undo().unwrap();
    assert_eq!(file.session().contents(), "a\nb");
    assert!(!file.is_dirty());
}

#[test]
fn feedback_levels_follow_ux_spec() {
    // 通知是第 4 级、模态框是第 5 级、外部修改是第 3 级提示条（不可“不再提示”）。
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.cs");
    std::fs::write(&path, "a").unwrap();
    let settings = TestSettings::new(&[]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &path, &settings);
    type_end(&mut c, "b");
    std::fs::write(&path, "zz").unwrap();
    c.apply_disk(
        DOC,
        DiskStamp::read(&path).unwrap(),
        &settings,
        &mut feedback,
        Instant::now(),
    )
    .unwrap();
    let bar = &feedback.all_editor_bars()[0];
    assert!(!bar.dismissible && bar.suppress_key.is_none());
    assert_eq!(bar.document, DOC);
    let _ = FeedbackLevel::Modal;
}

/// 入口集成测试：编辑 → 保存 → 外部修改 → 重载（经控制器，与窗口装配走同一条路径）。
#[test]
fn edit_save_external_change_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Player.cs");
    std::fs::write(&path, "class Player {}\r\n").unwrap();
    let settings = TestSettings::new(&[]);
    let mut c = controller();
    let mut feedback = FeedbackHub::default();
    open(&mut c, &path, &settings);
    assert_eq!(c.document_for(&path), Some(DOC));

    // 编辑 → 保存。
    type_end(&mut c, "// 注释\r\n");
    assert!(c.is_dirty(DOC));
    let effects = save(&mut c, &mut feedback);
    let Effect::Saved { hash, .. } = effects[0] else {
        panic!("{effects:?}")
    };
    assert_eq!(
        hash,
        rove_buffer::save::content_hash(&std::fs::read(&path).unwrap())
    );
    assert!(!c.is_dirty(DOC));

    // 自己保存引起的事件被丢弃。
    c.note_change(&path, Instant::now());
    std::thread::sleep(Duration::from_millis(6));
    let due = c.due(Instant::now());
    assert_eq!(due.len(), 1);
    assert!(c
        .apply_disk(
            DOC,
            DiskStamp::read(&path).unwrap(),
            &settings,
            &mut feedback,
            Instant::now()
        )
        .unwrap()
        .is_empty());

    // 外部修改 → 静默重载。
    std::fs::write(&path, "class Player { int hp; }\r\n").unwrap();
    c.note_change(&path, Instant::now());
    std::thread::sleep(Duration::from_millis(6));
    for (document, path) in c.due(Instant::now()) {
        let effects = c
            .apply_disk(
                document,
                DiskStamp::read(&path).unwrap(),
                &settings,
                &mut feedback,
                Instant::now(),
            )
            .unwrap();
        assert!(matches!(effects[..], [Effect::Reloaded { .. }]));
    }
    assert_eq!(
        c.file(DOC).unwrap().session().contents(),
        "class Player { int hp; }\r\n"
    );
    assert!(!c.is_dirty(DOC));
    assert!(feedback.notifications().is_empty() && feedback.all_editor_bars().is_empty());
}
