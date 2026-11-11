//! 用户设置的分层视图与优先级解析。

use crate::{
    catalog::LanguageDefaults,
    jsonc::{Document, NodeKind},
    Scope, Setting, SettingRegistry,
};
use rove_base::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// `"[csharp]"`、`"[markdown][plaintext]"` → 语言 ID 列表；不是语言覆盖键返回 `None`。
/// 与 VS Code 的 `OVERRIDE_PROPERTY_REGEX = /^(\[[^\]]+\])+$/` 一致。
pub(crate) fn language_ids(key: &str) -> Option<Vec<&str>> {
    let inner = key.strip_prefix('[')?.strip_suffix(']')?;
    let ids: Vec<&str> = inner.split("][").collect();
    ids.iter()
        .all(|id| !id.is_empty() && !id.contains(['[', ']']))
        .then_some(ids)
}

/// 用户在文件里写下的值及其位置（1 起始的行列，供类型不匹配警告定位）。
#[derive(Clone, Debug)]
pub(crate) struct Located {
    pub value: Value,
    pub line: u32,
    pub column: u32,
}

/// 共用 `settings.json` 的内容：全局值与按语言的覆盖。重复键与重复语言块都是后者胜出。
/// 不认识的键也原样保存（读取时按登记表过滤），这样变更比较不需要登记表。
#[derive(Clone, Debug, Default)]
pub(crate) struct UserLayer {
    pub global: BTreeMap<String, Located>,
    pub languages: BTreeMap<String, BTreeMap<String, Located>>,
}

impl UserLayer {
    pub fn from_document(text: &str, doc: &Document) -> Self {
        let lines = LineIndex::new(text);
        let locate = |value: &crate::jsonc::Node| {
            let (line, column) = lines.position(text, value.span.start);
            Located {
                value: value.to_value(),
                line,
                column,
            }
        };
        let mut layer = Self::default();
        for member in doc.root_members() {
            match language_ids(&member.key) {
                Some(languages) => {
                    // 语言块的值不是对象时整块忽略（VS Code 同样不应用）。
                    let NodeKind::Object(inner) = &member.value.kind else {
                        continue;
                    };
                    for language in languages {
                        let target = layer.languages.entry(language.to_string()).or_default();
                        for m in inner {
                            target.insert(m.key.clone(), locate(&m.value));
                        }
                    }
                }
                None => {
                    layer
                        .global
                        .insert(member.key.clone(), locate(&member.value));
                }
            }
        }
        layer
    }

    fn language_value(&self, language: &str, key: &str) -> Option<&Located> {
        self.languages.get(language)?.get(key)
    }

    /// 用户是否在该作用域显式设置了 `key`（全局，或该语言的覆盖块）。
    pub fn is_set(&self, key: &str, setting: &Setting, scope: &Scope) -> bool {
        self.global.contains_key(key)
            || overridable_language(setting, scope)
                .is_some_and(|l| self.language_value(l, key).is_some())
    }

    /// 两份内容之间值发生变化、且已登记的键。
    pub fn changed_keys(&self, other: &Self, registry: &SettingRegistry) -> Vec<Arc<str>> {
        let mut keys = BTreeSet::new();
        diff(&self.global, &other.global, &mut keys);
        let languages: BTreeSet<&String> = self
            .languages
            .keys()
            .chain(other.languages.keys())
            .collect();
        let empty = BTreeMap::new();
        for language in languages {
            diff(
                self.languages.get(language).unwrap_or(&empty),
                other.languages.get(language).unwrap_or(&empty),
                &mut keys,
            );
        }
        keys.into_iter()
            .filter(|k| registry.get(k).is_some())
            .map(Arc::from)
            .collect()
    }
}

fn diff<'a>(
    a: &'a BTreeMap<String, Located>,
    b: &'a BTreeMap<String, Located>,
    out: &mut BTreeSet<&'a str>,
) {
    for (k, v) in a {
        if b.get(k).is_none_or(|w| w.value != v.value) {
            out.insert(k);
        }
    }
    for k in b.keys() {
        if !a.contains_key(k) {
            out.insert(k);
        }
    }
}

fn overridable_language<'a>(setting: &Setting, scope: &'a Scope) -> Option<&'a str> {
    scope
        .language
        .as_deref()
        .filter(|_| setting.language_overridable)
}

/// 候选值所在的层，按优先级从高到低。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// 用户 `settings.json` 里的 `"[lang]": {...}`。
    UserLanguage,
    /// VS Code 内置的默认语言覆盖（例如 `[csharp]` 的 `editor.maxTokenizationLineLength = 2500`）。
    LanguageDefault,
    /// 用户 `settings.json` 里的全局值。
    User,
    /// Rove 默认值（`compat/SETTINGS.md` §3 四项），仅在用户未设置该项时参与。
    RoveDefault,
    /// VS Code 默认值。
    Default,
}

pub(crate) struct Candidate<'a> {
    pub layer: Layer,
    pub value: &'a Value,
    pub location: Option<(u32, u32)>,
}

/// 按优先级列出 `key` 在 `scope` 下的候选值；最后一项总是 VS Code 默认值。
///
/// 顺序与 VS Code 一致：语言级用户覆盖 > 默认语言覆盖 > 全局用户值 > Rove 默认值 > VS Code 默认值。
/// （VS Code 的默认语言覆盖高于全局用户值：例如 `[markdown]` 的 `editor.wordWrap`。）
pub(crate) fn candidates<'a>(
    user: &'a UserLayer,
    defaults: &'a LanguageDefaults,
    key: &str,
    setting: &'a Setting,
    scope: &Scope,
) -> Vec<Candidate<'a>> {
    let mut out = Vec::with_capacity(3);
    let located = |layer, l: &'a Located| Candidate {
        layer,
        value: &l.value,
        location: Some((l.line, l.column)),
    };
    if let Some(language) = overridable_language(setting, scope) {
        if let Some(v) = user.language_value(language, key) {
            out.push(located(Layer::UserLanguage, v));
        }
        if let Some(v) = defaults.get(language, key) {
            out.push(Candidate {
                layer: Layer::LanguageDefault,
                value: v,
                location: None,
            });
        }
    }
    if let Some(v) = user.global.get(key) {
        out.push(located(Layer::User, v));
    }
    if let Some(v) = &setting.rove_default {
        if !user.is_set(key, setting, scope) {
            out.push(Candidate {
                layer: Layer::RoveDefault,
                value: v,
                location: None,
            });
        }
    }
    out.push(Candidate {
        layer: Layer::Default,
        value: &setting.default,
        location: None,
    });
    out
}

/// 行首偏移表：一次构建，按二分查找换算行列。
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' || (b == b'\r' && bytes.get(i + 1) != Some(&b'\n')) {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    fn position(&self, text: &str, offset: usize) -> (u32, u32) {
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let mut prefix = &text[self.starts[line]..offset];
        if line == 0 {
            prefix = prefix.strip_prefix('\u{feff}').unwrap_or(prefix);
        }
        (line as u32 + 1, prefix.chars().count() as u32 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_keys_follow_vscode_pattern() {
        assert_eq!(language_ids("[csharp]"), Some(vec!["csharp"]));
        assert_eq!(language_ids("[a][b-c]"), Some(vec!["a", "b-c"]));
        for bad in ["csharp", "[]", "[a]b", "[a][]", "[[a]]", "[a"] {
            assert_eq!(language_ids(bad), None, "{bad}");
        }
    }

    #[test]
    fn multi_language_blocks_and_positions() {
        let text = "\u{feff}{\r\n  \"[markdown][plaintext]\": {\"editor.wordWrap\": \"off\"},\r\n  \"[markdown]\": {\"editor.wordWrap\": \"on\"},\r\n  \"[bad]\": 1,\r\n  \"x\": 中\r\n}".replace('中', "true");
        let doc = Document::parse(&text).unwrap();
        let layer = UserLayer::from_document(&text, &doc);
        assert_eq!(
            layer
                .language_value("markdown", "editor.wordWrap")
                .unwrap()
                .value,
            Value::String("on".into())
        );
        assert_eq!(
            layer
                .language_value("plaintext", "editor.wordWrap")
                .unwrap()
                .value,
            Value::String("off".into())
        );
        assert!(!layer.languages.contains_key("bad"));
        let x = &layer.global["x"];
        assert_eq!((x.line, x.column), (5, 8));
    }
}
