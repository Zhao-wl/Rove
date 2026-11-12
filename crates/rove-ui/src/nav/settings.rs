//! #40 读取的设置项（全部按 C# 的语言作用域读取，`[csharp]` 覆盖生效）。登记在 `rove_config` 的内置目录里。

use rove_base::Value;
use rove_config::{Scope, Settings};
use std::sync::Arc;

/// `editor.gotoLocation.multiple*` 的取值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GotoMode {
    /// 打开速览（#41），不跳转。VS Code 默认值。
    Peek,
    /// 跳到第一个结果并打开速览。
    GotoAndPeek,
    /// 跳到第一个结果，进入符号导航模式（`hasSymbols`：`f12` 循环、`escape` 取消）。
    Goto,
}

impl GotoMode {
    fn parse(v: &Value) -> Option<Self> {
        match v {
            Value::String(s) => match &**s {
                "peek" => Some(Self::Peek),
                "gotoAndPeek" => Some(Self::GotoAndPeek),
                "goto" => Some(Self::Goto),
                _ => None,
            },
            _ => None,
        }
    }
}

/// `editor.hover.enabled`：VS Code 新版为 `on` / `off` / `onKeyboardModifier`，旧版为布尔值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoverEnabled {
    On,
    Off,
    /// 只在按住修饰键（多光标修饰键为 Alt 时即 Ctrl）时显示。
    OnKeyboardModifier,
}

/// `editor.occurrencesHighlight`：`off` / `singleFile` / `multiFile`（旧版布尔值：true 即 singleFile）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occurrences {
    Off,
    SingleFile,
    /// 所有打开的编辑器里都高亮。Rove 的语义高亮按代码视图各自计算，效果相同。
    MultiFile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavSettings {
    pub multiple_definitions: GotoMode,
    pub multiple_declarations: GotoMode,
    pub multiple_implementations: GotoMode,
    pub multiple_references: GotoMode,
    pub definition_link_opens_in_peek: bool,
    pub hover_enabled: HoverEnabled,
    pub hover_delay_ms: u64,
    pub hover_sticky: bool,
    pub hover_hiding_delay_ms: u64,
    pub hover_above: bool,
    pub hover_long_line_warning: bool,
    pub max_tokenization_line_length: usize,
    pub occurrences: Occurrences,
    pub occurrences_delay_ms: u64,
    /// Rove 每种语言只有一个高亮提供方（C# 语义 / 其他文本），所以只记录，不改变行为。
    pub occurrences_from_all_providers: bool,
}

impl Default for NavSettings {
    /// VS Code 默认值。
    fn default() -> Self {
        Self {
            multiple_definitions: GotoMode::Peek,
            multiple_declarations: GotoMode::Peek,
            multiple_implementations: GotoMode::Peek,
            multiple_references: GotoMode::Peek,
            definition_link_opens_in_peek: false,
            hover_enabled: HoverEnabled::On,
            hover_delay_ms: 300,
            hover_sticky: true,
            hover_hiding_delay_ms: 300,
            hover_above: true,
            hover_long_line_warning: true,
            max_tokenization_line_length: 20000,
            occurrences: Occurrences::SingleFile,
            occurrences_delay_ms: 0,
            occurrences_from_all_providers: false,
        }
    }
}

/// 设置项的 ID（[`NavSettings::read`] 读取的全部键；变更时重读）。
pub const KEYS: &[&str] = &[
    "editor.gotoLocation.multiple",
    "editor.gotoLocation.multipleDefinitions",
    "editor.gotoLocation.multipleDeclarations",
    "editor.gotoLocation.multipleImplementations",
    "editor.gotoLocation.multipleReferences",
    "editor.definitionLinkOpensInPeek",
    "editor.hover.enabled",
    "editor.hover.delay",
    "editor.hover.sticky",
    "editor.hover.hidingDelay",
    "editor.hover.above",
    "editor.hover.showLongLineWarning",
    "editor.maxTokenizationLineLength",
    "editor.occurrencesHighlight",
    "editor.occurrencesHighlightDelay",
    "editor.occurrencesHighlightFromAllProviders",
];

impl NavSettings {
    /// 按语言作用域读取（未知键或类型不对时用 VS Code 默认值）。
    pub fn read(settings: &dyn Settings, language: Option<&str>) -> Self {
        let scope = Scope {
            language: language.map(Arc::from),
        };
        let raw = |key: &str| settings.read(key, &scope).unwrap_or(Value::Null);
        let d = Self::default();
        let boolean = |key: &str, default: bool| match raw(key) {
            Value::Bool(b) => b,
            _ => default,
        };
        let number = |key: &str, default: u64, max: f64| match raw(key) {
            Value::Number(n) if n.is_finite() => n.clamp(0.0, max) as u64,
            _ => default,
        };
        // 已弃用的 `editor.gotoLocation.multiple` 设置了就覆盖四个分项（VS Code 同样处理）。
        let legacy = GotoMode::parse(&raw("editor.gotoLocation.multiple"));
        let mode = |key: &str| {
            legacy
                .or_else(|| GotoMode::parse(&raw(key)))
                .unwrap_or(GotoMode::Peek)
        };
        let hover_enabled = match raw("editor.hover.enabled") {
            Value::Bool(false) => HoverEnabled::Off,
            Value::String(s) if &*s == "off" => HoverEnabled::Off,
            Value::String(s) if &*s == "onKeyboardModifier" => HoverEnabled::OnKeyboardModifier,
            _ => HoverEnabled::On,
        };
        let occurrences = match raw("editor.occurrencesHighlight") {
            Value::Bool(false) => Occurrences::Off,
            Value::String(s) if &*s == "off" => Occurrences::Off,
            Value::String(s) if &*s == "multiFile" => Occurrences::MultiFile,
            _ => Occurrences::SingleFile,
        };
        Self {
            multiple_definitions: mode("editor.gotoLocation.multipleDefinitions"),
            multiple_declarations: mode("editor.gotoLocation.multipleDeclarations"),
            multiple_implementations: mode("editor.gotoLocation.multipleImplementations"),
            multiple_references: mode("editor.gotoLocation.multipleReferences"),
            definition_link_opens_in_peek: boolean(
                "editor.definitionLinkOpensInPeek",
                d.definition_link_opens_in_peek,
            ),
            hover_enabled,
            hover_delay_ms: number("editor.hover.delay", d.hover_delay_ms, 10_000.0),
            hover_sticky: boolean("editor.hover.sticky", d.hover_sticky),
            hover_hiding_delay_ms: number(
                "editor.hover.hidingDelay",
                d.hover_hiding_delay_ms,
                10_000.0,
            ),
            hover_above: boolean("editor.hover.above", d.hover_above),
            hover_long_line_warning: boolean(
                "editor.hover.showLongLineWarning",
                d.hover_long_line_warning,
            ),
            max_tokenization_line_length: number(
                "editor.maxTokenizationLineLength",
                d.max_tokenization_line_length as u64,
                f64::from(u32::MAX),
            ) as usize,
            occurrences,
            occurrences_delay_ms: number(
                "editor.occurrencesHighlightDelay",
                d.occurrences_delay_ms,
                2000.0,
            ),
            occurrences_from_all_providers: boolean(
                "editor.occurrencesHighlightFromAllProviders",
                d.occurrences_from_all_providers,
            ),
        }
    }

    /// 是否是本模块关心的设置键。
    pub fn is_key(key: &str) -> bool {
        KEYS.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::{Cancellation, Events, NoEvents, Task};
    use rove_config::{DefaultSettings, SettingRegistry, SettingsChange};

    struct Over(DefaultSettings, Vec<(&'static str, Value)>);
    impl Settings for Over {
        fn read(&self, key: &str, scope: &Scope) -> rove_base::Result<Value> {
            match self.1.iter().find(|(k, _)| *k == key) {
                Some((_, v)) => Ok(v.clone()),
                None => self.0.read(key, scope),
            }
        }
        fn write(&self, _: Arc<str>, _: Scope, _: Value, c: Cancellation) -> Task<()> {
            self.0.write("x".into(), Scope::default(), Value::Null, c)
        }
        fn changes(&self) -> &dyn Events<SettingsChange> {
            &NoEvents
        }
    }

    fn with(values: Vec<(&'static str, Value)>) -> NavSettings {
        let mut r = SettingRegistry::default();
        rove_config::register_builtin(&mut r).unwrap();
        crate::code_view::options::register(&mut r).unwrap();
        NavSettings::read(&Over(DefaultSettings(r), values), Some("csharp"))
    }

    #[test]
    fn defaults_are_vscode_defaults() {
        assert_eq!(with(vec![]), NavSettings::default());
    }

    #[test]
    fn every_setting_is_read() {
        let s = with(vec![
            (
                "editor.gotoLocation.multipleDefinitions",
                Value::String("goto".into()),
            ),
            (
                "editor.gotoLocation.multipleImplementations",
                Value::String("gotoAndPeek".into()),
            ),
            ("editor.definitionLinkOpensInPeek", Value::Bool(true)),
            ("editor.hover.enabled", Value::String("off".into())),
            ("editor.hover.delay", Value::Number(50.0)),
            ("editor.hover.sticky", Value::Bool(false)),
            ("editor.hover.hidingDelay", Value::Number(10.0)),
            ("editor.hover.above", Value::Bool(false)),
            ("editor.hover.showLongLineWarning", Value::Bool(false)),
            ("editor.occurrencesHighlight", Value::String("off".into())),
            ("editor.occurrencesHighlightDelay", Value::Number(100.0)),
            (
                "editor.occurrencesHighlightFromAllProviders",
                Value::Bool(true),
            ),
        ]);
        assert_eq!(s.multiple_definitions, GotoMode::Goto);
        assert_eq!(s.multiple_implementations, GotoMode::GotoAndPeek);
        assert_eq!(s.multiple_references, GotoMode::Peek);
        assert!(s.definition_link_opens_in_peek);
        assert_eq!(s.hover_enabled, HoverEnabled::Off);
        assert_eq!(s.hover_delay_ms, 50);
        assert!(!s.hover_sticky);
        assert_eq!(s.hover_hiding_delay_ms, 10);
        assert!(!s.hover_above);
        assert!(!s.hover_long_line_warning);
        assert_eq!(s.occurrences, Occurrences::Off);
        assert_eq!(s.occurrences_delay_ms, 100);
        assert!(s.occurrences_from_all_providers);
        // 旧版布尔值。
        assert_eq!(
            with(vec![("editor.hover.enabled", Value::Bool(false))]).hover_enabled,
            HoverEnabled::Off
        );
        assert_eq!(
            with(vec![("editor.occurrencesHighlight", Value::Bool(true))]).occurrences,
            Occurrences::SingleFile
        );
        // 已弃用的总开关覆盖分项。
        let legacy = with(vec![
            ("editor.gotoLocation.multiple", Value::String("goto".into())),
            (
                "editor.gotoLocation.multipleDefinitions",
                Value::String("peek".into()),
            ),
        ]);
        assert_eq!(legacy.multiple_definitions, GotoMode::Goto);
        assert_eq!(legacy.multiple_references, GotoMode::Goto);
    }
}
