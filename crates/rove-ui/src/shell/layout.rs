//! 区域级布局状态与按工作区持久化（#16，issue §4.1）。
//!
//! 保存：侧边栏宽度与可见性、面板大小与可见性、当前活动视图容器与面板视图、折叠的小节、
//! 窗口位置与大小、最大化状态。标签与编辑器组属于 #17，不在这里。
//!
//! 文件位置：`%APPDATA%\Rove\workspaceStorage\<工作区哈希>\layout.json`（与 VS Code 的
//! workspaceStorage 思路相同，但不写 VS Code 的目录）；全局状态（例如提示条的“不再提示”）在
//! `%APPDATA%\Rove\globalState.json`。文件只有几百字节：启动时在首帧前同步读取一次，
//! 之后的写入都在后台执行器里完成，UI 线程不做文件 I/O。
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub const DEFAULT_SIDEBAR_WIDTH: f32 = 300.0;
pub const MIN_SIDEBAR_WIDTH: f32 = 170.0;
pub const DEFAULT_PANEL_SIZE: f32 = 300.0;
pub const MIN_PANEL_SIZE: f32 = 77.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideBarLocation {
    Left,
    Right,
}
impl SideBarLocation {
    /// `workbench.sideBar.location` 的取值；未知值按 VS Code 回退到默认值 left。
    pub fn from_setting(value: &str) -> Self {
        match value {
            "right" => Self::Right,
            _ => Self::Left,
        }
    }
    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelLocation {
    Bottom,
    Top,
    Left,
    Right,
}
impl PanelLocation {
    /// `workbench.panel.defaultLocation` 的取值；未知值回退到 bottom。
    pub fn from_setting(value: &str) -> Self {
        match value {
            "top" => Self::Top,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => Self::Bottom,
        }
    }
    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Top => "top",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
    pub fn is_horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// 窗口位置与大小（逻辑像素，屏幕坐标）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutState {
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    pub panel_visible: bool,
    /// 底部 / 顶部时是高度，左右时是宽度。
    pub panel_size: f32,
    /// None 表示新工作区：由 `workbench.panel.defaultLocation` 决定。
    pub panel_location: Option<PanelLocation>,
    pub active_container: Option<String>,
    pub active_panel_view: Option<String>,
    pub collapsed_sections: BTreeSet<String>,
    pub window: Option<WindowPlacement>,
}
impl Default for LayoutState {
    /// 新工作区：侧边栏显示资源管理器，面板默认隐藏（UX_SPEC §1.4）。
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            panel_visible: false,
            panel_size: DEFAULT_PANEL_SIZE,
            panel_location: None,
            active_container: None,
            active_panel_view: None,
            collapsed_sections: BTreeSet::new(),
            window: None,
        }
    }
}

impl LayoutState {
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "version": 1,
            "sideBar": { "visible": self.sidebar_visible, "width": self.sidebar_width },
            "panel": { "visible": self.panel_visible, "size": self.panel_size },
            "collapsedSections": self.collapsed_sections.iter().collect::<Vec<_>>(),
        });
        let o = v.as_object_mut().unwrap();
        if let Some(l) = self.panel_location {
            o["panel"]["location"] = json!(l.as_setting());
        }
        if let Some(c) = &self.active_container {
            o.insert("activeViewContainer".into(), json!(c));
        }
        if let Some(p) = &self.active_panel_view {
            o["panel"]["activeView"] = json!(p);
        }
        if let Some(w) = self.window {
            o.insert(
                "window".into(),
                json!({ "x": w.x, "y": w.y, "width": w.width, "height": w.height, "maximized": w.maximized }),
            );
        }
        v
    }

    /// 宽容读取：缺字段或类型不对时用默认值，不让损坏的状态文件阻止启动。
    pub fn from_json(v: &Value) -> Self {
        let d = Self::default();
        let num = |v: &Value| v.as_f64().filter(|x| x.is_finite()).map(|x| x as f32);
        let side = &v["sideBar"];
        let panel = &v["panel"];
        let window = &v["window"];
        Self {
            sidebar_visible: side["visible"].as_bool().unwrap_or(d.sidebar_visible),
            sidebar_width: num(&side["width"])
                .unwrap_or(d.sidebar_width)
                .max(MIN_SIDEBAR_WIDTH),
            panel_visible: panel["visible"].as_bool().unwrap_or(d.panel_visible),
            panel_size: num(&panel["size"])
                .unwrap_or(d.panel_size)
                .max(MIN_PANEL_SIZE),
            panel_location: panel["location"].as_str().map(PanelLocation::from_setting),
            active_container: v["activeViewContainer"].as_str().map(str::to_owned),
            active_panel_view: panel["activeView"].as_str().map(str::to_owned),
            collapsed_sections: v["collapsedSections"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            window: match (
                num(&window["x"]),
                num(&window["y"]),
                num(&window["width"]),
                num(&window["height"]),
            ) {
                (Some(x), Some(y), Some(width), Some(height)) if width > 0.0 && height > 0.0 => {
                    Some(WindowPlacement {
                        x,
                        y,
                        width,
                        height,
                        maximized: window["maximized"].as_bool().unwrap_or(false),
                    })
                }
                _ => None,
            },
        }
    }
}

/// 用户数据目录下的状态存储。`None` 根目录表示不持久化（单元测试用）。
#[derive(Clone, Debug)]
pub struct LayoutStore {
    base: Option<PathBuf>,
    workspace_key: String,
}

impl LayoutStore {
    /// `%APPDATA%\Rove`；取不到时不持久化（不回退到当前目录，避免把状态写进项目）。
    pub fn for_workspace(root: &Path) -> Self {
        let base = std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Rove"));
        Self::with_base(base, root)
    }
    pub fn with_base(base: Option<PathBuf>, root: &Path) -> Self {
        Self {
            base,
            workspace_key: workspace_key(root),
        }
    }
    pub fn disabled() -> Self {
        Self {
            base: None,
            workspace_key: String::new(),
        }
    }
    pub fn layout_path(&self) -> Option<PathBuf> {
        self.base.as_ref().map(|b| {
            b.join("workspaceStorage")
                .join(&self.workspace_key)
                .join("layout.json")
        })
    }
    pub fn global_path(&self) -> Option<PathBuf> {
        self.base.as_ref().map(|b| b.join("globalState.json"))
    }
    /// 读取上次的布局；文件不存在或损坏时返回默认布局。
    pub fn load(&self) -> LayoutState {
        self.layout_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map(|v| LayoutState::from_json(&v))
            .unwrap_or_default()
    }
    pub fn load_global(&self) -> Map<String, Value> {
        self.global_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }
}

/// 后台写入：先写临时文件再替换，避免写到一半崩溃留下半个文件。
pub fn write_json(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)
}

/// 工作区路径的稳定哈希（Windows 路径大小写不敏感，统一分隔符后取 FNV-1a 64）。
fn workspace_key(root: &Path) -> String {
    let normalized = root
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase();
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in normalized.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_field() {
        let state = LayoutState {
            sidebar_visible: false,
            sidebar_width: 412.0,
            panel_visible: true,
            panel_size: 180.0,
            panel_location: Some(PanelLocation::Right),
            active_container: Some("workbench.view.search".into()),
            active_panel_view: Some("workbench.panel.markers".into()),
            collapsed_sections: ["workbench.explorer.fileView".to_owned()].into(),
            window: Some(WindowPlacement {
                x: 10.0,
                y: 20.0,
                width: 900.0,
                height: 700.0,
                maximized: true,
            }),
        };
        assert_eq!(LayoutState::from_json(&state.to_json()), state);
    }

    #[test]
    fn corrupt_or_missing_state_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = LayoutStore::with_base(Some(dir.path().into()), Path::new("D:/W"));
        assert_eq!(store.load(), LayoutState::default());
        let path = store.layout_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(store.load(), LayoutState::default());
        write_json(&path, &json!({ "sideBar": { "width": 5 } })).unwrap();
        assert_eq!(store.load().sidebar_width, MIN_SIDEBAR_WIDTH);
    }

    #[test]
    fn workspace_key_ignores_case_and_separators() {
        assert_eq!(
            workspace_key(Path::new("D:\\Proj\\Game\\")),
            workspace_key(Path::new("d:/proj/game"))
        );
        assert_ne!(
            workspace_key(Path::new("d:/a")),
            workspace_key(Path::new("d:/b"))
        );
    }
}
