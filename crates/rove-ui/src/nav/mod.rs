//! C# 导航交互（#40）：转到定义 / 声明 / 实现、`Ctrl+悬停` 与 `Ctrl+点击`、悬停卡片、
//! 符号导航模式、语义级同名高亮与近似结果的呈现规则（UX_SPEC §2.2、§2.4、§6.1–§6.3）。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`registry`] | 命令（VS Code ID）、界面字符串、颜色键、上下文键、`NavAction` | 仅 action |
//! | [`settings`] | `editor.gotoLocation.*`、`editor.hover.*`、`editor.definitionLinkOpensInPeek`、`editor.occurrencesHighlight*` 的读取 | 否 |
//! | [`model`] | 查询结果 → 呈现方式、符号导航模式、近似提示去重、语义同名高亮区间 | 否 |
//! | [`hover`] | 悬停卡片内容、XML 文档注释 → 段落、卡片内滚动 | 否 |
//! | [`semantic`] | #38 的装配：可替换的语义来源、打开中的 C# 缓冲区 → `BufferStates` | 否 |
//! | [`view`] | 悬停卡片的绘制 | 是 |
//!
//! 工作台的装配在 `skeleton/workbench/nav_host.rs`：查询一律经 #1 的 C# 导航查询接口（`CsRouter`，
//! 逐条精确度用 `navigate_detailed`）在后台执行器上执行，打开文件一律经 #17 的打开文档服务
//! （`OpenSource::CodeNavigation`，因此自动进入导航历史），装饰 / 选区 / 滚动一律经 #19 的代码视图接口。

pub mod hover;
pub mod model;
pub mod registry;
pub mod semantic;
pub mod settings;
pub mod view;
