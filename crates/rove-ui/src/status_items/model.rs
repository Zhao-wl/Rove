//! 文档状态 → 状态栏条目与选择器内容（纯数据，不依赖窗口；#27）。
//!
//! 所有文字都经字符串表（[`registry::STRINGS`](super::registry::STRINGS)）取出，颜色只用
//! [`StatusKind`] 映射到的 `statusBarItem.*` 键。没有活动编辑器时宿主直接移除这些条目
//! （与 VS Code 一致），因此这里的函数都假设有一个文档。

use super::registry;
use crate::{
    code_view::view::read_only_reason_key,
    shell::status::{StatusEntry, StatusKind},
};
use rove_buffer::{Encoding, Eol, LineEndings, ReadOnlyReason, ReducedFeature, SizeClass};
use rove_i18n::{Message, Strings};
use std::sync::Arc;

// ------------------------------------------------------------------ 字符串

fn s(strings: &dyn Strings, key: &str, args: &[(&str, String)]) -> String {
    let message = Message {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
            .collect(),
    };
    strings
        .text(&message)
        .unwrap_or_else(|e| panic!("status string {key}: {e}"))
}

/// 可能没有登记的 key（语言显示名）：取不到时回退到原始 ID，不 panic。
fn optional(strings: &dyn Strings, key: &str, fallback: &str) -> String {
    strings
        .text(&Message::key(key))
        .unwrap_or_else(|_| fallback.to_owned())
}

// ------------------------------------------------------------------ 缩进

/// 一个文件生效的缩进（视图参数，不改文件内容）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indentation {
    pub insert_spaces: bool,
    pub size: u64,
    /// 来自 `editor.detectIndentation` 的内容检测而不是设置值。
    pub detected: bool,
}

impl Default for Indentation {
    fn default() -> Self {
        Self {
            insert_spaces: true,
            size: 4,
            detected: false,
        }
    }
}

/// VS Code `guessIndentation` 的简化移植：统计前 `limit` 行的前导空白。
///
/// 规则：前导空白里出现制表符的行记为“制表符缩进”，否则只要有前导空格就记为“空格缩进”并记录
/// 与上一个空格缩进行的缩进差；空格行更多时判定为空格缩进，缩进宽度取出现最多的差值（2–8），
/// 没有可用差值时保留设置里的 `editor.tabSize`。完全没有缩进的文件保留设置值
/// （`detected` 为 `false`）。
pub fn guess_indentation<'a>(
    lines: impl Iterator<Item = &'a str>,
    limit: usize,
    configured: Indentation,
) -> Indentation {
    let mut tab_lines = 0u32;
    let mut space_lines = 0u32;
    let mut previous: Option<usize> = None;
    let mut diffs = [0u32; 9];
    for line in lines.take(limit) {
        let mut spaces = 0usize;
        let mut tabs = 0usize;
        for ch in line.chars() {
            match ch {
                ' ' => spaces += 1,
                '\t' => tabs += 1,
                _ => break,
            }
        }
        // 只有空白的行不参与统计（与 VS Code 一致）。
        if line.chars().all(char::is_whitespace) {
            continue;
        }
        if tabs > 0 {
            tab_lines += 1;
            previous = None;
            continue;
        }
        if spaces == 0 {
            previous = Some(0);
            continue;
        }
        space_lines += 1;
        if let Some(before) = previous {
            let diff = spaces.abs_diff(before);
            if (1..=8).contains(&diff) {
                diffs[diff] += 1;
            }
        }
        previous = Some(spaces);
    }
    if tab_lines == 0 && space_lines == 0 {
        return Indentation {
            detected: false,
            ..configured
        };
    }
    if tab_lines >= space_lines {
        return Indentation {
            insert_spaces: false,
            size: configured.size,
            detected: true,
        };
    }
    let best = (2..=8)
        .filter(|size| diffs[*size] > 0)
        .max_by_key(|size| (diffs[*size], std::cmp::Reverse(*size)));
    Indentation {
        insert_spaces: true,
        size: best.map_or(configured.size, |size| size as u64),
        detected: true,
    }
}

// ------------------------------------------------------------------ 光标位置

/// 一个选区的摘要：插入符位置（1 基）与选中的字符数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorSummary {
    pub line: u64,
    pub column: u64,
    pub selected: u64,
}

/// `行 12，列 34`；单个选区有选中内容时附加已选字符数，多选区时显示选区个数与总字符数。
pub fn cursor_entry(strings: &dyn Strings, cursors: &[CursorSummary]) -> StatusEntry {
    let text = match cursors {
        [] => s(
            strings,
            "status.cursor.position",
            &[("line", "1".into()), ("column", "1".into())],
        ),
        [one] if one.selected == 0 => s(
            strings,
            "status.cursor.position",
            &[
                ("line", one.line.to_string()),
                ("column", one.column.to_string()),
            ],
        ),
        [one] => s(
            strings,
            "status.cursor.positionSelected",
            &[
                ("line", one.line.to_string()),
                ("column", one.column.to_string()),
                ("count", one.selected.to_string()),
            ],
        ),
        many => {
            let characters: u64 = many.iter().map(|c| c.selected).sum();
            if characters == 0 {
                s(
                    strings,
                    "status.cursor.selections",
                    &[("count", many.len().to_string())],
                )
            } else {
                s(
                    strings,
                    "status.cursor.selectionsCharacters",
                    &[
                        ("count", many.len().to_string()),
                        ("characters", characters.to_string()),
                    ],
                )
            }
        }
    };
    StatusEntry {
        text,
        tooltip: Some(s(strings, "status.cursor.tooltip", &[])),
        command: Some(registry::GOTO_LINE.into()),
        kind: StatusKind::Standard,
    }
}

// ------------------------------------------------------------------ 缩进条目

pub fn indent_entry(strings: &dyn Strings, indent: Indentation) -> StatusEntry {
    let key = if indent.insert_spaces {
        "status.indent.spaces"
    } else {
        "status.indent.tabs"
    };
    let mut tooltip = s(strings, "status.indent.tooltip", &[]);
    if indent.detected {
        tooltip.push('\n');
        tooltip.push_str(&s(strings, "status.indent.detected", &[]));
    }
    StatusEntry {
        text: s(strings, key, &[("size", indent.size.to_string())]),
        tooltip: Some(tooltip),
        command: Some(registry::CHANGE_INDENTATION.into()),
        kind: StatusKind::Standard,
    }
}

// ------------------------------------------------------------------ 编码条目

/// 元信息里的编码 → 显示名（与 VS Code 的编码选择器相同的写法）。
pub fn encoding_label(encoding: &Encoding) -> String {
    match encoding {
        Encoding::Utf8 => "UTF-8".into(),
        Encoding::Utf16Le => "UTF-16 LE".into(),
        Encoding::Utf16Be => "UTF-16 BE".into(),
        Encoding::Gbk => "GBK".into(),
        Encoding::Other(name) => name.to_string(),
    }
}

/// 编码条目。`guessed` 来自 `rove_buffer::Detection`（冻结的元信息里没有这个字段）：
/// 自动识别出来的编码在悬停提示里注明（UX_SPEC §3.5）。
pub fn encoding_entry(
    strings: &dyn Strings,
    encoding: &Encoding,
    bom: bool,
    guessed: bool,
) -> StatusEntry {
    let name = encoding_label(encoding);
    let text = if bom {
        s(strings, "status.encoding.bomSuffix", &[("name", name)])
    } else {
        name
    };
    let tooltip = if guessed {
        "status.encoding.tooltipGuessed"
    } else {
        "status.encoding.tooltip"
    };
    StatusEntry {
        text,
        tooltip: Some(s(strings, tooltip, &[])),
        command: Some(registry::CHANGE_ENCODING.into()),
        kind: StatusKind::Standard,
    }
}

// ------------------------------------------------------------------ 换行符条目

pub fn eol_label(strings: &dyn Strings, endings: &LineEndings) -> String {
    match endings {
        LineEndings::Uniform(Eol::Lf) => "LF".into(),
        LineEndings::Uniform(Eol::CrLf) => "CRLF".into(),
        LineEndings::Uniform(Eol::Cr) => "CR".into(),
        LineEndings::Mixed { .. } => s(strings, "status.eol.mixed", &[]),
    }
}

pub fn eol_entry(strings: &dyn Strings, endings: &LineEndings) -> StatusEntry {
    let mut tooltip = s(strings, "status.eol.tooltip", &[]);
    if matches!(endings, LineEndings::Mixed { .. }) {
        tooltip.push('\n');
        tooltip.push_str(&s(strings, "status.eol.tooltipMixed", &[]));
    }
    StatusEntry {
        text: eol_label(strings, endings),
        tooltip: Some(tooltip),
        command: Some(registry::CHANGE_EOL.into()),
        kind: StatusKind::Standard,
    }
}

// ------------------------------------------------------------------ 语言模式条目

/// 语言 ID → 显示名；没有登记显示名的语言直接用 ID（与 VS Code 对未知语言的做法一致）。
pub fn language_label(strings: &dyn Strings, language: &str) -> String {
    optional(strings, &format!("status.language.{language}"), language)
}

pub fn language_entry(strings: &dyn Strings, language: &str) -> StatusEntry {
    StatusEntry {
        text: language_label(strings, language),
        tooltip: Some(s(strings, "status.language.tooltip", &[])),
        command: Some(registry::CHANGE_LANGUAGE_MODE.into()),
        kind: StatusKind::Standard,
    }
}

// ------------------------------------------------------------------ 只读 / 超大文件条目

/// 因文件大小被关闭的功能 → 界面字符串 key（功能清单与顺序来自 #8 的
/// [`rove_buffer::disabled_features`]，本 issue 不自己判定大小）。
pub fn feature_key(feature: ReducedFeature) -> &'static str {
    match feature {
        ReducedFeature::Minimap => "status.feature.minimap",
        ReducedFeature::StickyScroll => "status.feature.stickyScroll",
        ReducedFeature::BracketPairColorization => "status.feature.bracketPairColorization",
        ReducedFeature::OccurrencesHighlight => "status.feature.occurrencesHighlight",
        ReducedFeature::SemanticHighlighting => "status.feature.semanticHighlighting",
        ReducedFeature::SyntaxFolding => "status.feature.syntaxFolding",
        ReducedFeature::SyntaxHighlighting => "status.feature.syntaxHighlighting",
        ReducedFeature::Folding => "status.feature.folding",
        ReducedFeature::WordWrap => "status.feature.wordWrap",
        ReducedFeature::Replace => "status.feature.replace",
    }
}

/// `只读` 或 `超大文件 · 只读`；两者都没有时返回 `None`（条目不显示）。
/// 悬停提示列出只读原因，超大文件再列出被关闭的功能（数据来自 #8 / #20，本 issue 不自己探测）。
pub fn read_only_entry(
    strings: &dyn Strings,
    size: SizeClass,
    reasons: &[ReadOnlyReason],
    disabled: &[ReducedFeature],
) -> Option<StatusEntry> {
    let huge = size == SizeClass::Huge;
    if reasons.is_empty() && !huge {
        return None;
    }
    let text = s(
        strings,
        if huge {
            "status.readOnly.hugeText"
        } else {
            "status.readOnly.text"
        },
        &[],
    );
    let separator = s(strings, "status.readOnly.separator", &[]);
    let mut lines: Vec<String> = reasons
        .iter()
        .map(|reason| s(strings, read_only_reason_key(reason), &[]))
        .collect();
    lines.dedup();
    if !disabled.is_empty() {
        let features = disabled
            .iter()
            .map(|feature| s(strings, feature_key(*feature), &[]))
            .collect::<Vec<_>>()
            .join(&separator);
        if !features.is_empty() {
            lines.push(s(
                strings,
                "status.readOnly.disabled",
                &[("features", features)],
            ));
        }
    }
    Some(StatusEntry {
        text,
        tooltip: Some(lines.join("\n")),
        command: None,
        kind: StatusKind::Warning,
    })
}

// ------------------------------------------------------------------ 选择器

/// 选择器里的一行：`id` 是宿主执行时用的键，`group` 非空时在这一行之前插入分组标题。
#[derive(Clone, Debug, PartialEq)]
pub struct PickItem {
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub detail: Option<Arc<str>>,
    pub group: Option<Arc<str>>,
}

impl PickItem {
    fn new(id: &str, label: String) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: None,
            group: None,
        }
    }
    fn with_group(mut self, group: String) -> Self {
        self.group = Some(group.into());
        self
    }
    fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// 缩进选择器。前三项只改本文件的缩进视图参数；后两项会改写文件内容，走 #46 的编辑核心
/// （`editable` 为 false 时不登记、不显示，UX_SPEC §1.2）。
pub fn indent_picks(strings: &dyn Strings, editable: bool) -> Vec<PickItem> {
    let view = s(strings, "status.indent.group.view", &[]);
    let mut picks = vec![
        PickItem::new(
            registry::INDENT_USING_SPACES,
            s(strings, "status.indent.pick.useSpaces", &[]),
        )
        .with_group(view),
        PickItem::new(
            registry::INDENT_USING_TABS,
            s(strings, "status.indent.pick.useTabs", &[]),
        ),
        PickItem::new(
            registry::DETECT_INDENTATION,
            s(strings, "status.indent.pick.detect", &[]),
        ),
    ];
    if editable {
        let convert = s(strings, "status.indent.group.convert", &[]);
        picks.push(
            PickItem::new(
                "editor.action.indentationToSpaces",
                s(strings, "status.indent.pick.toSpaces", &[]),
            )
            .with_group(convert),
        );
        picks.push(PickItem::new(
            "editor.action.indentationToTabs",
            s(strings, "status.indent.pick.toTabs", &[]),
        ));
    }
    picks
}

/// 缩进宽度选择器（选了“使用空格 / 制表符缩进”之后）。
pub fn indent_size_picks(strings: &dyn Strings) -> Vec<PickItem> {
    (1..=8)
        .map(|size| {
            PickItem::new(
                &size.to_string(),
                s(
                    strings,
                    "status.indent.pick.size",
                    &[("size", size.to_string())],
                ),
            )
        })
        .collect()
}

/// 换行符选择器：显示当前值；统一操作会改写文件，只在可编辑时提供（UX_SPEC §3.6）。
pub fn eol_picks(strings: &dyn Strings, editable: bool) -> Vec<PickItem> {
    if !editable {
        return Vec::new();
    }
    vec![
        PickItem::new("\n", s(strings, "status.eol.toLf", &[])),
        PickItem::new("\r\n", s(strings, "status.eol.toCrlf", &[])),
    ]
}

/// 语言模式选择器：#12 已登记的语言 + `plaintext`，按显示名排序，当前语言排在最前。
pub fn language_picks(
    strings: &dyn Strings,
    languages: &[Arc<str>],
    current: &str,
) -> Vec<PickItem> {
    let mut picks: Vec<PickItem> = languages
        .iter()
        .map(|id| PickItem::new(id, language_label(strings, id)))
        .collect();
    picks.sort_by(|a, b| a.label.cmp(&b.label));
    if let Some(index) = picks.iter().position(|pick| &*pick.id == current) {
        let pick = picks.remove(index);
        picks.insert(0, pick);
    }
    picks
}

/// M1 的编码选择器第一层：只有“通过编码重新打开”；可保存时加上“通过编码保存”（#49）。
pub fn encoding_actions(strings: &dyn Strings, savable: bool) -> Vec<PickItem> {
    let mut picks = vec![PickItem::new(
        ENCODING_REOPEN,
        s(strings, "status.encoding.reopen", &[]),
    )];
    if savable {
        picks.push(PickItem::new(
            ENCODING_SAVE,
            s(strings, "status.encoding.save", &[]),
        ));
    }
    picks
}

/// 编码选择器第一层的两个动作键。
pub const ENCODING_REOPEN: &str = "reopen";
pub const ENCODING_SAVE: &str = "save";

/// 常用编码（VS Code 编码选择器里最常被选到的那几个），按 `files.encoding` 的 ID 书写。
pub const COMMON_ENCODINGS: &[&str] = &[
    "utf8",
    "utf16le",
    "utf16be",
    "gbk",
    "gb18030",
    "big5",
    "shiftjis",
    "euckr",
    "windows1252",
    "iso88591",
];

/// 编码候选列表：`files.encoding` → `files.candidateGuessEncodings` → 常用编码，去重后分组。
/// 每一行的 `id` 是 VS Code 的编码 ID（直接交给 `rove_buffer` 的 `explicit_encoding`），
/// `detail` 也写出这个 ID，方便用户照着改 `files.encoding`。无法识别的 ID 直接丢掉。
pub fn encoding_picks(
    strings: &dyn Strings,
    configured: &str,
    candidates: &[String],
) -> Vec<PickItem> {
    let groups = [
        s(strings, "status.encoding.group.configured", &[]),
        s(strings, "status.encoding.group.guessed", &[]),
        s(strings, "status.encoding.group.common", &[]),
    ];
    let sources: [Vec<String>; 3] = [
        vec![configured.to_owned()],
        candidates.to_vec(),
        COMMON_ENCODINGS.iter().map(|id| (*id).to_owned()).collect(),
    ];
    let mut picks: Vec<PickItem> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (group, ids) in groups.into_iter().zip(sources) {
        let mut first = true;
        for id in ids {
            let Some(encoding) = rove_buffer::settings::encoding_from_id(&id) else {
                continue;
            };
            let key = id.trim().to_ascii_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let mut pick = PickItem::new(&id, encoding.name().to_owned()).with_detail(id.clone());
            if first {
                pick = pick.with_group(group.clone());
                first = false;
            }
            picks.push(pick);
        }
    }
    picks
}
