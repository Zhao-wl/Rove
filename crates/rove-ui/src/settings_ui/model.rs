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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn settings_link_matches_the_effective_duplicate_in_jsonc() {
        let text =
            "{\n // comment\n \"files.exclude\": {},\n \"files.exclude\": {\"tmp\": true},\n}";
        assert_eq!(json_location(text, "files.exclude"), Some((4, 2)));
        assert_eq!(json_location(text, "files.other"), None);
    }
}
