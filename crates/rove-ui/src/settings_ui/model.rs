use rove_base::Value;
use rove_config::{Scope, Settings, SettingsService};
use rove_fuzzy::{
    create_matches, fuzzy_score, matches_contiguous_sub_string, utf16_to_byte_ranges,
    FuzzyScoreOptions,
};
use std::{ops::Range, sync::OnceLock};

const SETTINGS_CATALOG: &str = include_str!("../../../rove-config/data/settings-catalog.json");

#[derive(Clone, Debug)]
pub struct Choice {
    pub value: String,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct SettingRow {
    pub key: String,
    pub title: String,
    pub category: String,
    pub description: String,
    pub choices: Vec<Choice>,
    pub rove_default: bool,
    pub language_overridable: bool,
}

pub fn catalog_rows() -> &'static [SettingRow] {
    static ROWS: OnceLock<Vec<SettingRow>> = OnceLock::new();
    ROWS.get_or_init(|| {
        let data: serde_json::Value =
            serde_json::from_str(SETTINGS_CATALOG).expect("embedded settings catalog is valid");
        let active = crate::app_settings::consumer_registry().expect("consumers register settings");
        data["settings"]
            .as_array()
            .expect("embedded catalog has settings")
            .iter()
            .filter_map(|entry| {
                let key = entry["key"].as_str()?;
                let setting = active.get(key)?;
                let metadata = &entry["editor"];
                let choices = metadata["enum"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some(Choice {
                            value: option["value"].as_str()?.to_owned(),
                            description: option["description"].as_str()?.to_owned(),
                        })
                    })
                    .collect();
                Some(SettingRow {
                    key: key.to_owned(),
                    title: setting_title(key),
                    category: metadata["category"].as_str()?.to_owned(),
                    description: metadata["description"].as_str()?.to_owned(),
                    choices,
                    rove_default: setting.rove_default.is_some(),
                    language_overridable: setting.language_overridable,
                })
            })
            .collect()
    })
}

fn setting_title(key: &str) -> String {
    let mut title = String::new();
    let mut previous_lower = false;
    for ch in key.split('.').skip(1).collect::<Vec<_>>().join(" ").chars() {
        if previous_lower && ch.is_uppercase() {
            title.push(' ');
        }
        if title.is_empty() {
            title.extend(ch.to_uppercase());
        } else {
            title.push(ch);
        }
        previous_lower = ch.is_lowercase();
    }
    if title.is_empty() {
        key.to_owned()
    } else {
        title
    }
}

#[derive(Clone, Debug)]
pub struct RowMatch<'a> {
    pub row: &'a SettingRow,
    pub key: Vec<Range<usize>>,
    pub title: Vec<Range<usize>>,
    pub description: Vec<Range<usize>>,
}

pub fn search<'a>(
    rows: &'a [SettingRow],
    query: &str,
    is_modified: impl Fn(&str) -> bool,
) -> Vec<RowMatch<'a>> {
    let mut modified_only = false;
    let words = query
        .split_whitespace()
        .filter(|part| {
            if part.eq_ignore_ascii_case("@modified") {
                modified_only = true;
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>();
    let mut matches = rows
        .iter()
        .filter(|row| !modified_only || is_modified(&row.key))
        .filter_map(|row| {
            let mut result = RowMatch {
                row,
                key: Vec::new(),
                title: Vec::new(),
                description: Vec::new(),
            };
            let mut total = 0;
            for word in &words {
                let pattern: Vec<u16> = word.encode_utf16().collect();
                let lower_pattern: Vec<u16> = word.to_lowercase().encode_utf16().collect();
                let fields = [
                    (&row.key, &mut result.key, 3),
                    (&row.title, &mut result.title, 2),
                    (&row.description, &mut result.description, 1),
                ];
                let mut best = None;
                for (text, highlights, priority) in fields {
                    if let Some((score, ranges)) = score_field(word, &pattern, &lower_pattern, text)
                    {
                        highlights.extend(ranges);
                        best = Some(best.unwrap_or(i32::MIN).max(score + priority));
                    }
                }
                total += best?;
            }
            for ranges in [&mut result.key, &mut result.title, &mut result.description] {
                ranges.sort_unstable_by_key(|range| (range.start, range.end));
                let mut end = 0;
                ranges.retain_mut(|range| {
                    range.start = range.start.max(end);
                    end = range.end.max(end);
                    range.start < range.end
                });
            }
            Some((total, result))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    matches.into_iter().map(|(_, row)| row).collect()
}

fn score_field(
    word: &str,
    pattern: &[u16],
    lower_pattern: &[u16],
    text: &str,
) -> Option<(i32, Vec<Range<usize>>)> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let lower: Vec<u16> = text.to_lowercase().encode_utf16().collect();
    if let Some(score) = fuzzy_score(
        pattern,
        lower_pattern,
        0,
        &units,
        &lower,
        0,
        FuzzyScoreOptions {
            first_match_can_be_weak: true,
            ..Default::default()
        },
    ) {
        return Some((
            score.score,
            utf16_to_byte_ranges(text, &create_matches(Some(&score))),
        ));
    }
    // The scorer truncates long inputs. A literal hit anywhere in a description can still match.
    let ranges = matches_contiguous_sub_string(word, text)?;
    Some((0, utf16_to_byte_ranges(text, &ranges)))
}

pub fn effective_value(settings: &SettingsService, row: &SettingRow) -> Option<Value> {
    settings.read(&row.key, &Scope::default()).ok()
}

pub fn explicit_value(settings: &SettingsService, row: &SettingRow) -> bool {
    settings
        .is_user_set(&row.key, &Scope::default())
        .unwrap_or_default()
}

pub fn json_location(text: &str, key: &str) -> Option<(u32, u32)> {
    let doc = rove_config::jsonc::Document::parse(text).ok()?;
    let member = doc
        .root_members()
        .iter()
        .rev()
        .find(|item| item.key == key)?;
    Some(rove_config::jsonc::line_column(text, member.key_span.start))
}

pub fn language_overrides(text: &str) -> Option<std::collections::BTreeSet<String>> {
    use rove_config::jsonc::{Document, NodeKind};

    let doc = Document::parse(text).ok()?;
    if doc
        .root
        .as_ref()
        .is_some_and(|node| !matches!(node.kind, NodeKind::Object(_)))
    {
        return None;
    }
    let mut result = std::collections::BTreeSet::new();
    for member in doc.root_members() {
        if member.key.starts_with('[') && member.key.ends_with(']') {
            if let NodeKind::Object(members) = &member.value.kind {
                result.extend(members.iter().map(|entry| entry.key.clone()));
            }
        }
    }
    Some(result)
}

pub fn language_json_location(text: &str, key: &str) -> Option<(u32, u32)> {
    use rove_config::jsonc::{Document, NodeKind};

    let doc = Document::parse(text).ok()?;
    for member in doc.root_members().iter().rev() {
        if !member.key.starts_with('[') || !member.key.ends_with(']') {
            continue;
        }
        if let NodeKind::Object(members) = &member.value.kind {
            if let Some(setting) = members.iter().rev().find(|member| member.key == key) {
                return Some(rove_config::jsonc::line_column(
                    text,
                    setting.key_span.start,
                ));
            }
        }
    }
    None
}

pub fn source_position(text: &str, line: u32, column: u32) -> Option<rove_base::Position> {
    use rove_base::{Line, Position, Utf8Column};

    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut number = 1;
    let mut start = 0;
    let mut end = text.len();
    let mut iter = text.char_indices().peekable();
    while let Some((offset, ch)) = iter.next() {
        if !matches!(ch, '\r' | '\n') {
            continue;
        }
        if number == line {
            end = offset;
            break;
        }
        number += 1;
        start = offset + 1;
        if ch == '\r' && iter.peek().is_some_and(|(_, next)| *next == '\n') {
            iter.next();
            start += 1;
        }
    }
    if number != line || column == 0 {
        return None;
    }
    let content = text.get(start..end)?;
    let byte = content
        .char_indices()
        .nth(column.saturating_sub(1) as usize)
        .map_or(content.len(), |(byte, _)| byte);
    Some(Position {
        line: Line(u64::from(line.saturating_sub(1))),
        column: Utf8Column(byte as u64),
    })
}

pub fn clipboard_json(key: &str, value: &Value) -> Option<String> {
    let object = rove_config::edit::set("{}", &[key], Some(value)).ok()?;
    Some(
        object
            .strip_prefix('{')?
            .strip_suffix('}')?
            .trim()
            .to_owned(),
    )
}

pub fn text_value(current: &Value, text: &str) -> Option<Value> {
    match current {
        Value::Number(_) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(Value::Number),
        Value::String(_) => Some(Value::String(text.into())),
        _ => None,
    }
}

pub(crate) fn valid_edit(key: &str, value: &Value) -> bool {
    let Value::Number(value) = value else {
        return true;
    };
    value.is_finite()
        && crate::code_view::options::number_limits(key)
            .is_none_or(|(min, max)| (min..=max).contains(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_inputs_follow_the_code_view() {
        assert!(!valid_edit("editor.fontSize", &Value::Number(101.0)));
        assert!(!valid_edit("editor.tabSize", &Value::Number(0.0)));
        assert!(valid_edit("editor.fontSize", &Value::Number(14.5)));
    }

    #[test]
    fn active_catalog_exposes_metadata_and_excludes_registered_placeholders() {
        let rows = catalog_rows();
        let wrap = rows
            .iter()
            .find(|row| row.key == "editor.wordWrap")
            .unwrap();
        assert_eq!(wrap.title, "Word Wrap");
        assert_eq!(wrap.choices[0].value, "off");
        assert!(!wrap.choices[0].description.is_empty());
        assert!(!rows
            .iter()
            .any(|row| row.key == "workbench.settings.openDefaultSettings"));
        assert!(!rows.iter().any(|row| row.key == "telemetry.telemetryLevel"));
    }

    #[test]
    fn search_matches_metadata_and_filters_explicit_changes() {
        let rows = catalog_rows();
        let selected = "editor.fontSize";
        let hit = |query: &str| {
            search(rows, query, |key| key == selected)
                .into_iter()
                .find(|result| result.row.key == selected)
                .unwrap()
        };
        assert!(!hit("editor.fontSize").key.is_empty());
        assert!(!hit("Font Size").title.is_empty());
        let item = hit("像素");
        assert!(item
            .description
            .iter()
            .any(|range| { &item.row.description[range.clone()] == "像素" }));
        let modified = search(rows, "@modified", |key| key == selected);
        assert_eq!(modified.len(), 1);
        assert_eq!(modified[0].row.key, selected);
    }

    #[test]
    fn form_values_and_language_links_use_shared_jsonc_formats() {
        use std::sync::Arc;
        let text = "{\n \"[csharp][xml]\": {\n   \"editor.fontSize\": 16\n }\n}";
        assert_eq!(
            language_json_location(text, "editor.fontSize"),
            Some((3, 4))
        );
        assert_eq!(language_json_location(text, "editor.lineHeight"), None);
        assert_eq!(
            source_position("\u{feff}甲乙\r\n丙丁\r戊己", 1, 3)
                .unwrap()
                .column
                .0,
            6
        );
        assert_eq!(
            source_position("甲乙\r\n丙丁\r戊己", 3, 2)
                .unwrap()
                .column
                .0,
            3
        );
        assert_eq!(
            text_value(&Value::Number(14.0), " 16.5 "),
            Some(Value::Number(16.5))
        );
        assert_eq!(text_value(&Value::Number(14.0), "NaN"), None);
        assert_eq!(
            clipboard_json(
                "editor.fontFamily",
                &Value::String(Arc::from("Rove \"Code\""))
            )
            .as_deref(),
            Some("\"editor.fontFamily\": \"Rove \\\"Code\\\"\"")
        );
    }

    #[test]
    fn settings_link_matches_the_effective_duplicate_in_jsonc() {
        let text =
            "{\n // comment\n \"files.exclude\": {},\n \"files.exclude\": {\"tmp\": true},\n}";
        assert_eq!(json_location(text, "files.exclude"), Some((4, 2)));
        assert_eq!(json_location(text, "files.other"), None);
    }
}
