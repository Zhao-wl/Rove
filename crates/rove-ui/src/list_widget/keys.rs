//! GPUI action bridge for the list/quick-input commands (#18).
//!
//! Each VS Code command ID gets one GPUI action; [`bind_default_keys`] installs the
//! KEYBINDINGS.md rows with their `when` clause translated into a GPUI key-context predicate
//! (identical operators; quotes dropped). The views publish the context keys on their key
//! context, so `ctrl+up` / `ctrl+down` stop at `listScrollAtBoundary` exactly as the when-clause
//! says. When #14's resolver lands it can call `ListView::run_command` directly instead.
use super::registry::KEYBINDINGS;
use gpui::{actions, App, Context, InteractiveElement, KeyBinding, Window};

macro_rules! command_actions {
    ($($group:ident { $($ty:ident => $id:literal),* $(,)? })*) => {
        actions!(rove_list, [$($($ty),*),*]);

        /// Builds the GPUI binding for a registered command ID.
        pub fn binding_for(command: &str, keystrokes: &str, context: Option<&str>) -> Option<KeyBinding> {
            match command {
                $($($id => Some(KeyBinding::new(keystrokes, $ty, context)),)*)*
                _ => None,
            }
        }

        $(
            /// Attaches one handler per command of this group.
            pub fn $group<V: 'static, E: InteractiveElement>(
                mut element: E,
                run: fn(&mut V, &'static str, &mut Window, &mut Context<V>),
                cx: &mut Context<V>,
            ) -> E {
                $(element = element.on_action(cx.listener(move |v, _: &$ty, w, cx| run(v, $id, w, cx)));)*
                element
            }
        )*
    };
}

command_actions! {
    list_actions {
        ListClear => "list.clear",
        ListCloseFind => "list.closeFind",
        ListCollapse => "list.collapse",
        ListCollapseAll => "list.collapseAll",
        ListExpand => "list.expand",
        ListExpandSelectionDown => "list.expandSelectionDown",
        ListExpandSelectionUp => "list.expandSelectionUp",
        ListFind => "list.find",
        ListFocusAnyDown => "list.focusAnyDown",
        ListFocusAnyFirst => "list.focusAnyFirst",
        ListFocusAnyLast => "list.focusAnyLast",
        ListFocusAnyUp => "list.focusAnyUp",
        ListFocusDown => "list.focusDown",
        ListFocusFirst => "list.focusFirst",
        ListFocusLast => "list.focusLast",
        ListFocusPageDown => "list.focusPageDown",
        ListFocusPageUp => "list.focusPageUp",
        ListFocusUp => "list.focusUp",
        ListScrollDown => "list.scrollDown",
        ListScrollUp => "list.scrollUp",
        ListSelect => "list.select",
        ListSelectAll => "list.selectAll",
        ListShowHover => "list.showHover",
        ListStickyCollapse => "list.stickyScroll.collapse",
        ListStickySelect => "list.stickyScrollselect",
        ListStickyToggleExpand => "list.stickyScrolltoggleExpand",
        ListToggleExpand => "list.toggleExpand",
        ListToggleSelection => "list.toggleSelection",
        ListTriggerTypeNavigation => "list.triggerTypeNavigation",
        ListContextMenu => "rove.list.contextMenu",
    }
    quick_input_actions {
        QuickAccept => "quickInput.accept",
        QuickAcceptInBackground => "quickInput.acceptInBackground",
        QuickFirst => "quickInput.first",
        QuickHide => "quickInput.hide",
        QuickLast => "quickInput.last",
        QuickNext => "quickInput.next",
        QuickNextSeparator => "quickInput.nextSeparator",
        QuickNextSeparatorFallback => "quickInput.nextSeparatorWithQuickAccessFallback",
        QuickPageNext => "quickInput.pageNext",
        QuickPagePrevious => "quickInput.pagePrevious",
        QuickPrevious => "quickInput.previous",
        QuickPreviousSeparator => "quickInput.previousSeparator",
        QuickPreviousSeparatorFallback => "quickInput.previousSeparatorWithQuickAccessFallback",
        QuickToggleCheckbox => "quickInput.toggleCheckbox",
        QuickToggleHover => "quickInput.toggleHover",
    }
    history_actions {
        HistoryShowNext => "history.showNext",
        HistoryShowPrevious => "history.showPrevious",
    }
}

actions!(
    rove_text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
    ]
);

/// `ctrl+shift+enter` → `ctrl-shift-enter`; chords stay space separated.
pub fn gpui_keystrokes(vscode: &str) -> String {
    vscode
        .split(' ')
        .map(|part| part.replace('+', "-"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// VS Code when-clause → GPUI predicate. The operators used by the list/quick-input bindings
/// (`&&`, `||`, `!`, `==`, `!=`) have the same precedence in both; string literals lose quotes.
pub fn gpui_predicate(when: &str) -> String {
    when.replace('\'', "")
}

/// Context name the text input publishes; the generic `Right` yields to
/// `quickInput.acceptInBackground` when the caret is at the end of a quick input box.
pub const TEXT_INPUT_CONTEXT: &str = "RoveTextInput";

/// Installs the default list / quick-input / text-input bindings on the app keymap.
pub fn bind_default_keys(cx: &mut App) {
    let mut bindings: Vec<KeyBinding> = KEYBINDINGS
        .iter()
        .filter_map(|(key, command, when)| {
            let predicate = gpui_predicate(when);
            binding_for(command, &gpui_keystrokes(key), Some(&predicate))
        })
        .collect();
    bindings.push(KeyBinding::new(
        "shift-f10",
        ListContextMenu,
        Some("listFocus"),
    ));
    let input = Some(TEXT_INPUT_CONTEXT);
    bindings.extend([
        KeyBinding::new("backspace", Backspace, input),
        KeyBinding::new("delete", Delete, input),
        KeyBinding::new("left", Left, input),
        KeyBinding::new(
            "right",
            Right,
            Some("RoveTextInput && !cursorAtEndOfQuickInputBox"),
        ),
        KeyBinding::new("shift-left", SelectLeft, input),
        KeyBinding::new("shift-right", SelectRight, input),
        KeyBinding::new("ctrl-a", SelectAll, input),
        KeyBinding::new("home", Home, input),
        KeyBinding::new("end", End, input),
        KeyBinding::new("ctrl-v", Paste, input),
        KeyBinding::new("ctrl-x", Cut, input),
        KeyBinding::new("ctrl-c", Copy, input),
    ]);
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::KeyBindingContextPredicate;

    #[test]
    fn every_default_binding_translates() {
        for (key, command, when) in KEYBINDINGS {
            assert!(
                binding_for(command, &gpui_keystrokes(key), None).is_some(),
                "{command} has no action"
            );
            KeyBindingContextPredicate::parse(&gpui_predicate(when))
                .unwrap_or_else(|e| panic!("{when}: {e}"));
        }
        assert_eq!(gpui_keystrokes("ctrl+k ctrl+i"), "ctrl-k ctrl-i");
    }
}
