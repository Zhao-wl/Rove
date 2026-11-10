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
pub fn fold_key(path: &str) -> String {
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

/// Allocation-free case-folded ordering, used to sort snapshots deterministically on Windows.
pub(crate) fn identity_cmp(left: &str, right: &str) -> std::cmp::Ordering {
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

#[cfg(test)]
mod tests {
    use super::*;

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
