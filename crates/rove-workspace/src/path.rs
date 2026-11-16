//! Path identity, long-path and case-folding helpers (#15).
//!
//! Two separate concerns live here:
//!
//! - **Filesystem paths**: the workspace root is canonicalized once and keeps the Windows
//!   extended-length prefix (`\\?\`), so every joined path works past `MAX_PATH` without
//!   prefixing at each call site.
//! - **Workspace identity**: [`WorkspacePath`](rove_base::WorkspacePath) is the case-preserving
//!   display form. Lookups use [`identity_key`], which folds case on Windows. The fold is the
//!   locale-independent Unicode default lowercase mapping, not the OS `CompareString` table and
//!   not `str::to_lowercase` of the whole path: per-character mapping keeps the key independent
//!   of the Turkish locale problem (`I` and `i` fold together, `İ` folds to `i` + combining dot).
//!   The display path always keeps the case the disk reported.

use rove_base::{Error, Result};
use std::path::{Component, Path, PathBuf};

/// Canonical absolute workspace root.
///
/// On Windows [`std::fs::canonicalize`] returns a `\\?\`-prefixed path, which is exactly the
/// extended-length form the rest of the crate wants.
pub(crate) fn canonical_root(root: &Path) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(root)?;
    if !canonical.is_dir() {
        return Err(Error::NotFound);
    }
    Ok(canonical)
}

/// Workspace-relative `/`-separated path of an absolute path under `root`.
///
/// `None` means the entry must be skipped: outside the root, the root itself, or a non-UTF-8
/// name (the workspace identity is UTF-8 by contract, and one bad name must not fail the scan).
pub(crate) fn relative_str(root: &Path, absolute: &Path) -> Option<String> {
    let relative = absolute.strip_prefix(root).ok()?;
    let mut out = String::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                let name = name.to_str()?;
                if !out.is_empty() {
                    out.push('/');
                }
                out.push_str(name);
            }
            _ => return None,
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Locale-independent Unicode default lowercase folding.
///
/// Per-character mapping (not `str::to_lowercase`) so the result does not depend on the
/// surrounding text. `İ` (U+0130) maps to `i` + U+0307, which is intentional: it is the default
/// Unicode mapping, not the Turkish locale rule.
///
/// ASCII-only input (what a Unity project is made of, #210) takes a byte-wise path: no ASCII
/// character has a non-ASCII or multi-character default lowercase mapping, so
/// [`str::to_ascii_lowercase`] produces exactly the same bytes as the per-character loop while
/// avoiding the per-character `to_lowercase()` iterator. Anything non-ASCII falls back to the
/// general loop, so the mapping rules above are unchanged.
pub fn fold_key(path: &str) -> String {
    if path.is_ascii() {
        return path.to_ascii_lowercase();
    }
    let mut key = String::with_capacity(path.len());
    for character in path.chars() {
        for folded in character.to_lowercase() {
            key.push(folded);
        }
    }
    key
}

/// Key used for workspace lookups: case-folded on Windows, exact elsewhere.
///
/// Windows file systems are case-insensitive; the scan keeps the on-disk case for display while
/// every lookup (children/rules/events) goes through this key.
pub(crate) fn identity_key(path: &str) -> String {
    #[cfg(windows)]
    {
        fold_key(path)
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

/// Parent key of an identity key (`""` is the root). Keys are `/`-joined identity strings.
pub(crate) fn parent_key(key: &str) -> &str {
    key.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// Case-folded ordering of two workspace paths: compare their [`identity_key`]s.
///
/// UTF-8 orders byte-wise exactly as its code points order, so comparing the two folded strings
/// is the same ordering as comparing the per-character lowercase mappings element by element —
/// which is why `Snapshot::finish` folds once per entry and then compares the prepared keys
/// instead of re-folding both sides on every comparison (#210). The equivalence is pinned by
/// `folding_order_matches_the_per_character_mapping`.
#[cfg(test)]
pub(crate) fn identity_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    identity_key(left).cmp(&identity_key(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference implementation of the ordering: lowercase mapping applied character by
    /// character, compared element by element. The shipping path never runs this.
    fn per_character_cmp(left: &str, right: &str) -> std::cmp::Ordering {
        #[cfg(windows)]
        {
            left.chars()
                .flat_map(char::to_lowercase)
                .cmp(right.chars().flat_map(char::to_lowercase))
        }
        #[cfg(not(windows))]
        {
            left.cmp(right)
        }
    }

    /// Paths that exercise both the ASCII fast path and every awkward mapping around it.
    const CORPUS: &[&str] = &[
        "A.cs",
        "a.cs",
        "Assets/Player.cs",
        "Assets/player.CS",
        "Assets/Player",
        "Assets/Plays.cs",
        "assets/Zebra.cs",
        "İ.cs",
        "i.cs",
        "I.cs",
        "i\u{307}.cs",
        "\u{212A}elvin.cs", // KELVIN SIGN, folds to the ASCII `k`
        "kelvin.cs",
        "Straße.cs",
        "STRASSE.cs",
        "资源/玩家.cs",
        "资源/Player.cs",
    ];

    #[test]
    fn folding_is_lowercase_and_locale_independent() {
        assert_eq!(fold_key("Assets/Player.CS"), "assets/player.cs");
        // Turkish dotted capital I keeps the Unicode default mapping.
        assert_eq!(fold_key("İ.cs"), "i\u{307}.cs");
        assert_ne!(fold_key("İ.cs"), fold_key("i.cs"));
        assert_eq!(fold_key("I.cs"), fold_key("i.cs"));
        assert_eq!(identity_cmp("a.cs", "B.cs"), std::cmp::Ordering::Less);
    }

    #[test]
    fn folding_the_ascii_fast_path_matches_the_per_character_mapping() {
        // `fold_key` short-circuits on ASCII-only input (#210); the result must be identical.
        for path in CORPUS {
            let mut reference = String::new();
            for character in path.chars() {
                reference.extend(character.to_lowercase());
            }
            assert_eq!(fold_key(path), reference, "fold_key({path:?})");
        }
    }

    #[test]
    fn folding_order_matches_the_per_character_mapping() {
        // `Snapshot::finish` sorts prepared keys instead of folding inside the comparator;
        // that is only allowed because the two orderings agree on every pair (#210).
        for left in CORPUS {
            for right in CORPUS {
                assert_eq!(
                    identity_cmp(left, right),
                    per_character_cmp(left, right),
                    "ordering of {left:?} vs {right:?}"
                );
            }
        }
    }

    #[test]
    fn relative_paths_use_forward_slashes_and_skip_the_root() {
        let root = Path::new(r"C:\project");
        assert_eq!(
            relative_str(root, Path::new(r"C:\project\Assets\A.cs")).as_deref(),
            Some("Assets/A.cs")
        );
        assert_eq!(relative_str(root, root), None);
        assert_eq!(relative_str(root, Path::new(r"C:\other\A.cs")), None);
    }

    #[test]
    fn parent_key_of_root_is_empty_and_of_child_is_the_directory() {
        assert_eq!(parent_key(""), "");
        assert_eq!(parent_key("A.cs"), "");
        assert_eq!(parent_key("Assets/A.cs"), "Assets");
    }
}
