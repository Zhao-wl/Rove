//! VS Code compatible glob rules for `files.exclude` and friends (#15).
//!
//! Semantics follow VS Code's [glob patterns](https://aka.ms/vscode-glob-patterns) for the subset
//! the product uses: `*`, `?`, `**`, `{a,b}` and `[...]`, plus the implicit leading `**/` match
//! (`**/Library` also matches a top-level `Library`). Patterns are matched against the
//! workspace-relative `/`-separated path; all globs of one setting are compiled into a single
//! [`GlobSet`] so the scan hot path matches every rule in one pass instead of one regex per rule.
//!
//! Two documented limitations:
//!
//! 1. `{"when": "$(basename).ts"}` sibling-file conditions are **not** evaluated. Values that are
//!    not booleans are ignored (counted in [`RuleSet::ignored_conditions`]) instead of failing the
//!    whole rule set, so a settings file using them never crashes the workspace.
//! 2. Invalid glob patterns are skipped (counted in [`RuleSet::invalid_patterns`]) rather than
//!    rejecting the setting, matching VS Code's tolerance for bad user patterns.

use globset::{GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use rove_base::{Error, Result, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// A compiled set of `glob -> bool` rules.
#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    /// Fast path: every valid pattern in one `regex::RegexSet`.
    set: Option<GlobSet>,
    /// Per-rule metadata in insertion order (parallels the `GlobSet` indices).
    rules: Vec<RuleMeta>,
    /// Fallback matchers, only populated when building the `GlobSet` failed (pathological size).
    fallback: Vec<GlobMatcher>,
    ignored_conditions: usize,
    invalid_patterns: usize,
}

#[derive(Clone, Debug)]
struct RuleMeta {
    value: bool,
    specificity: u32,
}

thread_local! {
    /// Reused match buffer so the scan hot path does not allocate per entry.
    static MATCHES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

impl RuleSet {
    /// No rules at all: nothing matches.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Compiles a settings value. The value must be an object; non-boolean members are ignored.
    pub fn from_value(value: &Value) -> Result<Self> {
        let (map, ignored) = decode(value)?;
        Ok(Self::from_map(&map, ignored))
    }

    /// Compiles `high` on top of `low`: for the same pattern text `high` wins.
    ///
    /// This is how `files.exclude` overrides `rove.files.extraExclude`; an explicit
    /// `"**/Library": false` in `files.exclude` therefore re-includes a directory that the Rove
    /// extra default would exclude.
    pub fn merged(high: &Value, low: &Value) -> Result<Self> {
        let (high_map, high_ignored) = decode(high)?;
        let (low_map, low_ignored) = decode(low)?;
        let mut combined = low_map;
        combined.extend(high_map);
        Ok(Self::from_map(&combined, high_ignored + low_ignored))
    }

    fn from_map(map: &BTreeMap<String, bool>, ignored_conditions: usize) -> Self {
        let mut builder = GlobSetBuilder::new();
        let mut rules = Vec::with_capacity(map.len());
        let mut fallback = Vec::new();
        let mut invalid_patterns = 0;
        for (pattern, value) in map {
            let glob = match GlobBuilder::new(pattern)
                .literal_separator(true)
                .backslash_escape(false)
                .case_insensitive(cfg!(windows))
                .build()
            {
                Ok(glob) => glob,
                Err(_) => {
                    invalid_patterns += 1;
                    continue;
                }
            };
            rules.push(RuleMeta {
                value: *value,
                specificity: specificity(pattern),
            });
            fallback.push(glob.compile_matcher());
            builder.add(glob);
        }
        let set = if fallback.is_empty() {
            None
        } else {
            builder.build().ok()
        };
        if set.is_some() {
            // The `GlobSet` path does not need the individual matchers.
            fallback.clear();
        }
        Self {
            set,
            rules,
            fallback,
            ignored_conditions,
            invalid_patterns,
        }
    }

    /// Number of object-form (`when`) values that were ignored.
    pub fn ignored_conditions(&self) -> usize {
        self.ignored_conditions
    }

    /// Number of invalid glob patterns that were skipped.
    pub fn invalid_patterns(&self) -> usize {
        self.invalid_patterns
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Evaluates one path only (no ancestor walk).
    ///
    /// The scan uses this because excluded directories are pruned by the walker, so a yielded
    /// entry can never sit under an excluded ancestor.
    pub fn matches_self(&self, path: &str) -> Option<bool> {
        if self.rules.is_empty() {
            return None;
        }
        match &self.set {
            Some(set) => MATCHES.with(|buffer| {
                let mut buffer = buffer.borrow_mut();
                buffer.clear();
                set.matches_into(path, &mut buffer);
                self.best(&buffer)
            }),
            None => {
                let hits: Vec<usize> = self
                    .fallback
                    .iter()
                    .enumerate()
                    .filter(|(_, matcher)| matcher.is_match(path))
                    .map(|(index, _)| index)
                    .collect();
                self.best(&hits)
            }
        }
    }

    fn best(&self, hits: &[usize]) -> Option<bool> {
        let mut best: Option<usize> = None;
        for &index in hits {
            let Some(rule) = self.rules.get(index) else {
                continue;
            };
            let replace = match best.and_then(|current| self.rules.get(current)) {
                None => true,
                Some(current) => {
                    rule.specificity > current.specificity
                        || (rule.specificity == current.specificity && rule.value && !current.value)
                }
            };
            if replace {
                best = Some(index);
            }
        }
        best.and_then(|index| self.rules.get(index))
            .map(|rule| rule.value)
    }

    /// Evaluates a path and its ancestors, deepest first.
    ///
    /// Deepest-first is what allows `{"**/Library": true, "**/Library/Keep": false}` to
    /// re-include `Library/Keep` while the rest of `Library` stays excluded.
    pub fn matches(&self, path: &str) -> Option<bool> {
        if self.rules.is_empty() {
            return None;
        }
        let mut candidate = path;
        loop {
            if let Some(value) = self.matches_self(candidate) {
                return Some(value);
            }
            let index = candidate.rfind('/')?;
            candidate = &candidate[..index];
        }
    }

    /// Convenience form: excluded means a matching rule evaluated to `true`.
    pub fn is_excluded(&self, path: &str) -> bool {
        self.matches(path).unwrap_or(false)
    }
}

/// Decodes an object of booleans. Non-boolean values (the `when` condition form) are ignored and
/// counted so callers/tests can prove the documented limitation.
fn decode(value: &Value) -> Result<(BTreeMap<String, bool>, usize)> {
    let Value::Object(map) = value else {
        return Err(Error::InvalidInput);
    };
    let mut rules = BTreeMap::new();
    let mut ignored = 0;
    for (pattern, value) in map.iter() {
        match value {
            Value::Bool(enabled) => {
                rules.insert(pattern.clone(), *enabled);
            }
            _ => ignored += 1,
        }
    }
    Ok((rules, ignored))
}

/// Rough "how specific is this pattern" score: literal characters weigh more than wildcards,
/// extra path segments add a little. Used to pick a winner when several rules match one path.
fn specificity(pattern: &str) -> u32 {
    let mut score = 0u32;
    for character in pattern.chars() {
        match character {
            '*' | '?' | '[' | ']' | '{' | '}' => {}
            _ => score += 4,
        }
    }
    score + pattern.matches('/').count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn rules(pairs: &[(&str, bool)]) -> RuleSet {
        let map: BTreeMap<String, Value> = pairs
            .iter()
            .map(|(pattern, value)| (pattern.to_string(), Value::Bool(*value)))
            .collect();
        RuleSet::from_value(&Value::Object(Arc::new(map))).unwrap()
    }

    #[test]
    fn leading_double_star_matches_the_top_level_too() {
        let set = rules(&[("**/Library", true)]);
        assert_eq!(set.matches_self("Library"), Some(true));
        assert_eq!(set.matches_self("Assets/Library"), Some(true));
        assert_eq!(set.matches_self("MyLibrary"), None);
    }

    #[test]
    fn star_question_and_alternates_follow_vscode() {
        let set = rules(&[
            ("**/obj", true),
            ("**/Logs", true),
            ("**/*.{tmp,bak}", true),
            ("**/File[0-9].cs", true),
            ("**/Temp?", true),
        ]);
        assert!(set.is_excluded("Assets/obj"));
        assert!(set.is_excluded("Logs"));
        assert!(set.is_excluded("Assets/notes.tmp"));
        assert!(set.is_excluded("Assets/notes.bak"));
        assert!(!set.is_excluded("Assets/notes.txt"));
        assert!(set.is_excluded("Assets/File7.cs"));
        assert!(set.is_excluded("Temp1"));
        assert!(!set.is_excluded("Temp12"));
    }

    #[test]
    fn deeper_false_rule_reincludes_inside_an_excluded_directory() {
        let set = rules(&[("**/Library", true), ("**/Library/Keep", false)]);
        assert!(set.is_excluded("Library/PackageCache/A.cs"));
        assert!(!set.is_excluded("Library/Keep/A.cs"));
    }

    #[test]
    fn most_specific_pattern_wins_within_one_source() {
        let set = rules(&[("**/*.cs", true), ("**/Player.cs", false)]);
        assert!(set.is_excluded("Assets/A.cs"));
        assert!(!set.is_excluded("Assets/Player.cs"));
    }

    #[test]
    fn merged_rules_let_files_exclude_override_the_extra_default() {
        let high = Value::Object(Arc::new(BTreeMap::from([(
            "**/Library".to_string(),
            Value::Bool(false),
        )])));
        let low = Value::Object(Arc::new(BTreeMap::from([
            ("**/Library".to_string(), Value::Bool(true)),
            ("**/Temp".to_string(), Value::Bool(true)),
        ])));
        let merged = RuleSet::merged(&high, &low).unwrap();
        assert!(!merged.is_excluded("Library"));
        assert!(merged.is_excluded("Temp"));
    }

    #[test]
    fn object_values_are_ignored_and_counted_not_fatal() {
        let value = Value::Object(Arc::new(BTreeMap::from([
            (
                "**/A.cs".to_string(),
                Value::Object(Arc::new(BTreeMap::from([(
                    "when".to_string(),
                    Value::String("$(basename).ts".into()),
                )]))),
            ),
            ("**/B.cs".to_string(), Value::Bool(true)),
        ])));
        let set = RuleSet::from_value(&value).unwrap();
        assert_eq!(set.ignored_conditions(), 1);
        assert!(!set.is_excluded("A.cs"));
        assert!(set.is_excluded("B.cs"));
    }

    #[test]
    fn invalid_patterns_are_skipped_and_counted() {
        let set = rules(&[("**/[", true), ("**/Temp", true)]);
        assert_eq!(set.invalid_patterns(), 1);
        assert!(set.is_excluded("Temp"));
    }

    #[test]
    fn a_non_object_value_is_invalid_input() {
        assert_eq!(
            RuleSet::from_value(&Value::Bool(true)).unwrap_err(),
            Error::InvalidInput
        );
    }
}
