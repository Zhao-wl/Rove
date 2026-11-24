//! Explorer path commands and the active-file shortcuts (#63).

use super::Workbench;
use crate::explorer::registry::{
    self, COPY_ACTIVE_PATH, COPY_PATH, COPY_RELATIVE_PATH, REVEAL_ACTIVE_PATH, REVEAL_PATH,
};
use crate::shell::feedback::{Reason, Severity};
use gpui::{ClipboardItem, Context};
use rove_base::WorkspacePath;
use rove_config::{Key, Scope};
use rove_i18n::Message;
use std::path::{Path, PathBuf};
use std::{io, sync::Arc};

const COPY_SEPARATOR: Key<String> = Key::new("explorer.copyPathSeparator");
const RELATIVE_SEPARATOR: Key<String> = Key::new("explorer.copyRelativePathSeparator");

pub(super) fn handles(id: &str) -> bool {
    matches!(
        id,
        COPY_PATH | COPY_RELATIVE_PATH | COPY_ACTIVE_PATH | REVEAL_PATH | REVEAL_ACTIVE_PATH
    )
}

impl Workbench {
    pub(super) fn explorer_path_command(
        &mut self,
        id: &str,
        from_tree: bool,
        cx: &mut Context<Self>,
    ) {
        let paths = if from_tree && !matches!(id, COPY_ACTIVE_PATH | REVEAL_ACTIVE_PATH) {
            let explorer = self.explorer.read(cx);
            let mut selected = explorer.selected(cx);
            let focus = explorer.focused_path(cx);
            if let (Some(row), Some(focus)) = (explorer.focused_row(cx), focus) {
                if selected.is_empty() {
                    selected.push(focus.as_str().to_owned());
                } else if let Some(id) = selected
                    .iter_mut()
                    .find(|id| id.as_str() == row.id.0.as_ref())
                {
                    *id = focus.as_str().to_owned();
                }
            }
            selected
                .iter()
                .filter_map(|path| WorkspacePath::new(path).ok())
                .map(|path| self.root.join(path.as_str()))
                .collect::<Vec<_>>()
        } else {
            self.editor
                .active()
                .current()
                .and_then(|tab| self.disk_path(&tab.resource))
                .into_iter()
                .collect()
        };
        if paths.is_empty() {
            return;
        }
        if matches!(id, REVEAL_PATH | REVEAL_ACTIVE_PATH) {
            self.reveal_path_in_explorer(paths[0].clone(), cx);
            return;
        }
        let relative = id == COPY_RELATIVE_PATH;
        let separator = if relative {
            &RELATIVE_SEPARATOR
        } else {
            &COPY_SEPARATOR
        };
        let separator = separator
            .read(&*self.settings, &Scope::default())
            .unwrap_or_default();
        cx.write_to_clipboard(ClipboardItem::new_string(format_paths(
            &paths, &self.root, relative, &separator,
        )));
    }

    fn reveal_path_in_explorer(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let task = cx
            .background_executor()
            .spawn(async move { reveal_in_explorer(&path) });
        cx.spawn(async move |this, cx| {
            if let Err(error) = task.await {
                eprintln!("explorer reveal: {error}");
                let _ = this.update(cx, |view, cx| {
                    let _ = view.shell.feedback.report(
                        registry::REVEAL_FAILED,
                        Reason::Global(Severity::Error),
                        Message::key(registry::REVEAL_FAILED),
                        Arc::from([]),
                    );
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

fn format_paths(paths: &[PathBuf], root: &Path, relative: bool, separator: &str) -> String {
    let separator = match separator {
        "/" => '/',
        "\\" => '\\',
        _ => std::path::MAIN_SEPARATOR,
    };
    paths
        .iter()
        .map(|path| {
            let path = if relative {
                path.strip_prefix(root).unwrap_or(path)
            } else {
                path
            };
            if separator == '/' {
                path.to_string_lossy().replace('\\', "/")
            } else {
                path.to_string_lossy().replace('/', "\\")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(windows)]
fn reveal_in_explorer(path: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;

    let windows = std::env::var_os("SystemRoot").ok_or(io::ErrorKind::NotFound)?;
    let explorer = PathBuf::from(windows).join("explorer.exe");
    std::process::Command::new(explorer)
        .raw_arg(select_argument(path))
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
fn reveal_in_explorer(_path: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(windows)]
fn select_argument(path: &Path) -> std::ffi::OsString {
    // Explorer's command-line parser uses commas as switch delimiters.
    // Keep every path quoted, including names containing a comma with no spaces.
    let mut argument = std::ffi::OsString::from("/select,\"");
    argument.push(path.as_os_str());
    argument.push("\"");
    argument
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_multiple_paths_with_the_selected_separator() {
        let root = PathBuf::from("workspace");
        let paths = [root.join("Assets/脚本.cs"), root.join("Settings.json")];
        assert_eq!(
            format_paths(&paths, &root, true, "\\"),
            "Assets\\脚本.cs\nSettings.json"
        );
        assert_eq!(
            format_paths(&paths, &root, true, "/"),
            "Assets/脚本.cs\nSettings.json"
        );
        assert_eq!(
            format_paths(&paths, &root, false, "/"),
            "workspace/Assets/脚本.cs\nworkspace/Settings.json"
        );
        #[cfg(windows)]
        assert_eq!(
            format_paths(&paths, &root, true, "auto"),
            "Assets\\脚本.cs\nSettings.json"
        );
    }

    #[cfg(windows)]
    #[test]
    fn quotes_explorer_selection_paths_with_unicode_spaces_and_comma() {
        let path = Path::new(r"C:\临时 目录\foo,bar.cs");
        assert_eq!(
            select_argument(path),
            r#"/select,"C:\临时 目录\foo,bar.cs""#
        );
    }
}
