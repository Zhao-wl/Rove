//! #59 的窗口装配：C# 语义着色与不生效预处理区域置灰（D9）。
//!
//! - **开关**：`editor.semanticHighlighting.enabled` 三个取值照搬 VS Code
//!   （`true` / `false` / `configuredByTheme`，默认后者），按 `[csharp]` 语言作用域读取；
//!   `configuredByTheme` 时取主题的 `semanticHighlighting` 字段（#10 解析，已含
//!   `editor.semanticTokenColorCustomizations.enabled` 与旧写法的覆盖）。
//!   **关闭时一次查询都不发**（[`Workbench::semantic_query_count`] 可断言）。
//! - **查询**一律经 #1 契约的 `CsQueries`（#38 的 `CsRouter`）在后台执行器上执行，
//!   结果按「请求代次 + 文档版本」回到窗口，过期结果丢弃。
//! - **着色**：token 类型 + 修饰符 → 主题的 `semanticTokenColors` → 回退表 → `tokenColors`
//!   （全部在 #10 的 `ResolvedTheme` 里，本文件只负责把结果交给代码视图）。
//! - **降级**：`Approximate` 且原因是 `ModifiedRegion` 时仍然采用（#38 已经把改过的行剔掉了）；
//!   其余近似原因（索引未就绪、文件不在索引里）与 `Syntax` 一律不产出语义 token，**也不加提示**。
//! - 精确度变化（索引建好、保存后重建完成）由 [`Workbench::refresh_semantic`] 静默重查，
//!   只让被覆盖的行失效，不闪烁、不弹提示。

use super::*;
use crate::code_view::semantic::{SemanticStyles, LANGUAGE};
use gpui::EntityId;
use rove_lang_cs::query::{ApproximationReason, CsQueries, Precision, SemanticToken};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 触发重新判定的设置键。
pub(super) const SETTING_KEYS: &[&str] = &[
    "editor.semanticHighlighting.enabled",
    "editor.maxTokenizationLineLength",
];

/// `editor.semanticHighlighting.enabled` 的三个取值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SemanticHighlighting {
    On,
    Off,
    /// 默认：由主题的 `semanticHighlighting` 字段决定。
    #[default]
    ConfiguredByTheme,
}

impl SemanticHighlighting {
    pub(super) fn read(settings: &dyn Settings) -> Self {
        let scope = Scope {
            language: Some(Arc::from(LANGUAGE)),
        };
        match settings.read("editor.semanticHighlighting.enabled", &scope) {
            Ok(Value::Bool(true)) => Self::On,
            Ok(Value::Bool(false)) => Self::Off,
            Ok(Value::String(s)) if &*s == "true" => Self::On,
            Ok(Value::String(s)) if &*s == "false" => Self::Off,
            _ => Self::ConfiguredByTheme,
        }
    }

    /// 最终是否开启（`configuredByTheme` 时看主题）。
    pub(super) fn resolve(self, theme_says: bool) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
            Self::ConfiguredByTheme => theme_says,
        }
    }
}

/// 最近一次采用的结果：(文档, 版本, token, 当时失效的行)。
type LastResult = (DocumentId, Version, Arc<[SemanticToken]>, Vec<Range<u64>>);

/// 一个代码视图槽位上的语义状态。
#[derive(Default)]
struct SemanticSlot {
    owner: Option<(EntityId, DocumentId)>,
    /// 请求代次：文档换了、版本变了、索引换代了都会作废旧请求。
    epoch: u64,
    /// 已经发出过请求的 (文档, 版本, 刷新代次)。**每帧都会走一遍 `refresh_semantic`
    /// （`render` → `sync_views`），靠它去重，否则会每帧发一次查询。**
    requested: Option<(DocumentId, Version, u64)>,
    /// 最近一次采用的结果（主题换了只重新着色，不重查）。
    last: Option<LastResult>,
}

#[derive(Default)]
pub(super) struct SemanticState {
    setting: SemanticHighlighting,
    slots: HashMap<(EditorGroupId, bool), SemanticSlot>,
    queries: Arc<AtomicUsize>,
    /// 刷新代次：索引换代、设置变化、主题变化时 +1，让所有槽位重新请求一次。
    generation: u64,
}

impl SemanticState {
    pub(super) fn new(settings: &dyn Settings) -> Self {
        Self {
            setting: SemanticHighlighting::read(settings),
            ..Self::default()
        }
    }
}

impl Workbench {
    /// 已发起的语义查询次数（测试断言「关闭时一次查询都不发」）。
    pub fn semantic_query_count(&self) -> usize {
        self.semantic.queries.load(Ordering::SeqCst)
    }

    /// 当前设置 + 主题下语义高亮是否开启。
    pub fn semantic_highlighting_on(&self) -> bool {
        self.semantic.setting.resolve(
            self.theme_service
                .current()
                .semantic_highlighting_for_language(LANGUAGE),
        )
    }

    pub(super) fn semantic_settings_changed(
        &mut self,
        keys: &BTreeSet<Arc<str>>,
        cx: &mut Context<Self>,
    ) {
        if !keys.iter().any(|k| SETTING_KEYS.contains(&&**k)) {
            return;
        }
        self.semantic.setting = SemanticHighlighting::read(&*self.settings);
        self.semantic.generation += 1;
        self.refresh_semantic(cx);
    }

    /// 索引换代或状态变化（#36 / #38）：静默重查，不弹提示、不清空，结果到达后替换。
    pub(super) fn semantic_index_changed(&mut self, cx: &mut Context<Self>) {
        self.semantic.generation += 1;
        self.refresh_semantic(cx);
    }

    /// 主题换了：颜色立即更新，但**不重新查询**（token 没变，只是颜色映射换了）。
    pub(super) fn semantic_theme_changed(&mut self, cx: &mut Context<Self>) {
        if !self.semantic_highlighting_on() {
            // 新主题可能把语义高亮关掉了。
            self.semantic.generation += 1;
            self.refresh_semantic(cx);
            return;
        }
        self.refresh_semantic(cx);
        let theme = self.theme_service.current();
        let slots: Vec<(EditorGroupId, bool)> = self.semantic.slots.keys().copied().collect();
        for key in slots {
            let Some((document, version, tokens, _)) =
                self.semantic.slots.get(&key).and_then(|s| s.last.clone())
            else {
                continue;
            };
            if self
                .shown_document(key.0, key.1)
                .is_none_or(|shown| shown.id != document || shown.text.version() != version)
            {
                continue;
            }
            let Some(code) = self.group_code(key.0, key.1).cloned() else {
                continue;
            };
            let styles = SemanticStyles::build(&tokens, &|ty, mods| {
                theme.semantic_token_style(ty, mods, LANGUAGE)
            });
            code.update(cx, |c, cx| {
                c.set_semantic_styles(styles, cx);
            });
        }
        // 置灰的不透明度跟着主题走（代码视图在 `set_theme` 里重算调色板）。
    }

    /// 重新判定每个代码视图的语义着色：开关、语言、文档级别，必要时发起查询。
    /// 开关为假或文档不是 C# 时清空并**不发起查询**。
    pub(super) fn refresh_semantic(&mut self, cx: &mut Context<Self>) {
        let on = self.semantic_highlighting_on();
        let groups: Vec<(EditorGroupId, bool)> = self
            .views
            .iter()
            .flat_map(|(id, views)| {
                std::iter::once((*id, false))
                    .chain(views.secondary.is_some().then_some((*id, true)))
            })
            .collect();
        self.semantic.slots.retain(|key, _| groups.contains(key));
        for (group, secondary) in groups {
            let document = self.shown_document(group, secondary);
            let is_csharp = document
                .as_ref()
                .is_some_and(|d| &*d.metadata.language == LANGUAGE);
            let enabled = on && is_csharp;
            let Some(code) = self.group_code(group, secondary).cloned() else {
                continue;
            };
            code.update(cx, |c, cx| c.set_semantic_enabled(enabled, cx));
            let generation = self.semantic.generation;
            let slot = self.semantic.slots.entry((group, secondary)).or_default();
            if !enabled {
                slot.requested = None;
                slot.last = None;
                slot.epoch += 1;
                continue;
            }
            let Some(document) = document else {
                slot.requested = None;
                slot.last = None;
                slot.epoch += 1;
                continue;
            };
            if !code.read(cx).semantic_enabled() {
                // 大文件 / 超大文件：级别关闭，不查询（原因在 `disabled_features()` 里）。
                slot.requested = None;
                slot.last = None;
                slot.epoch += 1;
                continue;
            }
            // 每帧都会走到这里：同一份（文档, 版本, 刷新代次）只请求一次。
            let key = (document.id, document.text.version(), generation);
            let view_id = code.entity_id();
            if slot.owner != Some((view_id, document.id)) {
                code.update(cx, |c, cx| {
                    c.set_semantic_styles(SemanticStyles::default(), cx);
                    c.set_inactive_regions(Vec::new(), cx);
                });
                slot.owner = Some((view_id, document.id));
                slot.requested = None;
                slot.last = None;
            }
            if slot.requested == Some(key) {
                continue;
            }
            // The editor can adopt its editable snapshot before the navigation
            // router finishes registering that version. Query from the matched
            // buffer so a stale response cannot strand the view without colors.
            if code.read(cx).version() != key.1
                || matches!(&document.resource, Resource::Workspace(path)
                    if self.nav.router.buffers().get(path.as_str())
                        .is_none_or(|buffer| buffer.version != key.1))
            {
                continue;
            }
            slot.requested = Some(key);
            slot.epoch += 1;
            self.request_semantic(group, secondary, document, cx);
        }
    }

    fn request_semantic(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        document: Arc<Document>,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.semantic.slots.get(&(group, secondary)) else {
            return;
        };
        let epoch = slot.epoch;
        let owner = slot.owner;
        let key = (document.id, document.text.version());
        let resource = document.resource.clone();
        let router = self.cs_router().clone();
        self.semantic.queries.fetch_add(1, Ordering::SeqCst);
        let task = cx.background_executor().spawn(async move {
            let cancel = Cancellation::default();
            let tokens = router.semantic_tokens(&resource, key.1, &cancel);
            let inactive = router.inactive_regions(&resource, key.1, &cancel);
            (tokens, inactive)
        });
        cx.spawn(async move |this, cx| {
            let (tokens, inactive) = task.await;
            let _ = this.update(cx, |view, cx| {
                if view
                    .semantic
                    .slots
                    .get(&(group, secondary))
                    .is_none_or(|s| s.epoch != epoch || s.owner != owner)
                    || view
                        .shown_document(group, secondary)
                        .map(|d| (d.id, d.text.version()))
                        != Some(key)
                {
                    return; // 过期结果：丢弃，屏幕上留着旧样式（不闪烁）。
                }
                let Some(code) = view.group_code(group, secondary).cloned() else {
                    return;
                };
                if Some((code.entity_id(), key.0)) != owner || code.read(cx).version() != key.1 {
                    return;
                }
                let generation = view.cs_router().semantic_generation();
                if tokens
                    .as_ref()
                    .is_ok_and(|result| Some(result.index_generation) != generation)
                    || inactive
                        .as_ref()
                        .is_ok_and(|result| Some(result.index_generation) != generation)
                {
                    if let Some(slot) = view.semantic.slots.get_mut(&(group, secondary)) {
                        slot.requested = None;
                    }
                    cx.notify();
                    return;
                }
                // A stale query can race a mapped edit. A missing or disabled
                // index must instead fall back to syntax highlighting.
                if matches!(tokens, Err(rove_base::Error::Stale))
                    && matches!(inactive, Err(rove_base::Error::Stale))
                {
                    return;
                }
                let tokens: Arc<[SemanticToken]> = match tokens {
                    Ok(r) if usable(r.precision, r.reason.as_ref()) => r.value,
                    // 索引未就绪 / 近似：不产出语义 token，也不加任何提示。
                    _ => Arc::from(Vec::new()),
                };
                let inactive: Vec<Range<u64>> = match inactive {
                    Ok(r) if usable(r.precision, r.reason.as_ref()) => r
                        .value
                        .iter()
                        .map(|r| r.start().line.0..r.end().line.0.max(r.start().line.0 + 1))
                        .collect(),
                    _ => Vec::new(),
                };
                let theme = view.theme_service.current();
                let styles = SemanticStyles::build(&tokens, &|ty, mods| {
                    theme.semantic_token_style(ty, mods, LANGUAGE)
                });
                let invalid = code.update(cx, |c, cx| {
                    let mut invalid = c.set_semantic_styles(styles, cx);
                    invalid.extend(c.set_inactive_regions(inactive, cx));
                    invalid
                });
                if let Some(slot) = view.semantic.slots.get_mut(&(group, secondary)) {
                    slot.last = Some((key.0, key.1, tokens, invalid));
                }
            });
        })
        .detach();
    }

    /// 当前主题快照（语义着色的颜色解析入口；测试里用来独立算一遍期望颜色）。
    #[cfg(test)]
    pub(crate) fn resolved_theme(&self) -> Arc<rove_theme::ResolvedTheme> {
        self.theme_service.current()
    }

    /// 测试用：直接设定三值开关（设置文件的读取由 [`SemanticHighlighting::read`] 自己测）。
    #[cfg(test)]
    pub(crate) fn set_semantic_setting(
        &mut self,
        value: SemanticHighlighting,
        cx: &mut Context<Self>,
    ) {
        self.semantic.setting = value;
        self.semantic.generation += 1;
        self.refresh_semantic(cx);
    }

    /// 最近一次采用的结果里某个位置的语义 token（类型 + 修饰符）。
    #[cfg(test)]
    pub(crate) fn semantic_token_at(
        &self,
        group: EditorGroupId,
        secondary: bool,
        line: u32,
        column: u32,
    ) -> Option<(String, Vec<String>)> {
        let (_, _, tokens, _) = self
            .semantic
            .slots
            .get(&(group, secondary))?
            .last
            .as_ref()?;
        tokens
            .iter()
            .find(|t| t.line.0 == u64::from(line) && t.column.0 == u64::from(column))
            .map(|t| {
                (
                    t.token_type.to_string(),
                    t.modifiers.iter().map(|m| m.to_string()).collect(),
                )
            })
    }

    /// 最近一次采用的结果让哪些行失效（测试断言「只让被覆盖的行失效」）。
    #[cfg(test)]
    pub(crate) fn semantic_invalidated_lines(
        &self,
        group: EditorGroupId,
        secondary: bool,
    ) -> Vec<Range<u64>> {
        self.semantic
            .slots
            .get(&(group, secondary))
            .and_then(|s| s.last.as_ref())
            .map(|(_, _, _, lines)| lines.clone())
            .unwrap_or_default()
    }
}

/// 这份结果能不能用：精确可以用；近似只有「改过的区域」这一种原因可以用
/// （#38 已经把改过的行剔掉了，剩下的仍然是精确绑定）；语法层一律不用。
fn usable(precision: Precision, reason: Option<&ApproximationReason>) -> bool {
    match precision {
        Precision::Exact => true,
        Precision::Approximate => matches!(reason, Some(ApproximationReason::ModifiedRegion)),
        Precision::Syntax => false,
    }
}
