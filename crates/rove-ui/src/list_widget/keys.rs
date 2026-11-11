//! GPUI action bridge for the list/quick-input commands (#18).
//!
//! Each VS Code command ID gets one GPUI action. Keys are resolved by #14's keybinding service
//! (`crate::keys`) from the generated VS Code default table — the views publish the context keys
//! (`listFocus`, `inQuickInput`, `listScrollAtBoundary`...) on their key context, so the `when`
//! clauses evaluate exactly as in VS Code — and dispatched here through [`action_for`].
//! Text-input editing keys (Backspace, arrows, clipboard) are not keybindings: they are the input
//! box's native behaviour ([`native_text_input_action`]), reached only when no keybinding applies.
use gpui::{actions, Action, Context, InteractiveElement, Keystroke, Window};

macro_rules! command_actions {
    ($($group:ident { $($ty:ident => $id:literal),* $(,)? })*) => {
        actions!(rove_list, [$($($ty),*),*]);

        /// The GPUI action of a registered command ID.
        pub fn action_for(command: &str) -> Option<Box<dyn Action>> {
            match command {
                $($($id => Some(Box::new($ty)),)*)*
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

/// List defaults that are not in VS Code's table (VS Code key syntax): opened through the
/// keybinding service, so users can rebind or remove them.
pub const EXTRA_DEFAULTS: &[(&str, &str, &str)] =
    &[("shift+f10", "rove.list.contextMenu", "listFocus")];

/// Context name the text input publishes.
pub const TEXT_INPUT_CONTEXT: &str = "RoveTextInput";

/// Native editing keys of the text input (not keybindings; run only when no keybinding applied).
pub fn native_text_input_action(k: &Keystroke) -> Option<Box<dyn Action>> {
    let m = &k.modifiers;
    let plain = !m.control && !m.alt && !m.platform && !m.shift;
    let shift_only = m.shift && !m.control && !m.alt && !m.platform;
    let ctrl_only = m.control && !m.alt && !m.platform && !m.shift;
    let action: Box<dyn Action> = match k.key.as_str() {
        "backspace" if plain => Box::new(Backspace),
        "delete" if plain => Box::new(Delete),
        "left" if plain => Box::new(Left),
        "right" if plain => Box::new(Right),
        "left" if shift_only => Box::new(SelectLeft),
        "right" if shift_only => Box::new(SelectRight),
        "home" if plain => Box::new(Home),
        "end" if plain => Box::new(End),
        "a" if ctrl_only => Box::new(SelectAll),
        "v" if ctrl_only => Box::new(Paste),
        "x" if ctrl_only => Box::new(Cut),
        "c" if ctrl_only => Box::new(Copy),
        _ => return None,
    };
    Some(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_list_command_has_an_action() {
        let mut r = rove_commands::CommandRegistry::default();
        super::super::registry::register_commands(&mut r).unwrap();
        for (id, _) in r.entries() {
            assert!(action_for(id).is_some(), "{id} has no action");
        }
        for (_, command, _) in EXTRA_DEFAULTS {
            assert!(r.get(command).is_some(), "{command} is not registered");
        }
        let k = |s: &str| native_text_input_action(&Keystroke::parse(s).unwrap()).is_some();
        assert!(k("backspace") && k("shift-left") && k("ctrl-v"));
        assert!(!k("ctrl-p") && !k("enter") && !k("down"));
    }
}
