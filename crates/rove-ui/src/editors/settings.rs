use crate::documents::{OpenOptions, OpenSource, Presentation};
use globset::{GlobBuilder, GlobMatcher};
use rove_base::{Error, Registry, Resource, Result, Value};
use rove_config::{DefaultSettings, Key, Scope, Setting, SettingRegistry, Settings};
use std::{collections::BTreeMap, path::Path, sync::Arc};

pub(crate) const SOURCE: &str = "default";
pub(crate) const MARKDOWN: &str = "vscode.markdown.preview.editor";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EditorKind {
    Text,
    Placeholder { id: Arc<str>, label: &'static str },
}

pub(crate) struct EditorTypes {
    types: Registry<EditorKind>,
    rules: Vec<(GlobMatcher, bool, Arc<str>)>,
    markdown: GlobMatcher,
    /// #29：本机没有 WebView2 运行时（或创建失败）时为假：关联到预览的文件改为以源码打开，
    /// 显式要求预览返回 `Unsupported`（UX_SPEC §6.4）。
    preview_available: bool,
}

fn compile(pattern: &str) -> Option<(GlobMatcher, bool)> {
    let basename = !pattern.contains('/');
    let matcher = GlobBuilder::new(pattern)
        .literal_separator(true)
        .case_insensitive(cfg!(windows))
        .build()
        .ok()?
        .compile_matcher();
    Some((matcher, basename))
}

fn resource_path(resource: &Resource) -> &Path {
    match resource {
        Resource::Workspace(path) => Path::new(path.as_str()),
        Resource::File(path) => path,
        Resource::Virtual(name) => Path::new(name.as_ref()),
    }
}

fn matches(glob: &GlobMatcher, basename: bool, path: &Path) -> bool {
    if basename {
        path.file_name()
            .is_some_and(|name| glob.is_match(Path::new(name)))
    } else {
        glob.is_match(path)
    }
}

impl EditorTypes {
    fn new(associations: Value) -> Result<Self> {
        let mut types = Registry::default();
        types.register(SOURCE, EditorKind::Text)?;
        types.register(
            MARKDOWN,
            EditorKind::Placeholder {
                id: MARKDOWN.into(),
                label: "editors.markdown",
            },
        )?;
        let mut rules = Vec::new();
        if let Value::Object(patterns) = associations {
            for (pattern, id) in patterns.iter().rev() {
                if let (Some((glob, basename)), Value::String(id)) = (compile(pattern), id) {
                    rules.push((glob, basename, id.clone()));
                }
            }
        }
        Ok(Self {
            types,
            rules,
            markdown: compile("*.md").expect("valid built-in glob").0,
            preview_available: true,
        })
    }

    pub(crate) fn register_placeholder(&mut self, id: &str, label: &'static str) -> Result<()> {
        self.types.register(
            id,
            EditorKind::Placeholder {
                id: id.into(),
                label,
            },
        )
    }

    /// #29：Markdown 预览是否可用（见 `preview_available`）。
    pub(crate) fn set_preview_available(&mut self, available: bool) {
        self.preview_available = available;
    }

    pub(crate) fn preview_available(&self) -> bool {
        self.preview_available
    }

    pub(crate) fn resolve(&self, resource: &Resource, options: &OpenOptions) -> Result<EditorKind> {
        let kind = self.resolve_any(resource, options)?;
        if !self.preview_available && is_markdown_preview(&kind) {
            return match options.presentation {
                Presentation::Preview => Err(Error::Unsupported),
                _ => Ok(EditorKind::Text),
            };
        }
        Ok(kind)
    }

    fn resolve_any(&self, resource: &Resource, options: &OpenOptions) -> Result<EditorKind> {
        if options.location.is_some()
            || options.selection.is_some()
            || matches!(options.presentation, Presentation::Source)
            || matches!(
                options.source,
                OpenSource::Search
                    | OpenSource::CodeNavigation
                    | OpenSource::GoToLine
                    | OpenSource::Outline
                    | OpenSource::Breadcrumb
            )
        {
            return Ok(EditorKind::Text);
        }
        let path = resource_path(resource);
        let mut associated = self
            .rules
            .iter()
            .filter(|(glob, basename, _)| matches(glob, *basename, path))
            .filter_map(|(_, _, id)| self.types.get(id));
        if matches!(options.presentation, Presentation::Preview) {
            associated
                .find(|kind| matches!(kind, EditorKind::Placeholder { .. }))
                .or_else(|| {
                    if matches(&self.markdown, true, path) {
                        self.types.get(MARKDOWN)
                    } else {
                        None
                    }
                })
                .cloned()
                .ok_or(Error::Unsupported)
        } else {
            Ok(associated.next().cloned().unwrap_or(EditorKind::Text))
        }
    }
}

/// #29：是不是 Markdown 预览编辑器。
pub(crate) fn is_markdown_preview(kind: &EditorKind) -> bool {
    matches!(kind, EditorKind::Placeholder { id, .. } if id.as_ref() == MARKDOWN)
}

/// #29：Markdown 预览的编辑器类型。
pub(crate) fn markdown_preview() -> EditorKind {
    EditorKind::Placeholder {
        id: MARKDOWN.into(),
        label: "editors.markdown",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tabs {
    None,
    Single,
    Multiple,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sizing {
    Fit,
    Shrink,
    Fixed,
}

#[derive(Clone, Copy)]
pub(crate) struct SizePolicy {
    pub(crate) huge_bytes: u64,
    pub(crate) optimize: bool,
}

impl Default for SizePolicy {
    fn default() -> Self {
        Self {
            huge_bytes: 64 * 1024 * 1024,
            optimize: true,
        }
    }
}

pub(crate) struct EditorSettings {
    pub(crate) editors: EditorTypes,
    pub(crate) close_empty: bool,
    pub(crate) close_on_delete: bool,
    pub(crate) pinned_row: bool,
    pub(crate) restore_view: bool,
    pub(crate) reveal_if_open: bool,
    pub(crate) tab_limit: Option<usize>,
    pub(crate) limit_per_group: bool,
    pub(crate) limit_exclude_dirty: bool,
    pub(crate) tabs: Tabs,
    pub(crate) sizing: Sizing,
    pub(crate) min_width: f32,
    pub(crate) max_width: f32,
    pub(crate) wrap: bool,
    pub(crate) actions_left: bool,
    pub(crate) show_index: bool,
    pub(crate) tab_height: f32,
    pub(crate) double_click: String,
    pub(crate) secondary_vertical: bool,
    pub(crate) highlight_modified: bool,
    pub(crate) badges: bool,
    pub(crate) colors: bool,
    pub(crate) size: SizePolicy,
    pub(crate) explorer_preserve_focus: bool,
    preview: bool,
    preview_quick: bool,
    preview_code: bool,
    position: String,
    custom_labels: Vec<(GlobMatcher, bool, Arc<str>)>,
}

macro_rules! text {
    ($value:literal) => {
        Value::String(Arc::from($value))
    };
}

pub fn register() -> Result<SettingRegistry> {
    let mut registry = SettingRegistry::default();
    for (key, default) in [
        ("workbench.editor.closeEmptyGroups", Value::Bool(true)),
        ("workbench.editor.closeOnFileDelete", Value::Bool(false)),
        ("workbench.editor.enablePreview", Value::Bool(true)),
        (
            "workbench.editor.enablePreviewFromQuickOpen",
            Value::Bool(false),
        ),
        (
            "workbench.editor.enablePreviewFromCodeNavigation",
            Value::Bool(false),
        ),
        (
            "workbench.editor.pinnedTabsOnSeparateRow",
            Value::Bool(false),
        ),
        ("workbench.editor.restoreViewState", Value::Bool(true)),
        ("workbench.editor.revealIfOpen", Value::Bool(false)),
        ("workbench.editor.openPositioning", text!("right")),
        ("workbench.editor.limit.enabled", Value::Bool(false)),
        ("workbench.editor.limit.value", Value::Number(10.0)),
        ("workbench.editor.limit.perEditorGroup", Value::Bool(false)),
        ("workbench.editor.limit.excludeDirty", Value::Bool(false)),
        ("workbench.editor.showTabs", text!("multiple")),
        ("workbench.editor.tabSizing", text!("fit")),
        (
            "workbench.editor.tabSizingFixedMinWidth",
            Value::Number(50.0),
        ),
        (
            "workbench.editor.tabSizingFixedMaxWidth",
            Value::Number(160.0),
        ),
        ("workbench.editor.wrapTabs", Value::Bool(false)),
        ("workbench.editor.tabActionLocation", text!("right")),
        ("workbench.editor.showTabIndex", Value::Bool(false)),
        ("workbench.editor.customLabels.enabled", Value::Bool(true)),
        (
            "workbench.editor.customLabels.patterns",
            Value::Object(Arc::new(BTreeMap::new())),
        ),
        ("workbench.editor.decorations.badges", Value::Bool(true)),
        ("workbench.editor.decorations.colors", Value::Bool(true)),
        (
            "workbench.editor.doubleClickTabToToggleEditorGroupSizes",
            text!("expand"),
        ),
        ("workbench.editor.splitInGroupLayout", text!("horizontal")),
        ("workbench.editor.highlightModifiedTabs", Value::Bool(false)),
        ("window.density.editorTabHeight", text!("default")),
        (
            "editor.largeFileOptimizations",
            Value::Bool(SizePolicy::default().optimize),
        ),
        (
            "rove.largeFile.threshold",
            Value::Number(SizePolicy::default().huge_bytes as f64),
        ),
    ] {
        registry.register(
            key,
            Setting {
                default,
                rove_default: None,
                language_overridable: false,
            },
        )?;
    }
    registry.register(
        "workbench.editorAssociations",
        Setting {
            default: Value::Object(Arc::new(BTreeMap::new())),
            rove_default: Some(Value::Object(Arc::new(
                [("*.md".to_string(), Value::String(MARKDOWN.into()))].into(),
            ))),
            language_overridable: false,
        },
    )?;
    Ok(registry)
}

impl EditorSettings {
    pub(crate) fn read(settings: &dyn Settings) -> Result<Self> {
        let scope = Scope::default();
        let flag = |key| Key::<bool>::new(key).read(settings, &scope);
        let number = |key| Key::<f64>::new(key).read(settings, &scope);
        let value = |key| Key::<String>::new(key).read(settings, &scope);
        let object = |key| Key::<Value>::new(key).read(settings, &scope);
        let labels = object("workbench.editor.customLabels.patterns")?;
        let custom_labels = match labels {
            Value::Object(labels) if flag("workbench.editor.customLabels.enabled")? => labels
                .iter()
                .filter_map(|(pattern, label)| {
                    let (glob, basename) = compile(pattern)?;
                    match label {
                        Value::String(label) => Some((glob, basename, label.clone())),
                        _ => None,
                    }
                })
                .collect(),
            _ => Vec::new(),
        };
        let min_width = number("workbench.editor.tabSizingFixedMinWidth")?.max(1.0) as f32;
        let tab_limit = if flag("workbench.editor.limit.enabled")? {
            Some(number("workbench.editor.limit.value")?.max(1.0) as usize)
        } else {
            None
        };
        Ok(Self {
            editors: EditorTypes::new(object("workbench.editorAssociations")?)?,
            close_empty: flag("workbench.editor.closeEmptyGroups")?,
            close_on_delete: flag("workbench.editor.closeOnFileDelete")?,
            pinned_row: flag("workbench.editor.pinnedTabsOnSeparateRow")?,
            restore_view: flag("workbench.editor.restoreViewState")?,
            reveal_if_open: flag("workbench.editor.revealIfOpen")?,
            tab_limit,
            limit_per_group: flag("workbench.editor.limit.perEditorGroup")?,
            limit_exclude_dirty: flag("workbench.editor.limit.excludeDirty")?,
            tabs: match value("workbench.editor.showTabs")?.as_str() {
                "none" => Tabs::None,
                "single" => Tabs::Single,
                _ => Tabs::Multiple,
            },
            sizing: match value("workbench.editor.tabSizing")?.as_str() {
                "fixed" => Sizing::Fixed,
                "shrink" => Sizing::Shrink,
                _ => Sizing::Fit,
            },
            min_width,
            max_width: (number("workbench.editor.tabSizingFixedMaxWidth")? as f32).max(min_width),
            wrap: flag("workbench.editor.wrapTabs")?,
            actions_left: value("workbench.editor.tabActionLocation")? == "left",
            show_index: flag("workbench.editor.showTabIndex")?,
            tab_height: if value("window.density.editorTabHeight")? == "compact" {
                22.0
            } else {
                35.0
            },
            double_click: value("workbench.editor.doubleClickTabToToggleEditorGroupSizes")?,
            secondary_vertical: value("workbench.editor.splitInGroupLayout")? == "vertical",
            highlight_modified: flag("workbench.editor.highlightModifiedTabs")?,
            badges: flag("workbench.editor.decorations.badges")?,
            colors: flag("workbench.editor.decorations.colors")?,
            size: SizePolicy {
                huge_bytes: number("rove.largeFile.threshold")?.max(1.0) as u64,
                optimize: flag("editor.largeFileOptimizations")?,
            },
            explorer_preserve_focus: match flag("explorer.openFilePreserveFocus") {
                Ok(value) => value,
                Err(Error::NotFound) => false, // Registered by the explorer when installed.
                Err(error) => return Err(error),
            },
            preview: flag("workbench.editor.enablePreview")?,
            preview_quick: flag("workbench.editor.enablePreviewFromQuickOpen")?,
            preview_code: flag("workbench.editor.enablePreviewFromCodeNavigation")?,
            position: value("workbench.editor.openPositioning")?,
            custom_labels,
        })
    }

    pub(crate) fn preview(&self, options: &OpenOptions) -> bool {
        self.preview
            && options.preview_tab
            && match options.source {
                OpenSource::QuickOpen => self.preview_quick,
                OpenSource::CodeNavigation => self.preview_code,
                _ => true,
            }
    }

    pub(crate) fn insert_at(&self, active: Option<usize>, count: usize) -> usize {
        match self.position.as_str() {
            "first" => 0,
            "last" => count,
            "left" => active.unwrap_or(0),
            _ => active.map_or(count, |index| index + 1),
        }
    }

    pub(crate) fn tab_label(&self, resource: &Resource) -> String {
        let path = resource_path(resource);
        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        if let Some((_, _, pattern)) = self
            .custom_labels
            .iter()
            .rev()
            .find(|(glob, basename, _)| matches(glob, *basename, path))
        {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let ext = path.extension().unwrap_or_default().to_string_lossy();
            let dirname = path
                .parent()
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy();
            pattern
                .replace("${filename}", &stem)
                .replace("${extname}", &ext)
                .replace("${dirname}", &dirname)
        } else {
            filename.into_owned()
        }
    }
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self::read(&DefaultSettings(
            register().expect("unique editor settings"),
        ))
        .expect("valid editor defaults")
    }
}
