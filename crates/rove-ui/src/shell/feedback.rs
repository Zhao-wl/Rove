//! 五级反馈的正式实现（#16，UX_SPEC §6.1）。
//!
//! **级别由反馈层决定**：调用方用 [`FeedbackHub::report`] 给出“原因”（[`Reason`]），
//! 由 [`level_for`] 按 §6.1 的表映射到级别；契约里的 `FeedbackService::show` 仍可直接传级别，
//! 但模态框只接受经 `report` 声明为“数据丢失 / 安全确认”的 key，其余一律拒绝（`InvalidInput`）。
//!
//! 本模块不依赖 GPUI：状态由 UI 线程持有并在渲染时读取；计时（状态栏消息 3 秒、
//! 通知 toast 自动隐藏）由 UI 层按 `generation` 回调 [`FeedbackHub::expire_status`] 等方法。
pub use crate::feedback::Severity;
use crate::feedback::{Feedback, FeedbackAction, FeedbackEvent, FeedbackLevel, FeedbackService};
use rove_base::*;
use rove_i18n::Message;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

/// 状态栏临时消息的默认显示时长（UX_SPEC §6.1：3 秒后消失）。
pub const STATUS_DURATION_MS: u32 = 3000;
/// 没有按钮的信息 / 警告通知，toast 在这段时间后收起（仍保留在通知中心），与 VS Code 一致。
pub const TOAST_AUTO_HIDE_MS: u32 = 10_000;
/// 编辑器提示条“不再提示”按钮发出的动作 ID。
pub const NEVER_SHOW_AGAIN: &str = "feedback.neverShowAgain";

/// 调用方描述“发生了什么”，不自己挑级别。
#[derive(Clone, Debug)]
pub enum Reason {
    /// 操作成功的回执、不需要用户处理的提示 → 1 状态栏临时消息。
    Receipt,
    /// 正常的后台工作（索引、扫描、预读）→ 永远只走状态栏，绝不弹通知。
    Background,
    /// 当前操作没有结果 → 2 光标处提示。
    NoResult {
        document: DocumentId,
        position: Position,
    },
    /// 与当前文件相关的背景信息 → 3 编辑器顶部提示条；`reason` 是“不再提示”的去重键。
    FileContext {
        document: DocumentId,
        reason: Arc<str>,
    },
    /// 需要用户知道或处理的全局事件 → 4 通知。
    Global(Severity),
    /// 不立即处理就会丢失数据 → 5 模态框。
    DataLoss,
    /// 与 VS Code 一致的安全确认（例如打开外部链接）→ 5 模态框。
    SecurityConfirmation,
}

/// §6.1 的分级表。
pub fn level_for(reason: &Reason) -> FeedbackLevel {
    match reason {
        Reason::Receipt | Reason::Background => FeedbackLevel::Status {
            duration_ms: STATUS_DURATION_MS,
        },
        Reason::NoResult { document, position } => FeedbackLevel::Cursor {
            document: *document,
            position: *position,
        },
        Reason::FileContext { document, reason } => FeedbackLevel::EditorBar {
            document: *document,
            dismissible: true,
            suppress_key: Some(reason.clone()),
        },
        Reason::Global(severity) => FeedbackLevel::Notification {
            severity: severity.clone(),
        },
        Reason::DataLoss | Reason::SecurityConfirmation => FeedbackLevel::Modal,
    }
}

#[derive(Clone, Debug)]
pub struct StatusMessage {
    pub key: Arc<str>,
    pub message: Message,
    pub generation: u64,
    pub duration_ms: u32,
}
#[derive(Clone, Debug)]
pub struct CursorHint {
    pub key: Arc<str>,
    pub document: DocumentId,
    pub position: Position,
    pub message: Message,
}
#[derive(Clone, Debug)]
pub struct EditorBar {
    pub key: Arc<str>,
    pub document: DocumentId,
    pub message: Message,
    pub actions: Arc<[FeedbackAction]>,
    pub dismissible: bool,
    pub suppress_key: Option<Arc<str>>,
}
#[derive(Clone, Debug)]
pub struct Notification {
    pub key: Arc<str>,
    pub severity: Severity,
    pub message: Message,
    pub actions: Arc<[FeedbackAction]>,
    /// 同一原因触发的次数；大于 1 时界面显示次数。
    pub count: u32,
    /// 是否作为右下角 toast 弹出（勿扰模式下只有错误级会弹出）。
    pub toast: bool,
    pub expanded: bool,
    /// 每次显示 / 更新递增，供 toast 自动隐藏计时判断是否过期。
    pub generation: u64,
}
#[derive(Clone, Debug)]
pub struct Modal {
    pub key: Arc<str>,
    pub message: Message,
    pub actions: Arc<[FeedbackAction]>,
}

type Listener = Arc<dyn Fn(FeedbackEvent) + Send + Sync>;
#[derive(Default)]
struct Bus {
    listeners: Arc<Mutex<(u64, BTreeMap<u64, Listener>)>>,
}
impl Bus {
    fn emit(&self, event: FeedbackEvent) {
        let listeners: Vec<_> = self.listeners.lock().unwrap().1.values().cloned().collect();
        for listener in listeners {
            listener(event.clone());
        }
    }
}
impl Events<FeedbackEvent> for Bus {
    fn subscribe(&self, listener: Listener) -> Subscription {
        let mut state = self.listeners.lock().unwrap();
        state.0 += 1;
        let id = state.0;
        state.1.insert(id, listener);
        let weak = Arc::downgrade(&self.listeners);
        Subscription::new(move || {
            if let Some(state) = weak.upgrade() {
                state.lock().unwrap().1.remove(&id);
            }
        })
    }
}

/// 窗口拥有的反馈状态；只由 UI 线程调用。
#[derive(Default)]
pub struct FeedbackHub {
    do_not_disturb: bool,
    generation: u64,
    status: Option<StatusMessage>,
    cursor: Option<CursorHint>,
    bars: Vec<EditorBar>,
    suppressed: BTreeSet<Arc<str>>,
    /// 新的在前。
    notifications: Vec<Notification>,
    center_visible: bool,
    modal: Option<Modal>,
    queued_modals: Vec<Modal>,
    modal_keys: BTreeSet<Arc<str>>,
    events: Bus,
}

impl FeedbackHub {
    /// 推荐入口：给出原因，由反馈层决定级别。
    pub fn report(
        &mut self,
        key: impl Into<Arc<str>>,
        reason: Reason,
        message: Message,
        actions: Arc<[FeedbackAction]>,
    ) -> Result<()> {
        let key = key.into();
        if matches!(reason, Reason::DataLoss | Reason::SecurityConfirmation) {
            self.modal_keys.insert(key.clone());
        }
        self.show(Feedback {
            key,
            level: level_for(&reason),
            message,
            actions,
        })
    }

    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    pub fn do_not_disturb(&self) -> bool {
        self.do_not_disturb
    }
    /// 勿扰模式：开启后只弹出错误级通知；已有的非错误 toast 立即收起（仍在通知中心里）。
    pub fn set_do_not_disturb(&mut self, enabled: bool) {
        self.do_not_disturb = enabled;
        if enabled {
            for n in &mut self.notifications {
                if !matches!(n.severity, Severity::Error) {
                    n.toast = false;
                }
            }
        }
    }

    pub fn status(&self) -> Option<&StatusMessage> {
        self.status.as_ref()
    }
    /// 计时结束：只清除同一代的消息，较新的消息不受影响。
    pub fn expire_status(&mut self, generation: u64) -> bool {
        if self.status.as_ref().map(|s| s.generation) == Some(generation) {
            self.status = None;
            return true;
        }
        false
    }
    pub fn cursor_hint(&self) -> Option<&CursorHint> {
        self.cursor.as_ref()
    }
    /// 光标移动、输入或切换编辑器时调用，与 VS Code 的 MessageController 相同。
    pub fn clear_cursor_hint(&mut self) {
        self.cursor = None;
    }
    /// 某个文档的提示条（按到达顺序）。
    pub fn editor_bars(&self, document: DocumentId) -> impl Iterator<Item = &EditorBar> {
        self.bars.iter().filter(move |b| b.document == document)
    }
    pub fn all_editor_bars(&self) -> &[EditorBar] {
        &self.bars
    }
    pub fn suppressed(&self) -> impl Iterator<Item = &Arc<str>> {
        self.suppressed.iter()
    }
    /// 恢复持久化的“不再提示”记录（启动时由布局存储加载）。
    pub fn restore_suppressed(&mut self, keys: impl IntoIterator<Item = Arc<str>>) {
        self.suppressed.extend(keys);
    }
    /// 提示条的“不再提示”：记录原因、关闭提示条并通知调用方。
    pub fn never_show_again(&mut self, key: &str) -> Result<()> {
        let index = self
            .bars
            .iter()
            .position(|b| b.key.as_ref() == key)
            .ok_or(Error::NotFound)?;
        let bar = self.bars.remove(index);
        let reason = bar.suppress_key.clone().ok_or(Error::Unsupported)?;
        self.suppressed.insert(reason.clone());
        self.bars
            .retain(|b| b.suppress_key.as_ref() != Some(&reason));
        self.events.emit(FeedbackEvent::Action {
            key: bar.key,
            action: NEVER_SHOW_AGAIN.into(),
        });
        Ok(())
    }

    /// 通知中心里的全部通知（新的在前）。
    pub fn notifications(&self) -> &[Notification] {
        &self.notifications
    }
    /// 当前弹出的 toast（通知中心打开时不弹 toast，与 VS Code 一致）。
    pub fn toasts(&self) -> impl Iterator<Item = &Notification> {
        let hidden = self.center_visible;
        self.notifications
            .iter()
            .filter(move |n| n.toast && !hidden)
    }
    pub fn center_visible(&self) -> bool {
        self.center_visible
    }
    pub fn set_center_visible(&mut self, visible: bool) {
        self.center_visible = visible;
        if visible {
            for n in &mut self.notifications {
                n.toast = false;
            }
        }
    }
    pub fn hide_toasts(&mut self) {
        for n in &mut self.notifications {
            n.toast = false;
        }
    }
    /// toast 自动收起：只有同一代、没有按钮的非错误通知才收起。
    pub fn expire_toast(&mut self, key: &str, generation: u64) -> bool {
        match self
            .notifications
            .iter_mut()
            .find(|n| n.key.as_ref() == key && n.generation == generation)
        {
            Some(n) if n.actions.is_empty() && !matches!(n.severity, Severity::Error) => {
                let was = n.toast;
                n.toast = false;
                was
            }
            _ => false,
        }
    }
    pub fn toggle_expanded(&mut self, key: &str) -> Result<()> {
        let n = self
            .notifications
            .iter_mut()
            .find(|n| n.key.as_ref() == key)
            .ok_or(Error::NotFound)?;
        n.expanded = !n.expanded;
        Ok(())
    }
    pub fn set_expanded(&mut self, key: &str, expanded: bool) -> Result<()> {
        let n = self
            .notifications
            .iter_mut()
            .find(|n| n.key.as_ref() == key)
            .ok_or(Error::NotFound)?;
        n.expanded = expanded;
        Ok(())
    }
    pub fn clear_all_notifications(&mut self) {
        for n in std::mem::take(&mut self.notifications) {
            self.events.emit(FeedbackEvent::Dismissed { key: n.key });
        }
    }

    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    /// 用户点击了某个按钮（通知、提示条或模态框）：发出事件并关闭该反馈。
    pub fn act(&mut self, key: &str, action: &str) -> Result<()> {
        if action == NEVER_SHOW_AGAIN {
            return self.never_show_again(key);
        }
        let known = |actions: &[FeedbackAction]| actions.iter().any(|a| a.id.as_ref() == action);
        let found = if let Some(m) = self.modal.as_ref().filter(|m| m.key.as_ref() == key) {
            known(&m.actions).then(|| {
                self.modal = None;
                self.advance_modal();
            })
        } else if let Some(i) = self
            .notifications
            .iter()
            .position(|n| n.key.as_ref() == key)
        {
            known(&self.notifications[i].actions).then(|| {
                self.notifications.remove(i);
            })
        } else if let Some(i) = self.bars.iter().position(|b| b.key.as_ref() == key) {
            known(&self.bars[i].actions).then(|| {
                self.bars.remove(i);
            })
        } else {
            return Err(Error::NotFound);
        };
        found.ok_or(Error::InvalidInput)?;
        self.events.emit(FeedbackEvent::Action {
            key: key.into(),
            action: action.into(),
        });
        Ok(())
    }

    fn advance_modal(&mut self) {
        if self.modal.is_none() && !self.queued_modals.is_empty() {
            self.modal = Some(self.queued_modals.remove(0));
        }
    }
}

impl FeedbackService for FeedbackHub {
    fn show(&mut self, f: Feedback) -> Result<()> {
        if f.key.is_empty() {
            return Err(Error::InvalidInput);
        }
        let generation = self.next_generation();
        match f.level {
            FeedbackLevel::Status { duration_ms } => {
                self.status = Some(StatusMessage {
                    key: f.key,
                    message: f.message,
                    generation,
                    duration_ms,
                });
            }
            FeedbackLevel::Cursor { document, position } => {
                self.cursor = Some(CursorHint {
                    key: f.key,
                    document,
                    position,
                    message: f.message,
                });
            }
            FeedbackLevel::EditorBar {
                document,
                dismissible,
                suppress_key,
            } => {
                if suppress_key
                    .as_ref()
                    .is_some_and(|k| self.suppressed.contains(k))
                {
                    // 用户选过“不再提示”：静默不显示，这是用户的选择而不是失败。
                    return Ok(());
                }
                // 按“文件 + 原因”去重：同一文件同一原因只保留一条，更新内容。
                let reason = suppress_key.clone().unwrap_or_else(|| f.key.clone());
                self.bars.retain(|b| {
                    !(b.document == document
                        && b.suppress_key.as_ref().unwrap_or(&b.key) == &reason)
                        && b.key != f.key
                });
                self.bars.push(EditorBar {
                    key: f.key,
                    document,
                    message: f.message,
                    actions: f.actions,
                    dismissible,
                    suppress_key,
                });
            }
            FeedbackLevel::Notification { severity } => {
                let toast = (!self.do_not_disturb || matches!(severity, Severity::Error))
                    && !self.center_visible;
                if let Some(i) = self.notifications.iter().position(|n| n.key == f.key) {
                    // 同一原因不重复弹出：更新次数与内容，移到最前。
                    let mut n = self.notifications.remove(i);
                    n.count += 1;
                    n.message = f.message;
                    n.actions = f.actions;
                    n.severity = severity;
                    n.toast = toast;
                    n.generation = generation;
                    self.notifications.insert(0, n);
                } else {
                    self.notifications.insert(
                        0,
                        Notification {
                            key: f.key,
                            severity,
                            message: f.message,
                            actions: f.actions,
                            count: 1,
                            toast,
                            expanded: false,
                            generation,
                        },
                    );
                }
            }
            FeedbackLevel::Modal => {
                if !self.modal_keys.contains(&f.key) {
                    // 模态框只用于数据丢失与安全确认；必须经 report(DataLoss/SecurityConfirmation) 声明。
                    return Err(Error::InvalidInput);
                }
                let modal = Modal {
                    key: f.key,
                    message: f.message,
                    actions: f.actions,
                };
                if self.modal.as_ref().is_some_and(|m| m.key == modal.key) {
                    self.modal = Some(modal);
                } else if self.modal.is_some() {
                    self.queued_modals.retain(|m| m.key != modal.key);
                    self.queued_modals.push(modal);
                } else {
                    self.modal = Some(modal);
                }
            }
        }
        Ok(())
    }

    fn dismiss(&mut self, key: &str) -> Result<()> {
        let mut found = false;
        if self.status.as_ref().is_some_and(|s| s.key.as_ref() == key) {
            self.status = None;
            found = true;
        }
        if self.cursor.as_ref().is_some_and(|s| s.key.as_ref() == key) {
            self.cursor = None;
            found = true;
        }
        let before = self.bars.len() + self.notifications.len() + self.queued_modals.len();
        self.bars.retain(|b| b.key.as_ref() != key);
        self.notifications.retain(|n| n.key.as_ref() != key);
        self.queued_modals.retain(|m| m.key.as_ref() != key);
        found |= before != self.bars.len() + self.notifications.len() + self.queued_modals.len();
        if self.modal.as_ref().is_some_and(|m| m.key.as_ref() == key) {
            self.modal = None;
            self.advance_modal();
            found = true;
        }
        if !found {
            return Err(Error::NotFound);
        }
        // Dismissed 表示“取消 / 关闭”，绝不代表接受了任何选项。
        self.events
            .emit(FeedbackEvent::Dismissed { key: key.into() });
        Ok(())
    }

    fn events(&self) -> &dyn Events<FeedbackEvent> {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(key: &str) -> Message {
        Message::key(key)
    }
    fn none() -> Arc<[FeedbackAction]> {
        Arc::from([])
    }

    #[test]
    fn level_is_chosen_by_reason_not_by_caller() {
        assert!(matches!(
            level_for(&Reason::Receipt),
            FeedbackLevel::Status { duration_ms: 3000 }
        ));
        assert!(matches!(
            level_for(&Reason::Background),
            FeedbackLevel::Status { .. }
        ));
        assert!(matches!(
            level_for(&Reason::Global(Severity::Warning)),
            FeedbackLevel::Notification {
                severity: Severity::Warning
            }
        ));
        assert!(matches!(level_for(&Reason::DataLoss), FeedbackLevel::Modal));
    }

    #[test]
    fn same_cause_updates_count_instead_of_new_notification() {
        let mut hub = FeedbackHub::default();
        for _ in 0..3 {
            hub.report(
                "index.failed",
                Reason::Global(Severity::Error),
                msg("x"),
                none(),
            )
            .unwrap();
        }
        assert_eq!(hub.notifications().len(), 1);
        assert_eq!(hub.notifications()[0].count, 3);
    }

    #[test]
    fn modal_requires_data_loss_reason() {
        let mut hub = FeedbackHub::default();
        let direct = Feedback {
            key: "close.dirty".into(),
            level: FeedbackLevel::Modal,
            message: msg("x"),
            actions: none(),
        };
        assert_eq!(hub.show(direct.clone()), Err(Error::InvalidInput));
        assert!(hub.modal().is_none());
        hub.report("close.dirty", Reason::DataLoss, msg("x"), none())
            .unwrap();
        assert!(hub.modal().is_some());
        // 声明过的 key 之后也可以直接 show（例如更新内容）。
        hub.show(direct).unwrap();
        hub.dismiss("close.dirty").unwrap();
        assert!(hub.modal().is_none());
    }

    #[test]
    fn status_expiry_only_clears_its_generation() {
        let mut hub = FeedbackHub::default();
        hub.report("a", Reason::Receipt, msg("a"), none()).unwrap();
        let first = hub.status().unwrap().generation;
        hub.report("b", Reason::Receipt, msg("b"), none()).unwrap();
        assert!(!hub.expire_status(first));
        assert_eq!(hub.status().unwrap().key.as_ref(), "b");
        let second = hub.status().unwrap().generation;
        assert!(hub.expire_status(second));
        assert!(hub.status().is_none());
    }

    #[test]
    fn events_report_actions_and_dismissals() {
        let mut hub = FeedbackHub::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let _sub = hub.events().subscribe(Arc::new(move |e| {
            sink.lock().unwrap().push(match e {
                FeedbackEvent::Action { action, .. } => action.to_string(),
                FeedbackEvent::Dismissed { key } => format!("dismissed:{key}"),
            })
        }));
        let retry: Arc<[FeedbackAction]> = Arc::from([FeedbackAction {
            id: "retry".into(),
            label: msg("common.retry"),
        }]);
        hub.report("n", Reason::Global(Severity::Error), msg("x"), retry)
            .unwrap();
        assert_eq!(hub.act("n", "nope"), Err(Error::InvalidInput));
        hub.act("n", "retry").unwrap();
        hub.report("m", Reason::Global(Severity::Info), msg("x"), none())
            .unwrap();
        hub.dismiss("m").unwrap();
        assert_eq!(hub.dismiss("m"), Err(Error::NotFound));
        assert_eq!(*seen.lock().unwrap(), vec!["retry", "dismissed:m"]);
    }
}
