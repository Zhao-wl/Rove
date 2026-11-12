//! 文本同名高亮（#20）：`editor.occurrencesHighlight` 高亮光标所在单词在本文件中的其他出现；
//! `editor.selectionHighlight` 高亮与选中文本相同的其他位置。语义级同名高亮（只高亮同一符号）由 #40
//! 以更高优先级的装饰层接入。
//!
//! 匹配按行进行（装饰提供方只在行进入视口或预取范围时被调用一次），不扫描整个文件。

use super::decorations::{DecorationProvider, InlineDecoration, LineDecorations, TextStyle};
use rove_base::ByteOffset;
use rove_syntax::language_config::CompiledLanguageConfiguration;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// 单词边界的判定：语言配置的 `wordPattern`，没有时用 `editor.wordSeparators`。
#[derive(Clone)]
pub enum WordRule {
    Language(&'static CompiledLanguageConfiguration),
    Separators(Arc<str>),
}

impl WordRule {
    /// `column` 处的单词（字节区间）。
    pub fn word_at(&self, line: &str, column: usize) -> Option<(usize, usize)> {
        let column = column.min(line.len());
        match self {
            WordRule::Language(c) => {
                let r = c.word_at(line, ByteOffset(column as u64)).ok()??;
                Some((r.start().0 as usize, r.end().0 as usize))
            }
            WordRule::Separators(s) => {
                let (a, b) = super::selection::word_range_at(line, column, s);
                let w = &line[a..b];
                (a < b && !w.chars().all(char::is_whitespace)).then_some((a, b))
            }
        }
    }
}

/// 当前要高亮的目标。
#[derive(Clone)]
pub enum Target {
    /// 光标处的单词（大小写敏感、整词匹配）。
    Word { text: Arc<str>, rule: WordRule },
    /// 选中的文本；`exclude` 是选区本身的位置（行, 起始列），不重复高亮。
    Selection {
        text: Arc<str>,
        case_sensitive: bool,
        exclude: (u64, usize),
    },
}

/// 同名高亮层的状态。
#[derive(Default)]
pub struct Occurrences {
    pub target: Option<Target>,
    pub word: TextStyle,
    pub selection: TextStyle,
}

impl Occurrences {
    /// `hasWordHighlights` 上下文键：当前有单词高亮。
    pub fn has_word_highlights(&self) -> bool {
        matches!(self.target, Some(Target::Word { .. }))
    }

    /// 某行所有匹配的字节区间。
    pub fn matches(&self, line: u64, text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        match &self.target {
            None => {}
            Some(Target::Word { text: w, rule }) => {
                for (i, _) in text.match_indices(&**w) {
                    if rule.word_at(text, i) == Some((i, i + w.len())) {
                        out.push((i, i + w.len()));
                    }
                }
            }
            Some(Target::Selection {
                text: s,
                case_sensitive,
                exclude,
            }) => {
                let (hay, needle) = (text.as_bytes(), s.as_bytes());
                if needle.is_empty() || needle.len() > hay.len() {
                    return out;
                }
                let mut i = 0;
                while i + needle.len() <= hay.len() {
                    let window = &hay[i..i + needle.len()];
                    let hit = if *case_sensitive {
                        window == needle
                    } else {
                        window.eq_ignore_ascii_case(needle)
                    };
                    if hit && text.is_char_boundary(i) && text.is_char_boundary(i + needle.len()) {
                        if (line, i) != *exclude {
                            out.push((i, i + needle.len()));
                        }
                        i += needle.len();
                    } else {
                        i += 1;
                    }
                }
            }
        }
        out
    }
}

/// 同名高亮的装饰提供方。
pub struct OccurrenceProvider(pub Rc<RefCell<Occurrences>>);

impl DecorationProvider for OccurrenceProvider {
    fn decorate(&self, line: u64, text: &str, out: &mut LineDecorations) {
        let o = self.0.borrow();
        let style = match o.target {
            Some(Target::Word { .. }) => o.word,
            Some(Target::Selection { .. }) => o.selection,
            None => return,
        };
        for (a, b) in o.matches(line, text) {
            out.inline.push(InlineDecoration {
                range: a as u32..b as u32,
                style,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_words_only_and_selection_skips_itself() {
        let o = Occurrences {
            target: Some(Target::Word {
                text: "foo".into(),
                rule: WordRule::Separators("`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into()),
            }),
            ..Default::default()
        };
        assert_eq!(o.matches(0, "foo(foobar, foo)"), vec![(0, 3), (12, 15)]);
        let o = Occurrences {
            target: Some(Target::Selection {
                text: "ab".into(),
                case_sensitive: false,
                exclude: (0, 0),
            }),
            ..Default::default()
        };
        assert_eq!(o.matches(0, "ab AB xab"), vec![(3, 5), (7, 9)]);
        assert_eq!(o.matches(1, "ab"), vec![(0, 2)]);
    }

    #[test]
    fn language_word_pattern_is_used() {
        let (langs, _) = rove_syntax::language_config::BuiltinLanguages::shared();
        let cs = langs.compiled("csharp").expect("csharp");
        let rule = WordRule::Language(cs);
        assert_eq!(rule.word_at("var x = count;", 9), Some((8, 13)));
    }
}
