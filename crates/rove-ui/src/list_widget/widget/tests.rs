//! One test (or more) per `list.*` command, asserting focus/selection/expansion like VS Code.
use super::*;
use crate::list_widget::{
    matcher::{FindMode, MatchType},
    options::{IndentGuides, ListOptions},
    plain_row,
    tree::tests::PathSource,
    StaticList,
};

const H: f32 = 20.0;

fn labels(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("item{i}")).collect()
}

fn flat_with(n: usize, options: ListOptions) -> ListWidget {
    let l = labels(n);
    let mut w = ListWidget::new(
        Arc::new(StaticList::from_labels(l.iter().map(|s| s.as_str()))),
        RowHeight::Fixed(H),
        options,
    );
    w.set_viewport(5.0 * H, 200.0);
    w
}

fn flat(n: usize) -> ListWidget {
    flat_with(n, ListOptions::default())
}

fn tree_with(paths: &[&'static str], options: ListOptions) -> ListWidget {
    let mut w = ListWidget::tree(
        TreeModel::new(Arc::new(PathSource::new(paths))),
        RowHeight::Fixed(H),
        options,
    );
    w.set_viewport(5.0 * H, 200.0);
    w
}

const PATHS: &[&str] = &[
    "a",
    "a/a1.cs",
    "a/a2",
    "a/a2/x.cs",
    "a/a3.cs",
    "b",
    "b/b1.cs",
    "c.cs",
];

fn tree() -> ListWidget {
    tree_with(PATHS, ListOptions::default())
}

fn shown(w: &ListWidget) -> Vec<String> {
    w.display_rows(0..w.len())
        .iter()
        .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), r.label))
        .collect()
}

fn focused_label(w: &ListWidget) -> Option<String> {
    w.focus()
        .and_then(|f| w.row(f))
        .map(|r| r.label.to_string())
}

fn run(w: &mut ListWidget, c: &str) {
    w.execute(c).unwrap_or_else(|e| panic!("{c}: {e}"));
}

fn key(w: &ListWidget, k: &str) -> Value {
    w.context_keys()
        .into_iter()
        .find(|(n, _)| *n == k)
        .map(|(_, v)| v)
        .unwrap_or(Value::Null)
}

#[test]
fn focus_down_up_start_at_first_and_do_not_wrap() {
    let mut w = flat(10);
    run(&mut w, "list.focusDown");
    assert_eq!(w.focus(), Some(0), "no focus => first row");
    run(&mut w, "list.focusDown");
    assert_eq!(w.focus(), Some(1));
    run(&mut w, "list.focusUp");
    run(&mut w, "list.focusUp");
    assert_eq!(w.focus(), Some(0), "no wrap at top");
    run(&mut w, "list.focusLast");
    run(&mut w, "list.focusDown");
    assert_eq!(w.focus(), Some(9), "no wrap at bottom");
    assert_eq!(w.scroll_top(), 5.0 * H, "focus is revealed");
    let mut empty = flat(0);
    run(&mut empty, "list.focusDown");
    assert_eq!(empty.focus(), None);
}

#[test]
fn focus_first_and_last_reveal() {
    let mut w = flat(100);
    run(&mut w, "list.focusLast");
    assert_eq!(w.focus(), Some(99));
    assert_eq!(w.scroll_top(), 95.0 * H);
    run(&mut w, "list.focusFirst");
    assert_eq!((w.focus(), w.scroll_top()), (Some(0), 0.0));
}

#[test]
fn focus_any_moves_without_selection_following() {
    let mut w = flat(10);
    w.follow_focus = true;
    run(&mut w, "list.focusDown");
    assert_eq!(w.selection_indices(), [0]);
    assert!(w.take_events().iter().any(|e| matches!(
        e,
        ListEvent::Open {
            preserve_focus: true,
            ..
        }
    )));
    run(&mut w, "list.focusAnyDown");
    assert_eq!((w.focus(), w.selection_indices()), (Some(1), &[0][..]));
    assert!(!w
        .take_events()
        .iter()
        .any(|e| matches!(e, ListEvent::Open { .. })));
    run(&mut w, "list.focusAnyLast");
    assert_eq!(w.focus(), Some(9));
    run(&mut w, "list.focusAnyUp");
    assert_eq!(w.focus(), Some(8));
    run(&mut w, "list.focusAnyFirst");
    assert_eq!((w.focus(), w.selection_indices()), (Some(0), &[0][..]));
}

#[test]
fn page_down_and_up_follow_vscode_two_step_paging() {
    let mut w = flat(20);
    run(&mut w, "list.focusFirst");
    run(&mut w, "list.focusPageDown");
    assert_eq!(w.focus(), Some(4), "last fully visible row first");
    run(&mut w, "list.focusPageDown");
    assert_eq!(
        (w.focus(), w.scroll_top()),
        (Some(9), 5.0 * H),
        "then a page further"
    );
    run(&mut w, "list.focusPageUp");
    assert_eq!(w.focus(), Some(5), "first visible row first");
    run(&mut w, "list.focusPageUp");
    assert_eq!((w.focus(), w.scroll_top()), (Some(0), 0.0));
    run(&mut w, "list.focusLast");
    run(&mut w, "list.focusPageDown");
    assert_eq!(w.focus(), Some(19), "stays at the end");
}

#[test]
fn scroll_up_down_move_ten_pixels_and_report_boundaries() {
    let mut w = flat(20);
    assert_eq!(w.scroll_boundary(), ScrollBoundary::Top);
    assert_eq!(key(&w, "listScrollAtBoundary"), Value::String("top".into()));
    run(&mut w, "list.scrollDown");
    assert_eq!(w.scroll_top(), 10.0);
    assert_eq!(w.scroll_boundary(), ScrollBoundary::None);
    run(&mut w, "list.scrollUp");
    assert_eq!(w.scroll_top(), 0.0);
    w.set_scroll_top(1e9);
    assert_eq!(w.scroll_top(), 15.0 * H);
    assert_eq!(w.scroll_boundary(), ScrollBoundary::Bottom);
    run(&mut w, "list.scrollDown");
    assert_eq!(w.scroll_top(), 15.0 * H, "clamped at bottom");
    let short = flat(3);
    assert_eq!(
        short.scroll_boundary(),
        ScrollBoundary::Both,
        "content shorter than viewport"
    );
    let empty = flat(0);
    assert_eq!(empty.scroll_boundary(), ScrollBoundary::Both);
}

#[test]
fn select_opens_pinned_and_selects_focus() {
    let mut w = flat(5);
    assert!(!w.execute("list.select").unwrap(), "nothing focused");
    run(&mut w, "list.focusDown");
    run(&mut w, "list.focusDown");
    w.take_events();
    run(&mut w, "list.select");
    assert_eq!(w.selection_indices(), [1]);
    let events = w.take_events();
    assert!(events.contains(&ListEvent::Open {
        ids: Arc::from([ItemId("item1".into())]),
        preserve_focus: false,
        pinned: true,
        side_by_side: false
    }));
    assert!(events
        .iter()
        .any(|e| matches!(e, ListEvent::SelectionChanged(ids) if ids.len() == 1)));
}

#[test]
fn select_on_tree_folder_toggles_without_opening() {
    let mut w = tree();
    run(&mut w, "list.focusFirst");
    w.take_events();
    run(&mut w, "list.select");
    assert_eq!(shown(&w)[1], "  a1.cs");
    assert!(!w
        .take_events()
        .iter()
        .any(|e| matches!(e, ListEvent::Open { .. })));
    run(&mut w, "list.select");
    assert_eq!(shown(&w), ["a", "b", "c.cs"]);
}

#[test]
fn clear_removes_selection_then_focus() {
    let mut w = flat(5);
    w.multi_select = true;
    run(&mut w, "list.focusDown");
    run(&mut w, "list.selectAll");
    assert_eq!(key(&w, "listHasSelectionOrFocus"), Value::Bool(true));
    run(&mut w, "list.clear");
    assert!(w.selection_indices().is_empty());
    assert_eq!(w.focus(), Some(0), "focus survives the first escape");
    run(&mut w, "list.clear");
    assert_eq!(w.focus(), None);
    assert_eq!(key(&w, "listHasSelectionOrFocus"), Value::Bool(false));
}

#[test]
fn select_all_requires_multiselect() {
    let mut w = flat(5);
    run(&mut w, "list.focusDown");
    assert!(!w.execute("list.selectAll").unwrap());
    assert!(w.selection_indices().is_empty());
    w.multi_select = true;
    run(&mut w, "list.selectAll");
    assert_eq!(w.selection_indices(), [0, 1, 2, 3, 4]);
}

#[test]
fn select_all_in_tree_widens_scope_on_repeat() {
    let mut w = tree();
    w.multi_select = true;
    run(&mut w, "list.focusFirst");
    run(&mut w, "list.expand"); // a
    run(&mut w, "list.focusDown"); // a1.cs
    run(&mut w, "list.selectAll");
    // Siblings under "a": a1.cs, a2, a3.cs.
    let sel: Vec<String> = w.selected_ids().iter().map(|i| i.0.to_string()).collect();
    assert_eq!(sel, ["a/a1.cs", "a/a2", "a/a3.cs"]);
    run(&mut w, "list.selectAll");
    assert_eq!(
        w.selection_indices().len(),
        w.len(),
        "second press selects the root scope"
    );
}

#[test]
fn toggle_selection_adds_and_removes_focus() {
    let mut w = flat(5);
    w.multi_select = true;
    run(&mut w, "list.focusDown");
    run(&mut w, "list.toggleSelection");
    run(&mut w, "list.focusDown");
    run(&mut w, "list.focusDown");
    run(&mut w, "list.toggleSelection");
    assert_eq!(w.selection_indices(), [0, 2]);
    run(&mut w, "list.toggleSelection");
    assert_eq!(w.selection_indices(), [0]);
}

#[test]
fn expand_selection_extends_from_the_anchor() {
    let mut w = flat(10);
    run(&mut w, "list.focusDown");
    assert!(
        !w.execute("list.expandSelectionDown").unwrap(),
        "needs multiselect"
    );
    w.multi_select = true;
    w.click(3, Modifiers::default(), 1, false);
    run(&mut w, "list.expandSelectionDown");
    run(&mut w, "list.expandSelectionDown");
    assert_eq!(
        (w.focus(), w.selection_indices()),
        (Some(5), &[3, 4, 5][..])
    );
    run(&mut w, "list.expandSelectionUp");
    run(&mut w, "list.expandSelectionUp");
    run(&mut w, "list.expandSelectionUp");
    assert_eq!((w.focus(), w.selection_indices()), (Some(2), &[2, 3][..]));
}

#[test]
fn show_hover_targets_the_focused_item() {
    let mut w = flat(3);
    assert!(!w.execute("list.showHover").unwrap());
    run(&mut w, "list.focusDown");
    w.take_events();
    run(&mut w, "list.showHover");
    assert_eq!(
        w.take_events(),
        [ListEvent::ShowHover(ItemId("item0".into()))]
    );
}

#[test]
fn expand_then_focus_first_child() {
    let mut w = tree();
    run(&mut w, "list.focusFirst");
    assert_eq!(key(&w, "treeElementCanExpand"), Value::Bool(true));
    run(&mut w, "list.expand");
    assert_eq!(shown(&w), ["a", "  a1.cs", "  a2", "  a3.cs", "b", "c.cs"]);
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("a"),
        "first press only expands"
    );
    assert_eq!(key(&w, "treeElementHasChild"), Value::Bool(true));
    run(&mut w, "list.expand");
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("a1.cs"),
        "second press enters"
    );
    run(&mut w, "list.expand");
    assert_eq!(focused_label(&w).as_deref(), Some("a1.cs"), "leaf: no-op");
}

#[test]
fn collapse_then_focus_parent() {
    let mut w = tree();
    run(&mut w, "list.focusFirst");
    run(&mut w, "list.expand");
    run(&mut w, "list.expand");
    assert_eq!(key(&w, "treeElementHasParent"), Value::Bool(true));
    run(&mut w, "list.collapse");
    assert_eq!(focused_label(&w).as_deref(), Some("a"), "leaf => parent");
    assert_eq!(key(&w, "treeElementCanCollapse"), Value::Bool(true));
    run(&mut w, "list.collapse");
    assert_eq!(shown(&w), ["a", "b", "c.cs"]);
    run(&mut w, "list.collapse");
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("a"),
        "root collapsed: no-op"
    );
}

#[test]
fn collapse_all_keeps_focus_on_outermost_ancestor() {
    let mut w = tree();
    run(&mut w, "list.focusFirst");
    run(&mut w, "list.expand");
    w.focus_id(&ItemId("a/a2".into())).unwrap();
    run(&mut w, "list.expand");
    run(&mut w, "list.expand");
    assert_eq!(focused_label(&w).as_deref(), Some("x.cs"));
    run(&mut w, "list.collapseAll");
    assert_eq!(shown(&w), ["a", "b", "c.cs"]);
    assert_eq!(focused_label(&w).as_deref(), Some("a"));
}

#[test]
fn toggle_expand_toggles_folders_and_selects_leaves() {
    let mut w = tree();
    run(&mut w, "list.focusFirst");
    run(&mut w, "list.toggleExpand");
    assert_eq!(w.len(), 6);
    run(&mut w, "list.toggleExpand");
    assert_eq!(w.len(), 3);
    run(&mut w, "list.focusLast");
    w.take_events();
    run(&mut w, "list.toggleExpand");
    assert_eq!(w.selection_indices(), [2]);
    assert!(w.take_events().iter().any(|e| matches!(
        e,
        ListEvent::Open {
            preserve_focus: true,
            ..
        }
    )));
}

#[test]
fn find_opens_only_where_supported_and_close_find_restores() {
    let mut w = flat(5);
    assert_eq!(key(&w, "treeFindOpen"), Value::Bool(false));
    run(&mut w, "list.find");
    assert_eq!(key(&w, "treeFindOpen"), Value::Bool(true));
    w.set_find_query("item3");
    run(&mut w, "list.closeFind");
    assert_eq!(key(&w, "treeFindOpen"), Value::Bool(false));
    assert!(w.highlights(3).is_empty());
    assert!(!w.execute("list.closeFind").unwrap(), "already closed");
    w.supports_find = false;
    assert!(!w.execute("list.find").unwrap());
    assert_eq!(key(&w, "listSupportsFind"), Value::Bool(false));
}

#[test]
fn sticky_scroll_commands_act_on_the_pinned_ancestor() {
    let mut paths: Vec<&'static str> = vec!["root"];
    let names: Vec<&'static str> = (0..30)
        .map(|i| &*Box::leak(format!("root/f{i:02}.cs").into_boxed_str()))
        .collect();
    paths.extend(names);
    let mut w = tree_with(&paths, ListOptions::default());
    run(&mut w, "list.focusFirst");
    run(&mut w, "list.expand");
    w.set_scroll_top(10.0 * H);
    assert_eq!(
        w.sticky_rows(),
        [0],
        "root pinned while its children scroll"
    );
    // Focus the first row under the sticky band, then Up enters sticky focus.
    let under = w.heights.index_at(w.scroll_top() + H);
    w.set_focus(Some(under));
    run(&mut w, "list.focusUp");
    assert_eq!(key(&w, "treestickyScrollFocused"), Value::Bool(true));
    assert_eq!(w.sticky_focus(), Some(0));
    run(&mut w, "list.stickyScrolltoggleExpand");
    assert_eq!(w.len(), 1, "collapsed via sticky");
    assert_eq!(key(&w, "treestickyScrollFocused"), Value::Bool(false));
    run(&mut w, "list.expand");
    w.set_scroll_top(10.0 * H);
    w.focus_sticky(0);
    run(&mut w, "list.stickyScroll.collapse");
    assert_eq!((w.len(), w.focus()), (1, Some(0)));
    run(&mut w, "list.expand");
    w.set_scroll_top(10.0 * H);
    w.focus_sticky(0);
    w.take_events();
    run(&mut w, "list.stickyScrollselect");
    assert_eq!((w.focus(), w.scroll_top()), (Some(0), 0.0), "revealed");
    assert_eq!(w.selection_indices(), [0]);
    let off = ListOptions {
        sticky_scroll: false,
        ..Default::default()
    };
    w.set_options(off);
    w.set_scroll_top(10.0 * H);
    assert!(w.sticky_rows().is_empty(), "enableStickyScroll = false");
}

#[test]
fn highlight_and_filter_modes_with_both_match_types() {
    let rows = ["Player.cs", "PlayerInput.cs", "Enemy.cs", "pyramid.txt"];
    let make = |mode: FindMode, ty: MatchType| {
        let mut w = ListWidget::new(
            Arc::new(StaticList::from_labels(rows)),
            RowHeight::Fixed(H),
            ListOptions {
                find_mode: mode,
                find_match_type: ty,
                ..Default::default()
            },
        );
        w.set_viewport(100.0, 100.0);
        run(&mut w, "list.find");
        w
    };
    // highlight + contiguous: everything stays, only exact substrings highlight.
    let mut w = make(FindMode::Highlight, MatchType::Contiguous);
    w.set_find_query("pyr");
    assert_eq!(w.len(), 4);
    assert_eq!(w.highlights(3), vec![0..3]);
    assert!(w.highlights(0).is_empty());
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("pyramid.txt"),
        "focus jumps to first match"
    );
    // highlight + fuzzy: subsequence highlights match the letters.
    let mut w = make(FindMode::Highlight, MatchType::Fuzzy);
    w.set_find_query("pyr");
    assert_eq!(w.highlights(0), vec![0..1, 3..4, 5..6]);
    // filter + contiguous: non-matching rows hidden.
    let mut w = make(FindMode::Filter, MatchType::Contiguous);
    w.set_find_query("player");
    assert_eq!(shown(&w), ["Player.cs", "PlayerInput.cs"]);
    assert_eq!(w.highlights(1), vec![0..6]);
    w.set_find_query("zzz");
    assert!(!w.has_matches());
    // filter + fuzzy.
    let mut w = make(FindMode::Filter, MatchType::Fuzzy);
    w.set_find_query("pyr");
    assert_eq!(shown(&w), ["Player.cs", "PlayerInput.cs", "pyramid.txt"]);
    // Toggles on the find widget switch both dimensions.
    w.toggle_find_match_type();
    assert_eq!(shown(&w), ["pyramid.txt"]);
    w.toggle_find_mode();
    assert_eq!(w.len(), 4);
}

#[test]
fn filtering_keeps_focus_and_selection_by_identity() {
    let mut w = flat_with(
        10,
        ListOptions {
            find_mode: FindMode::Filter,
            find_match_type: MatchType::Contiguous,
            ..Default::default()
        },
    );
    w.multi_select = true;
    w.click(7, Modifiers::default(), 1, false);
    run(&mut w, "list.find");
    w.set_find_query("7");
    assert_eq!((w.len(), focused_label(&w).as_deref()), (1, Some("item7")));
    run(&mut w, "list.focusDown");
    assert_eq!(
        w.focus(),
        Some(0),
        "navigation stays inside the filtered rows"
    );
    run(&mut w, "list.closeFind");
    assert_eq!(
        (w.len(), w.focus(), w.selection_indices()),
        (10, Some(7), &[7][..])
    );
}

#[test]
fn tree_filter_auto_expands_and_restores_on_close() {
    let mut w = tree_with(
        PATHS,
        ListOptions {
            find_mode: FindMode::Filter,
            find_match_type: MatchType::Contiguous,
            ..Default::default()
        },
    );
    // Load "a" and "a/a2" once, then collapse everything (VS Code filters loaded nodes).
    w.expand_id(&ItemId("a".into()), true).unwrap();
    w.expand_id(&ItemId("a/a2".into()), true).unwrap();
    run(&mut w, "list.focusFirst");
    run(&mut w, "list.collapseAll");
    assert_eq!(shown(&w), ["a", "b", "c.cs"]);
    run(&mut w, "list.find");
    w.set_find_query("x.cs");
    assert_eq!(shown(&w), ["a", "  a2", "    x.cs"]);
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("x.cs"),
        "first match focused"
    );
    run(&mut w, "list.focusFirst");
    run(&mut w, "list.focusDown");
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("a2"),
        "navigation inside filtered rows"
    );
    run(&mut w, "list.focusLast");
    run(&mut w, "list.closeFind");
    assert_eq!(shown(&w), ["a", "b", "c.cs"], "user expansion restored");
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("a"),
        "a focus hidden by the restore falls back to its nearest visible ancestor"
    );
}

#[test]
fn multi_select_modifier_setting_switches_ctrl_and_alt() {
    let ctrl = Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    let shift = Modifiers {
        shift: true,
        ..Default::default()
    };
    let mut w = flat(10);
    w.multi_select = true;
    w.click(1, Modifiers::default(), 1, false);
    w.click(3, ctrl, 1, false);
    assert_eq!(w.selection_indices(), [1, 3], "ctrlCmd: ctrl adds");
    w.click(6, shift, 1, false);
    assert_eq!(
        w.selection_indices(),
        [3, 4, 5, 6],
        "shift: range from anchor"
    );
    w.take_events();
    w.click(8, alt, 1, false);
    assert_eq!(
        w.selection_indices(),
        [8],
        "alt is not the multi-select modifier"
    );
    assert!(w.take_events().iter().any(|e| matches!(
        e,
        ListEvent::Open {
            side_by_side: true,
            ..
        }
    )));

    let mut w = flat_with(
        10,
        ListOptions {
            multi_select_modifier: MultiSelectModifier::Alt,
            ..Default::default()
        },
    );
    w.multi_select = true;
    w.click(1, Modifiers::default(), 1, false);
    w.click(3, alt, 1, false);
    assert_eq!(w.selection_indices(), [1, 3], "alt: alt adds");
    w.take_events();
    w.click(5, ctrl, 1, false);
    assert_eq!(w.selection_indices(), [5]);
    assert!(w.take_events().iter().any(|e| matches!(
        e,
        ListEvent::Open {
            side_by_side: true,
            ..
        }
    )));
}

#[test]
fn type_navigation_automatic_and_trigger() {
    let rows = ["alpha", "beta", "bravo", "charlie"];
    let mut w = ListWidget::new(
        Arc::new(StaticList::from_labels(rows)),
        RowHeight::Fixed(H),
        ListOptions::default(),
    );
    assert!(w.type_char("b", 0.0));
    assert_eq!(focused_label(&w).as_deref(), Some("beta"));
    assert!(w.type_char("r", 100.0));
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("bravo"),
        "prefix accumulates"
    );
    assert!(w.type_char("b", 2000.0));
    assert_eq!(
        focused_label(&w).as_deref(),
        Some("beta"),
        "timeout restarts, cycles"
    );
    let mut t = ListWidget::new(
        Arc::new(StaticList::from_labels(rows)),
        RowHeight::Fixed(H),
        ListOptions {
            type_navigation_mode: TypeNavigationMode::Trigger,
            ..Default::default()
        },
    );
    assert!(!t.type_char("c", 0.0), "trigger mode ignores plain typing");
    run(&mut t, "list.triggerTypeNavigation");
    assert!(t.type_char("c", 10.0));
    assert_eq!(focused_label(&t).as_deref(), Some("charlie"));
}

/// Every P0 `workbench.list.*` / `workbench.tree.*` setting has an observable effect.
#[test]
fn p0_settings_take_effect() {
    type Check = fn(ListOptions) -> bool;
    let cases: Vec<(&str, ListOptions, Check)> = vec![
        (
            "defaultFindMatchType",
            ListOptions {
                find_match_type: MatchType::Contiguous,
                ..Default::default()
            },
            |o| {
                let mut w = flat_with(10, o);
                run(&mut w, "list.find");
                w.set_find_query("i1");
                w.highlights(1).is_empty() // "item1" has no "i1" substring
            },
        ),
        (
            "defaultFindMode",
            ListOptions {
                find_mode: FindMode::Filter,
                ..Default::default()
            },
            |o| {
                let mut w = flat_with(10, o);
                run(&mut w, "list.find");
                w.set_find_query("item1");
                w.len() == 1
            },
        ),
        (
            "fastScrollSensitivity",
            ListOptions {
                fast_scroll_sensitivity: 3.0,
                ..Default::default()
            },
            |o| {
                let mut w = flat_with(100, o);
                w.wheel(
                    0.0,
                    10.0,
                    Modifiers {
                        alt: true,
                        ..Default::default()
                    },
                );
                w.scroll_top() == 30.0
            },
        ),
        (
            "horizontalScrolling",
            ListOptions {
                horizontal_scrolling: true,
                ..Default::default()
            },
            |o| {
                let shift = Modifiers {
                    shift: true,
                    ..Default::default()
                };
                let mut w = flat_with(10, o.clone());
                w.set_content_width(500.0);
                w.wheel(0.0, 40.0, shift);
                let mut off = flat_with(10, ListOptions::default());
                off.set_content_width(500.0);
                off.wheel(0.0, 40.0, shift);
                w.scroll_left() == 40.0 && off.scroll_left() == 0.0
            },
        ),
        (
            "mouseWheelScrollSensitivity",
            ListOptions {
                mouse_wheel_scroll_sensitivity: 2.0,
                ..Default::default()
            },
            |o| {
                let mut w = flat_with(100, o);
                w.wheel(0.0, 10.0, Modifiers::default());
                w.scroll_top() == 20.0
            },
        ),
        (
            "multiSelectModifier",
            ListOptions {
                multi_select_modifier: MultiSelectModifier::Alt,
                ..Default::default()
            },
            |o| {
                let mut w = flat_with(10, o);
                w.multi_select = true;
                w.click(1, Modifiers::default(), 1, false);
                w.click(
                    2,
                    Modifiers {
                        alt: true,
                        ..Default::default()
                    },
                    1,
                    false,
                );
                w.selection_indices() == [1, 2]
            },
        ),
        (
            "openMode",
            ListOptions {
                open_mode: OpenMode::DoubleClick,
                ..Default::default()
            },
            |o| {
                let mut w = flat_with(10, o);
                w.click(1, Modifiers::default(), 1, false);
                let single = w
                    .take_events()
                    .iter()
                    .any(|e| matches!(e, ListEvent::Open { .. }));
                w.click(1, Modifiers::default(), 2, false);
                let double = w
                    .take_events()
                    .iter()
                    .any(|e| matches!(e, ListEvent::Open { pinned: true, .. }));
                !single && double
            },
        ),
        (
            "scrollByPage",
            ListOptions {
                scroll_by_page: true,
                ..Default::default()
            },
            |o| {
                let mut w = flat_with(100, o);
                w.scrollbar_click(90.0); // below the thumb
                let paged = w.scroll_top() == 5.0 * H;
                let mut jump = flat_with(100, ListOptions::default());
                jump.scrollbar_click(90.0);
                paged && jump.scroll_top() > 5.0 * H
            },
        ),
        (
            "smoothScrolling",
            ListOptions {
                smooth_scrolling: true,
                ..Default::default()
            },
            |o| {
                let mut w = flat_with(100, o);
                w.scroll_by(100.0);
                let before = w.scroll_top();
                w.tick(0.0);
                w.tick(60.0);
                let mid = w.scroll_top();
                w.tick(200.0);
                before == 0.0
                    && mid > 0.0
                    && mid < 100.0
                    && w.scroll_top() == 100.0
                    && !w.is_animating()
            },
        ),
        (
            "typeNavigationMode",
            ListOptions {
                type_navigation_mode: TypeNavigationMode::Trigger,
                ..Default::default()
            },
            |o| !flat_with(10, o).type_char("i", 0.0),
        ),
        (
            "keyboardNavigation (deprecated)",
            ListOptions {
                find_mode: FindMode::Filter,
                ..Default::default()
            },
            // Read-side fallback is covered in options.rs; here: the resolved mode applies.
            |o| {
                let mut w = flat_with(10, o);
                run(&mut w, "list.find");
                w.find_state().mode == FindMode::Filter
            },
        ),
        (
            "tree.enableStickyScroll",
            ListOptions {
                sticky_scroll: false,
                ..Default::default()
            },
            |o| {
                let mut w = tree_with(PATHS, o);
                w.expand_id(&ItemId("a".into()), true).unwrap();
                w.set_viewport(H, 100.0);
                w.set_scroll_top(2.0 * H);
                w.sticky_rows().is_empty()
            },
        ),
        (
            "tree.stickyScrollMaxItemCount",
            ListOptions {
                sticky_scroll_max_items: 1,
                ..Default::default()
            },
            |o| {
                let mut w = tree_with(PATHS, o);
                w.expand_id(&ItemId("a".into()), true).unwrap();
                w.expand_id(&ItemId("a/a2".into()), true).unwrap();
                w.set_viewport(H, 100.0);
                w.set_scroll_top(3.0 * H); // first row x.cs: ancestors a, a2
                w.sticky_rows().len() == 1
            },
        ),
        (
            "tree.expandMode",
            ListOptions {
                expand_mode: OpenMode::DoubleClick,
                ..Default::default()
            },
            |o| {
                let mut w = tree_with(PATHS, o);
                w.click(0, Modifiers::default(), 1, false);
                let single = w.len() == 3;
                w.click(0, Modifiers::default(), 2, false);
                let double = w.len() == 6;
                w.click(0, Modifiers::default(), 1, true); // twistie always toggles
                single && double && w.len() == 3
            },
        ),
        (
            "tree.indent",
            ListOptions {
                indent: 20.0,
                ..Default::default()
            },
            |o| super::super::view::indent_px(&o, 2) == 40.0,
        ),
        (
            "tree.renderIndentGuides",
            ListOptions {
                indent_guides: IndentGuides::None,
                ..Default::default()
            },
            |o| {
                !super::super::view::guides_visible(&o, true)
                    && super::super::view::guides_visible(&ListOptions::default(), true)
                    && !super::super::view::guides_visible(&ListOptions::default(), false)
            },
        ),
    ];
    let mut failed = vec![];
    for (name, options, check) in cases {
        if !check(options) {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "settings without effect: {failed:?}");
}

#[test]
fn contract_controller_and_identity_preservation() {
    let mut w = flat(5);
    let c: &mut dyn ListController = &mut w;
    c.navigate(Navigation::Next).unwrap();
    c.navigate(Navigation::Last).unwrap();
    c.navigate(Navigation::Accept).unwrap();
    assert_eq!(c.selection()[0].0.as_ref(), "item4");
    c.filter("item4".into(), Cancellation::default()).unwrap();
    assert_eq!(w.len(), 1);
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert_eq!(w.filter("".into(), cancelled), Err(Error::Cancelled));
    w.filter("".into(), Cancellation::default()).unwrap();
    assert_eq!(
        w.expand(&ItemId("item0".into()), true),
        Err(Error::Unsupported)
    );
    // New snapshot with an inserted row: focus/selection follow the ItemId.
    let mut rows: Vec<Row> = (0..5)
        .map(|i| plain_row(&format!("item{i}"), "x"))
        .collect();
    rows.insert(0, plain_row("new", "x"));
    w.set_model(Arc::new(StaticList::new(rows, Version(1))));
    assert_eq!((w.focus(), w.selection_indices()), (Some(5), &[5][..]));

    let mut t = tree();
    assert_eq!(
        t.expand(&ItemId("missing".into()), true),
        Err(Error::NotFound)
    );
    t.expand(&ItemId("b".into()), true).unwrap();
    assert_eq!(shown(&t), ["a", "b", "  b1.cs", "c.cs"]);
    t.navigate(Navigation::Last).unwrap();
    t.navigate(Navigation::Previous).unwrap();
    t.navigate(Navigation::Parent).unwrap();
    assert_eq!(focused_label(&t).as_deref(), Some("b"));
}

#[test]
fn per_row_heights_separators_and_loading_tail() {
    let model = StaticList::from_labels(["h", "a", "b", "h2", "c"]);
    let mut w = ListWidget::new(
        Arc::new(model),
        RowHeight::PerRow(Arc::new(
            |r: &Row| {
                if r.label.starts_with('h') {
                    30.0
                } else {
                    20.0
                }
            },
        )),
        ListOptions::default(),
    );
    w.set_viewport(50.0, 100.0);
    assert_eq!(w.total_height(), 120.0);
    assert_eq!((w.row_top(3), w.row_height(3)), (70.0, 30.0));
    assert_eq!(w.visible_range(), 0..3);
    w.set_scroll_top(60.0);
    assert_eq!(w.visible_range(), 2..5);
    w.set_loading_tail(true);
    assert_eq!(
        w.total_height(),
        150.0,
        "tail row adds the first row height"
    );
    w.set_separators(vec![(0, None), (3, Some("组".into()))]);
    assert_eq!(w.group_starts()[1], (3, Some(Arc::from("组"))));
    w.set_filter_query(Some(("c".into(), FindMode::Filter)));
    assert_eq!(
        w.group_starts(),
        [(0, Some(Arc::from("组")))],
        "empty groups disappear"
    );
}

#[test]
fn hundred_thousand_rows_only_touch_the_viewport() {
    let l = labels(100_000);
    let mut w = ListWidget::new(
        Arc::new(StaticList::from_labels(l.iter().map(|s| s.as_str()))),
        RowHeight::Fixed(22.0),
        ListOptions::default(),
    );
    w.set_viewport(800.0, 300.0);
    w.set_scroll_top(50_000.0 * 22.0);
    let r = w.visible_range();
    assert!(r.len() <= 38, "{r:?}");
    assert_eq!(w.display_rows(r.clone())[0].label.as_ref(), "item50000");
}
