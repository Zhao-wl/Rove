use super::{Axis, Direction, EditorService};
use gpui::{Action, KeyBinding};
use rove_base::Result;
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = editors, no_json)]
pub(crate) struct EditorAction(pub(crate) &'static str);

#[derive(Clone, Copy)]
enum GroupIndex {
    First,
    Previous,
    Next,
    Last,
}

#[derive(Clone, Copy)]
enum Operation {
    Split(Direction),
    Orthogonal,
    Secondary(bool),
    Transpose,
    Maximize,
    Lock(Option<bool>),
    Close,
    CloseGroup,
    CloseAllGroups,
    CloseTabs { all: bool, preserve_pinned: bool },
    Reopen,
    Keep,
    Pin(bool),
    Cycle(bool),
    LastTab,
    ShiftTab(bool),
    MoveTo(GroupIndex),
    History(bool),
    LastLocation,
    FocusDirection(Direction),
    MoveDirection(Direction),
    FocusIndex(usize),
    TabIndex(usize),
}
use Operation::*;

struct Spec {
    id: &'static str,
    title_key: &'static str,
    title: &'static str,
    operation: Operation,
    keys: &'static str,
    when: Option<&'static str>,
}

macro_rules! spec {
    ($id:literal, $title:literal, $operation:expr, $keys:literal, $when:expr) => {
        Spec {
            id: concat!("workbench.action.", $id),
            title_key: concat!("editors.command.", $id),
            title: $title,
            operation: $operation,
            keys: $keys,
            when: $when,
        }
    };
}

static COMMANDS: &[Spec] = &[
    spec!(
        "splitEditor",
        "向右分屏",
        Split(Direction::Right),
        "ctrl-\\",
        None
    ),
    spec!(
        "splitEditorDown",
        "向下分屏",
        Split(Direction::Down),
        "",
        None
    ),
    spec!("splitEditorUp", "向上分屏", Split(Direction::Up), "", None),
    spec!(
        "splitEditorLeft",
        "向左分屏",
        Split(Direction::Left),
        "",
        None
    ),
    spec!(
        "splitEditorRight",
        "向右分屏",
        Split(Direction::Right),
        "",
        None
    ),
    spec!(
        "splitEditorOrthogonal",
        "正交分屏",
        Orthogonal,
        "ctrl-k ctrl-\\",
        None
    ),
    spec!(
        "splitEditorInGroup",
        "在组内拆分编辑器",
        Secondary(true),
        "ctrl-k ctrl-shift-\\",
        Some("!sideBySideEditorActive")
    ),
    spec!(
        "joinEditorInGroup",
        "合并组内编辑器",
        Secondary(false),
        "ctrl-k ctrl-shift-\\",
        Some("sideBySideEditorActive")
    ),
    spec!(
        "toggleEditorGroupLayout",
        "切换分屏方向",
        Transpose,
        "shift-alt-0",
        None
    ),
    spec!(
        "toggleMaximizeEditorGroup",
        "最大化或还原编辑器组",
        Maximize,
        "ctrl-k ctrl-m",
        None
    ),
    spec!(
        "toggleEditorGroupLock",
        "锁定或解锁编辑器组",
        Lock(None),
        "",
        None
    ),
    spec!(
        "lockEditorGroup",
        "锁定编辑器组",
        Lock(Some(true)),
        "",
        None
    ),
    spec!(
        "unlockEditorGroup",
        "解锁编辑器组",
        Lock(Some(false)),
        "",
        None
    ),
    spec!(
        "closeActiveEditor",
        "关闭活动标签",
        Close,
        "ctrl-w|ctrl-f4",
        None
    ),
    spec!("closeGroup", "关闭编辑器组", CloseGroup, "", None),
    spec!(
        "closeAllGroups",
        "关闭所有编辑器组",
        CloseAllGroups,
        "ctrl-k ctrl-shift-w",
        None
    ),
    spec!(
        "closeEditorsInGroup",
        "关闭组内标签",
        CloseTabs {
            all: false,
            preserve_pinned: false
        },
        "ctrl-k w",
        None
    ),
    spec!(
        "closeAllEditors",
        "关闭所有标签",
        CloseTabs {
            all: true,
            preserve_pinned: true
        },
        "ctrl-k ctrl-w",
        None
    ),
    spec!(
        "closeUnmodifiedEditors",
        "关闭未修改标签",
        CloseTabs {
            all: false,
            preserve_pinned: true
        },
        "ctrl-k u",
        None
    ),
    spec!(
        "reopenClosedEditor",
        "重新打开已关闭的标签",
        Reopen,
        "ctrl-shift-t",
        None
    ),
    spec!("keepEditor", "保留预览标签", Keep, "ctrl-k enter", None),
    spec!(
        "pinEditor",
        "固定标签",
        Pin(true),
        "ctrl-k shift-enter",
        Some("!activeEditorIsPinned")
    ),
    spec!(
        "unpinEditor",
        "取消固定标签",
        Pin(false),
        "ctrl-k shift-enter",
        Some("activeEditorIsPinned")
    ),
    spec!(
        "nextEditor",
        "下一个标签",
        Cycle(true),
        "ctrl-pagedown",
        None
    ),
    spec!(
        "previousEditor",
        "上一个标签",
        Cycle(false),
        "ctrl-pageup",
        None
    ),
    spec!(
        "nextEditorInGroup",
        "组内下一个标签",
        Cycle(true),
        "ctrl-k ctrl-pagedown",
        None
    ),
    spec!(
        "previousEditorInGroup",
        "组内上一个标签",
        Cycle(false),
        "ctrl-k ctrl-pageup",
        None
    ),
    spec!(
        "lastEditorInGroup",
        "组内最后一个标签",
        LastTab,
        "alt-0|ctrl-9",
        None
    ),
    spec!(
        "moveEditorLeftInGroup",
        "将标签左移",
        ShiftTab(false),
        "ctrl-shift-pageup",
        None
    ),
    spec!(
        "moveEditorRightInGroup",
        "将标签右移",
        ShiftTab(true),
        "ctrl-shift-pagedown",
        None
    ),
    spec!(
        "moveEditorToNextGroup",
        "将标签移到下一组",
        MoveTo(GroupIndex::Next),
        "ctrl-alt-right",
        None
    ),
    spec!(
        "moveEditorToPreviousGroup",
        "将标签移到上一组",
        MoveTo(GroupIndex::Previous),
        "ctrl-alt-left",
        None
    ),
    spec!(
        "moveEditorToFirstGroup",
        "将标签移到第一组",
        MoveTo(GroupIndex::First),
        "shift-alt-1",
        None
    ),
    spec!(
        "moveEditorToLastGroup",
        "将标签移到最后一组",
        MoveTo(GroupIndex::Last),
        "shift-alt-9",
        None
    ),
    spec!(
        "navigateBack",
        "后退",
        History(false),
        "alt-left",
        Some("canNavigateBack")
    ),
    spec!(
        "navigateForward",
        "前进",
        History(true),
        "alt-right",
        Some("canNavigateForward")
    ),
    spec!(
        "navigateToLastEditLocation",
        "跳到最后打开的位置",
        LastLocation,
        "ctrl-k ctrl-q",
        None
    ),
    spec!(
        "focusAboveGroup",
        "聚焦上方编辑器组",
        FocusDirection(Direction::Up),
        "ctrl-k ctrl-up",
        None
    ),
    spec!(
        "moveActiveEditorGroupUp",
        "将编辑器组向上移动",
        MoveDirection(Direction::Up),
        "ctrl-k up",
        None
    ),
    spec!(
        "focusBelowGroup",
        "聚焦下方编辑器组",
        FocusDirection(Direction::Down),
        "ctrl-k ctrl-down",
        None
    ),
    spec!(
        "moveActiveEditorGroupDown",
        "将编辑器组向下移动",
        MoveDirection(Direction::Down),
        "ctrl-k down",
        None
    ),
    spec!(
        "focusLeftGroup",
        "聚焦左方编辑器组",
        FocusDirection(Direction::Left),
        "ctrl-k ctrl-left",
        None
    ),
    spec!(
        "moveActiveEditorGroupLeft",
        "将编辑器组向左移动",
        MoveDirection(Direction::Left),
        "ctrl-k left",
        None
    ),
    spec!(
        "focusRightGroup",
        "聚焦右方编辑器组",
        FocusDirection(Direction::Right),
        "ctrl-k ctrl-right",
        None
    ),
    spec!(
        "moveActiveEditorGroupRight",
        "将编辑器组向右移动",
        MoveDirection(Direction::Right),
        "ctrl-k right",
        None
    ),
    spec!(
        "focusFirstEditorGroup",
        "聚焦第 1 组",
        FocusIndex(0),
        "ctrl-1",
        None
    ),
    spec!(
        "focusSecondEditorGroup",
        "聚焦第 2 组",
        FocusIndex(1),
        "ctrl-2",
        None
    ),
    spec!(
        "focusThirdEditorGroup",
        "聚焦第 3 组",
        FocusIndex(2),
        "ctrl-3",
        None
    ),
    spec!(
        "focusFourthEditorGroup",
        "聚焦第 4 组",
        FocusIndex(3),
        "ctrl-4",
        None
    ),
    spec!(
        "focusFifthEditorGroup",
        "聚焦第 5 组",
        FocusIndex(4),
        "ctrl-5",
        None
    ),
    spec!(
        "focusSixthEditorGroup",
        "聚焦第 6 组",
        FocusIndex(5),
        "ctrl-6",
        None
    ),
    spec!(
        "focusSeventhEditorGroup",
        "聚焦第 7 组",
        FocusIndex(6),
        "ctrl-7",
        None
    ),
    spec!(
        "focusEighthEditorGroup",
        "聚焦第 8 组",
        FocusIndex(7),
        "ctrl-8",
        None
    ),
    spec!(
        "openEditorAtIndex1",
        "打开第 1 个标签",
        TabIndex(0),
        "alt-1",
        None
    ),
    spec!(
        "openEditorAtIndex2",
        "打开第 2 个标签",
        TabIndex(1),
        "alt-2",
        None
    ),
    spec!(
        "openEditorAtIndex3",
        "打开第 3 个标签",
        TabIndex(2),
        "alt-3",
        None
    ),
    spec!(
        "openEditorAtIndex4",
        "打开第 4 个标签",
        TabIndex(3),
        "alt-4",
        None
    ),
    spec!(
        "openEditorAtIndex5",
        "打开第 5 个标签",
        TabIndex(4),
        "alt-5",
        None
    ),
    spec!(
        "openEditorAtIndex6",
        "打开第 6 个标签",
        TabIndex(5),
        "alt-6",
        None
    ),
    spec!(
        "openEditorAtIndex7",
        "打开第 7 个标签",
        TabIndex(6),
        "alt-7",
        None
    ),
    spec!(
        "openEditorAtIndex8",
        "打开第 8 个标签",
        TabIndex(7),
        "alt-8",
        None
    ),
    spec!(
        "openEditorAtIndex9",
        "打开第 9 个标签",
        TabIndex(8),
        "alt-9",
        None
    ),
];

pub(crate) fn register(registry: &mut CommandRegistry) -> Result<()> {
    for spec in COMMANDS {
        registry.register(
            spec.id,
            Command {
                title_key: spec.title_key.into(),
                category_key: None,
            },
        )?;
    }
    Ok(())
}

pub(crate) fn strings(strings: &mut StringTable) -> Result<()> {
    for spec in COMMANDS {
        strings.register(spec.title_key, spec.title.into())?;
    }
    Ok(())
}

pub(crate) fn bindings() -> Vec<KeyBinding> {
    COMMANDS
        .iter()
        .flat_map(|spec| {
            spec.keys
                .split('|')
                .filter(|key| !key.is_empty())
                .map(|key| KeyBinding::new(key, EditorAction(spec.id), spec.when))
        })
        .collect()
}

pub(crate) fn execute(editor: &mut EditorService, id: &str) -> bool {
    let Some(spec) = COMMANDS.iter().find(|spec| spec.id == id) else {
        return false;
    };
    let group = editor.focused;
    let tab = editor.active().active;
    match spec.operation {
        Split(direction) => {
            let _ = editor.split(direction);
        }
        Orthogonal => {
            let direction = if editor.layout.parent_axis(group) == Some(Axis::Vertical) {
                Direction::Right
            } else {
                Direction::Down
            };
            let _ = editor.split(direction);
        }
        Secondary(split) => {
            if let Some(tab) = editor.groups.get_mut(&group).unwrap().current_mut() {
                tab.secondary = split.then(|| tab.view.clone());
            }
        }
        Transpose => editor.layout.transpose(),
        Maximize => editor.toggle_maximize(group),
        Lock(value) => {
            let pane = editor.groups.get_mut(&group).unwrap();
            pane.locked = value.unwrap_or(!pane.locked);
        }
        Close => {
            if let Some(tab) = tab {
                editor.close_tab(group, tab);
            } else {
                editor.close_group(group);
            }
        }
        CloseGroup => {
            editor.close_group(group);
        }
        CloseAllGroups => {
            for group in editor.layout.order() {
                editor.close_group(group);
            }
        }
        CloseTabs {
            all,
            preserve_pinned,
        } => editor.close_tabs((!all).then_some(group), preserve_pinned),
        Reopen => editor.reopen(),
        Keep => {
            if let Some(tab) = tab {
                editor.keep(group, tab);
            }
        }
        Pin(pinned) => {
            if let Some(tab) = tab {
                editor.pin(group, tab, pinned);
            }
        }
        Cycle(forward) => editor.cycle_tab(forward),
        LastTab => editor.activate_index(editor.active().tabs.len().saturating_sub(1)),
        ShiftTab(right) => editor.shift_tab(right),
        MoveTo(index) => {
            let groups = editor.layout.order();
            let current = groups.iter().position(|id| *id == group).unwrap();
            let target = match index {
                GroupIndex::First => 0,
                GroupIndex::Previous => current.saturating_sub(1),
                GroupIndex::Next => (current + 1).min(groups.len() - 1),
                GroupIndex::Last => groups.len() - 1,
            };
            if let Some(tab) = tab.filter(|_| target != current) {
                editor.drop_tab(group, tab, groups[target], None, None);
            }
        }
        History(forward) => editor.navigate(forward),
        LastLocation => editor.navigate_last(),
        FocusDirection(direction) => {
            if let Some(target) = editor.layout.neighbor(group, direction) {
                editor.focus(target);
            }
        }
        MoveDirection(direction) => {
            if let Some(target) = editor.layout.neighbor(group, direction) {
                editor.layout.swap(group, target);
            }
        }
        FocusIndex(index) => {
            if let Some(group) = editor.layout.order().get(index) {
                editor.focus(*group);
            }
        }
        TabIndex(index) => editor.activate_index(index),
    }
    true
}

pub(crate) fn context(editor: &EditorService) -> String {
    let mut keys = String::from("EditorGroups");
    if editor.can_back() {
        keys.push_str(" canNavigateBack");
    }
    if editor.can_forward() {
        keys.push_str(" canNavigateForward");
    }
    if let Some(tab) = editor.active().current() {
        if tab.pinned {
            keys.push_str(" activeEditorIsPinned");
        }
        if tab.secondary.is_some() {
            keys.push_str(" sideBySideEditorActive");
        }
    }
    keys
}
