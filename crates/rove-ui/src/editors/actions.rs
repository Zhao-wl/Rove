use super::{Axis, Direction, EditorService};
use gpui::Action;
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
}

macro_rules! spec {
    ($id:literal, $title:literal, $operation:expr) => {
        Spec {
            id: concat!("workbench.action.", $id),
            title_key: concat!("editors.command.", $id),
            title: $title,
            operation: $operation,
        }
    };
}

static COMMANDS: &[Spec] = &[
    spec!("splitEditor", "向右分屏", Split(Direction::Right)),
    spec!("splitEditorDown", "向下分屏", Split(Direction::Down)),
    spec!("splitEditorUp", "向上分屏", Split(Direction::Up)),
    spec!("splitEditorLeft", "向左分屏", Split(Direction::Left)),
    spec!("splitEditorRight", "向右分屏", Split(Direction::Right)),
    spec!("splitEditorOrthogonal", "正交分屏", Orthogonal),
    spec!("splitEditorInGroup", "在组内拆分编辑器", Secondary(true)),
    spec!("joinEditorInGroup", "合并组内编辑器", Secondary(false)),
    spec!("toggleEditorGroupLayout", "切换分屏方向", Transpose),
    spec!(
        "toggleMaximizeEditorGroup",
        "最大化或还原编辑器组",
        Maximize
    ),
    spec!("toggleEditorGroupLock", "锁定或解锁编辑器组", Lock(None)),
    spec!("lockEditorGroup", "锁定编辑器组", Lock(Some(true))),
    spec!("unlockEditorGroup", "解锁编辑器组", Lock(Some(false))),
    spec!("closeActiveEditor", "关闭活动标签", Close),
    spec!("closeGroup", "关闭编辑器组", CloseGroup),
    spec!("closeAllGroups", "关闭所有编辑器组", CloseAllGroups),
    spec!(
        "closeEditorsInGroup",
        "关闭组内标签",
        CloseTabs {
            all: false,
            preserve_pinned: false
        }
    ),
    spec!(
        "closeAllEditors",
        "关闭所有标签",
        CloseTabs {
            all: true,
            preserve_pinned: true
        }
    ),
    spec!(
        "closeUnmodifiedEditors",
        "关闭未修改标签",
        CloseTabs {
            all: false,
            preserve_pinned: true
        }
    ),
    spec!("reopenClosedEditor", "重新打开已关闭的标签", Reopen),
    spec!("keepEditor", "保留预览标签", Keep),
    spec!("pinEditor", "固定标签", Pin(true)),
    spec!("unpinEditor", "取消固定标签", Pin(false)),
    spec!("nextEditor", "下一个标签", Cycle(true)),
    spec!("previousEditor", "上一个标签", Cycle(false)),
    spec!("nextEditorInGroup", "组内下一个标签", Cycle(true)),
    spec!("previousEditorInGroup", "组内上一个标签", Cycle(false)),
    spec!("lastEditorInGroup", "组内最后一个标签", LastTab),
    spec!("moveEditorLeftInGroup", "将标签左移", ShiftTab(false)),
    spec!("moveEditorRightInGroup", "将标签右移", ShiftTab(true)),
    spec!(
        "moveEditorToNextGroup",
        "将标签移到下一组",
        MoveTo(GroupIndex::Next)
    ),
    spec!(
        "moveEditorToPreviousGroup",
        "将标签移到上一组",
        MoveTo(GroupIndex::Previous)
    ),
    spec!(
        "moveEditorToFirstGroup",
        "将标签移到第一组",
        MoveTo(GroupIndex::First)
    ),
    spec!(
        "moveEditorToLastGroup",
        "将标签移到最后一组",
        MoveTo(GroupIndex::Last)
    ),
    spec!("navigateBack", "后退", History(false)),
    spec!("navigateForward", "前进", History(true)),
    spec!(
        "navigateToLastEditLocation",
        "跳到最后打开的位置",
        LastLocation
    ),
    spec!(
        "focusAboveGroup",
        "聚焦上方编辑器组",
        FocusDirection(Direction::Up)
    ),
    spec!(
        "moveActiveEditorGroupUp",
        "将编辑器组向上移动",
        MoveDirection(Direction::Up)
    ),
    spec!(
        "focusBelowGroup",
        "聚焦下方编辑器组",
        FocusDirection(Direction::Down)
    ),
    spec!(
        "moveActiveEditorGroupDown",
        "将编辑器组向下移动",
        MoveDirection(Direction::Down)
    ),
    spec!(
        "focusLeftGroup",
        "聚焦左方编辑器组",
        FocusDirection(Direction::Left)
    ),
    spec!(
        "moveActiveEditorGroupLeft",
        "将编辑器组向左移动",
        MoveDirection(Direction::Left)
    ),
    spec!(
        "focusRightGroup",
        "聚焦右方编辑器组",
        FocusDirection(Direction::Right)
    ),
    spec!(
        "moveActiveEditorGroupRight",
        "将编辑器组向右移动",
        MoveDirection(Direction::Right)
    ),
    spec!("focusFirstEditorGroup", "聚焦第 1 组", FocusIndex(0)),
    spec!("focusSecondEditorGroup", "聚焦第 2 组", FocusIndex(1)),
    spec!("focusThirdEditorGroup", "聚焦第 3 组", FocusIndex(2)),
    spec!("focusFourthEditorGroup", "聚焦第 4 组", FocusIndex(3)),
    spec!("focusFifthEditorGroup", "聚焦第 5 组", FocusIndex(4)),
    spec!("focusSixthEditorGroup", "聚焦第 6 组", FocusIndex(5)),
    spec!("focusSeventhEditorGroup", "聚焦第 7 组", FocusIndex(6)),
    spec!("focusEighthEditorGroup", "聚焦第 8 组", FocusIndex(7)),
    spec!("openEditorAtIndex1", "打开第 1 个标签", TabIndex(0)),
    spec!("openEditorAtIndex2", "打开第 2 个标签", TabIndex(1)),
    spec!("openEditorAtIndex3", "打开第 3 个标签", TabIndex(2)),
    spec!("openEditorAtIndex4", "打开第 4 个标签", TabIndex(3)),
    spec!("openEditorAtIndex5", "打开第 5 个标签", TabIndex(4)),
    spec!("openEditorAtIndex6", "打开第 6 个标签", TabIndex(5)),
    spec!("openEditorAtIndex7", "打开第 7 个标签", TabIndex(6)),
    spec!("openEditorAtIndex8", "打开第 8 个标签", TabIndex(7)),
    spec!("openEditorAtIndex9", "打开第 9 个标签", TabIndex(8)),
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

/// 命令 ID → GPUI action（键位服务 `crate::keys` 解析按键后分发；默认键位来自 VS Code 默认表）。
pub(crate) fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|spec| spec.id == id)
        .map(|spec| Box::new(EditorAction(spec.id)) as Box<dyn Action>)
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

/// 编辑器区域的 key context：发布 VS Code 默认键位 when 子句用到的上下文键。
pub(crate) fn context(editor: &EditorService) -> String {
    let mut keys = String::from("EditorGroups");
    let mut add = |on: bool, key: &str| {
        if on {
            keys.push(' ');
            keys.push_str(key);
        }
    };
    add(editor.can_back(), "canNavigateBack");
    add(editor.can_forward(), "canNavigateForward");
    let multiple = editor.groups.len() > 1;
    add(multiple, "multipleEditorGroups");
    add(multiple, "editorPartMultipleEditorGroups");
    add(editor.maximized.is_some(), "editorPartMaximizedEditorGroup");
    let current = editor.active().current();
    add(current.is_none(), "activeEditorGroupEmpty");
    if let Some(tab) = current {
        add(tab.pinned, "activeEditorIsPinned");
        add(tab.secondary.is_some(), "sideBySideEditorActive");
        add(tab.secondary.is_none(), "activeEditorCanSplitInGroup");
    }
    keys
}
