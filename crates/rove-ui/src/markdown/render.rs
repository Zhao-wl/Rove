//! Markdown → 净化后的 HTML（#29）。纯计算，不依赖 GPUI，在后台执行器上运行。
//!
//! - 解析：`pulldown-cmark`（CommonMark + GFM 表格、任务列表、删除线、脚注、提示块、`$` 数学公式、
//!   YAML front matter）。HTML 由本模块自己输出，以便给块级元素加上 VS Code 预览同样的
//!   `class="code-line" data-line="<从 0 开始的源码行>"`，供滚动同步与双击定位使用（VS Code 的
//!   `markdown.css` 也按这两个属性画“编辑器选区标记”）。
//! - 安全：整份输出经过 `ammonia` 白名单净化（文档里的 `<script>`、事件属性、`<iframe>`、`javascript:`
//!   链接一律去掉），之后才交给 WebView2；页面的 CSP 只允许带随机 nonce 的打包脚本，是第二道防线。
//! - 资源：图片等资源地址在净化时改写：相对路径与本地绝对路径 → `https://<盘符>.rove-file.local/...`
//!   （WebView2 按盘符映射到本地目录）；远程地址（`http(s)://`、`//host`、UNC 网络路径）在没有放行时
//!   换成打包的占位图并计数，放行后原样保留。
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, CowStr, Event, HeadingLevel, MetadataBlockKind,
    Options, Parser, Tag, TagEnd,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Component, Path, PathBuf, Prefix},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

/// 打包资源的虚拟主机（WebView2 只从这里与本地文件主机加载内容）。
pub const ASSET_ORIGIN: &str = "https://rove.local/";
/// 本地文件主机的后缀：`https://c.rove-file.local/Users/...` 对应 `C:\Users\...`。
pub const FILE_HOST_SUFFIX: &str = ".rove-file.local";
/// 被阻止的远程图片换成的占位图。
pub const BLOCKED_IMAGE: &str = "https://rove.local/blocked-image.svg";

/// `markdown.preview.frontMatter`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontMatter {
    Hide,
    Table,
    CodeBlock,
}

/// 提示块（`> [!NOTE]`）的标题，来自登记过的界面字符串。
#[derive(Clone, Debug)]
pub struct AlertTitles {
    pub note: String,
    pub tip: String,
    pub important: String,
    pub warning: String,
    pub caution: String,
}

impl Default for AlertTitles {
    fn default() -> Self {
        Self {
            note: "Note".into(),
            tip: "Tip".into(),
            important: "Important".into(),
            warning: "Warning".into(),
            caution: "Caution".into(),
        }
    }
}

/// 渲染选项：`markdown.preview.*` 与 `markdown.math.enabled` 的值。
#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub breaks: bool,
    pub linkify: bool,
    pub typographer: bool,
    pub math: bool,
    pub front_matter: FrontMatter,
    pub alerts: AlertTitles,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            breaks: false,
            linkify: true,
            typographer: false,
            math: true,
            front_matter: FrontMatter::Table,
            alerts: AlertTitles::default(),
        }
    }
}

/// 渲染结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rendered {
    /// 净化后的正文 HTML（放进预览页的 `<body>`）。
    pub html: String,
    /// 文档引用的远程资源个数（已替换成占位图）。
    pub remote: usize,
    /// 正文里引用到的本地盘符（小写），WebView2 需要为它们建立文件主机映射。
    pub drives: BTreeSet<char>,
    /// 源码总行数（滚动同步按行插值的上限）。
    pub lines: usize,
}

/// 把文档渲染成净化后的 HTML。`base` 是文档所在目录，用于解析相对路径；没有时相对路径按远程处理以外的
/// “无法解析”丢弃。
pub fn render(text: &str, base: Option<&Path>, options: &RenderOptions) -> Rendered {
    let raw = to_html(text, options);
    let remote = Arc::new(AtomicUsize::new(0));
    let drives = Arc::new(Mutex::new(BTreeSet::new()));
    let html = sanitize(
        &raw,
        base.map(Path::to_path_buf),
        remote.clone(),
        drives.clone(),
    );
    let drives = std::mem::take(&mut *drives.lock().unwrap());
    Rendered {
        html,
        remote: remote.load(Ordering::Relaxed),
        drives,
        lines: line_starts(text).len(),
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .filter(|&i| i < text.len() || i == 0)
        .collect()
}

fn escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

/// VS Code `githubSlugifier`：小写、空白换成 `-`、去掉标点，重复时加 `-1`、`-2`。
pub fn slugify(text: &str) -> String {
    const PUNCTUATION: &str = "[]!/'\"#$%&()*+,.:;<=>?@\\^{|}~`。，、；：？！…—·ˉ¨‘’“”々～‖∶＂＇｀｜〃〔〕〈〉《》「」『』．〖〗【】（）［］｛｝";
    let lower = text.trim().to_lowercase();
    let mut slug = String::with_capacity(lower.len());
    let mut space = false;
    for c in lower.chars() {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space {
            slug.push('-');
            space = false;
        }
        if !PUNCTUATION.contains(c) {
            slug.push(c);
        }
    }
    slug.trim_matches('-').to_string()
}

struct Writer<'o> {
    out: String,
    lines: Vec<usize>,
    options: &'o RenderOptions,
    /// 表头/表体：表头单元格用 `th`。
    in_head: bool,
    alignments: Vec<Alignment>,
    cell: usize,
    /// 图片的替代文字收集中（`![alt](src)` 的 alt 是内部事件的纯文本）。
    image: Option<(String, String, String)>,
    /// 当前在链接或代码里：不做自动链接。
    no_linkify: usize,
    /// 标题：收集文字以生成锚点 id。
    heading: Option<(HeadingLevel, Option<String>, usize, usize, String)>,
    slugs: BTreeMap<String, usize>,
    /// front matter 的原文。
    metadata: Option<(usize, String)>,
    code: Option<(usize, String, String)>,
}

impl Writer<'_> {
    fn line(&self, offset: usize) -> usize {
        self.lines
            .partition_point(|&start| start <= offset)
            .saturating_sub(1)
    }

    fn open(&mut self, tag: &str, class: &str, offset: usize) {
        let line = self.line(offset);
        let _ = write!(
            self.out,
            "<{tag} class=\"code-line{class}\" data-line=\"{line}\""
        );
    }

    fn text(&mut self, text: &str) {
        if let Some((_, _, _, _, buffer)) = self.heading.as_mut() {
            buffer.push_str(text);
        }
        if let Some((alt, _, _)) = self.image.as_mut() {
            alt.push_str(text);
            return;
        }
        if self.options.linkify && self.no_linkify == 0 {
            linkify(&mut self.out, text);
        } else {
            escape(&mut self.out, text);
        }
    }

    fn start(&mut self, tag: Tag, offset: usize) {
        match tag {
            Tag::Paragraph => {
                self.open("p", "", offset);
                self.out.push('>');
            }
            Tag::Heading { level, id, .. } => {
                // 标题的 id 要等文字收集完才知道：先记下位置，结束时补上开始标签。
                let at = self.out.len();
                self.heading = Some((
                    level,
                    id.map(|id| id.to_string()),
                    at,
                    offset,
                    String::new(),
                ));
            }
            Tag::BlockQuote(kind) => match kind {
                None => {
                    self.open("blockquote", "", offset);
                    self.out.push('>');
                }
                Some(kind) => {
                    let (name, title) = match kind {
                        BlockQuoteKind::Note => ("note", &self.options.alerts.note),
                        BlockQuoteKind::Tip => ("tip", &self.options.alerts.tip),
                        BlockQuoteKind::Important => ("important", &self.options.alerts.important),
                        BlockQuoteKind::Warning => ("warning", &self.options.alerts.warning),
                        BlockQuoteKind::Caution => ("caution", &self.options.alerts.caution),
                    };
                    let title = title.clone();
                    self.open(
                        "div",
                        &format!(" markdown-alert markdown-alert-{name}"),
                        offset,
                    );
                    self.out.push_str("><p class=\"markdown-alert-title\">");
                    escape(&mut self.out, &title);
                    self.out.push_str("</p>");
                }
            },
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c.is_whitespace() || c == '{' || c == ',')
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((offset, language, String::new()));
            }
            Tag::HtmlBlock => {}
            Tag::List(Some(start)) => {
                self.open("ol", "", offset);
                if start != 1 {
                    let _ = write!(self.out, " start=\"{start}\"");
                }
                self.out.push('>');
            }
            Tag::List(None) => {
                self.open("ul", "", offset);
                self.out.push('>');
            }
            Tag::Item => {
                self.open("li", "", offset);
                self.out.push('>');
            }
            Tag::FootnoteDefinition(name) => {
                self.open("div", " footnote-definition", offset);
                self.out.push_str(" id=\"fn-");
                escape(&mut self.out, &name);
                self.out
                    .push_str("\"><sup class=\"footnote-definition-label\">");
                escape(&mut self.out, &name);
                self.out.push_str("</sup>");
            }
            Tag::DefinitionList => {
                self.open("dl", "", offset);
                self.out.push('>');
            }
            Tag::DefinitionListTitle => self.out.push_str("<dt>"),
            Tag::DefinitionListDefinition => self.out.push_str("<dd>"),
            Tag::Table(alignments) => {
                self.alignments = alignments;
                self.open("table", "", offset);
                self.out.push('>');
            }
            Tag::TableHead => {
                self.in_head = true;
                self.cell = 0;
                self.open("thead><tr", "", offset);
                self.out.push('>');
            }
            Tag::TableRow => {
                self.cell = 0;
                self.open("tr", "", offset);
                self.out.push('>');
            }
            Tag::TableCell => {
                let tag = if self.in_head { "th" } else { "td" };
                let align = match self.alignments.get(self.cell) {
                    Some(Alignment::Left) => " style=\"text-align:left\"",
                    Some(Alignment::Center) => " style=\"text-align:center\"",
                    Some(Alignment::Right) => " style=\"text-align:right\"",
                    _ => "",
                };
                let _ = write!(self.out, "<{tag}{align}>");
            }
            Tag::Emphasis => self.out.push_str("<em>"),
            Tag::Strong => self.out.push_str("<strong>"),
            Tag::Strikethrough => self.out.push_str("<del>"),
            Tag::Superscript => self.out.push_str("<sup>"),
            Tag::Subscript => self.out.push_str("<sub>"),
            Tag::Link {
                dest_url, title, ..
            } => {
                self.no_linkify += 1;
                self.out.push_str("<a href=\"");
                escape(&mut self.out, &dest_url);
                self.out.push('"');
                if !title.is_empty() {
                    self.out.push_str(" title=\"");
                    escape(&mut self.out, &title);
                    self.out.push('"');
                }
                self.out.push('>');
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                self.image = Some((String::new(), dest_url.to_string(), title.to_string()));
            }
            Tag::MetadataBlock(_) => self.metadata = Some((offset, String::new())),
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.out.push_str("</p>\n"),
            TagEnd::Heading(_) => {
                let Some((level, id, at, offset, text)) = self.heading.take() else {
                    return;
                };
                let level = level as usize;
                let base = id.unwrap_or_else(|| slugify(&text));
                let count = self.slugs.entry(base.clone()).or_insert(0);
                let id = if *count == 0 {
                    base.clone()
                } else {
                    format!("{base}-{count}")
                };
                *count += 1;
                let mut open = String::new();
                let line = self.line(offset);
                let _ = write!(
                    open,
                    "<h{level} class=\"code-line\" data-line=\"{line}\" id=\""
                );
                escape(&mut open, &id);
                open.push_str("\">");
                self.out.insert_str(at, &open);
                let _ = writeln!(self.out, "</h{level}>");
            }
            TagEnd::BlockQuote(None) => self.out.push_str("</blockquote>\n"),
            TagEnd::BlockQuote(Some(_)) => self.out.push_str("</div>\n"),
            TagEnd::CodeBlock => {
                let Some((offset, language, body)) = self.code.take() else {
                    return;
                };
                if language.eq_ignore_ascii_case("mermaid") {
                    self.open("pre", " mermaid", offset);
                    self.out.push('>');
                    escape(&mut self.out, &body);
                    self.out.push_str("</pre>\n");
                } else {
                    self.open("pre", "", offset);
                    self.out.push_str("><code");
                    if !language.is_empty() {
                        self.out.push_str(" class=\"language-");
                        escape(&mut self.out, &language);
                        self.out.push('"');
                    }
                    self.out.push('>');
                    escape(&mut self.out, &body);
                    self.out.push_str("</code></pre>\n");
                }
            }
            TagEnd::HtmlBlock => {}
            TagEnd::List(true) => self.out.push_str("</ol>\n"),
            TagEnd::List(false) => self.out.push_str("</ul>\n"),
            TagEnd::Item => self.out.push_str("</li>\n"),
            TagEnd::FootnoteDefinition => self.out.push_str("</div>\n"),
            TagEnd::DefinitionList => self.out.push_str("</dl>\n"),
            TagEnd::DefinitionListTitle => self.out.push_str("</dt>\n"),
            TagEnd::DefinitionListDefinition => self.out.push_str("</dd>\n"),
            TagEnd::Table => self.out.push_str("</tbody></table>\n"),
            TagEnd::TableHead => {
                self.in_head = false;
                self.out.push_str("</tr></thead><tbody>\n");
            }
            TagEnd::TableRow => self.out.push_str("</tr>\n"),
            TagEnd::TableCell => {
                self.out
                    .push_str(if self.in_head { "</th>" } else { "</td>" });
                self.cell += 1;
            }
            TagEnd::Emphasis => self.out.push_str("</em>"),
            TagEnd::Strong => self.out.push_str("</strong>"),
            TagEnd::Strikethrough => self.out.push_str("</del>"),
            TagEnd::Superscript => self.out.push_str("</sup>"),
            TagEnd::Subscript => self.out.push_str("</sub>"),
            TagEnd::Link => {
                self.no_linkify = self.no_linkify.saturating_sub(1);
                self.out.push_str("</a>");
            }
            TagEnd::Image => {
                if let Some((alt, src, title)) = self.image.take() {
                    self.out.push_str("<img src=\"");
                    escape(&mut self.out, &src);
                    self.out.push_str("\" alt=\"");
                    escape(&mut self.out, &alt);
                    self.out.push('"');
                    if !title.is_empty() {
                        self.out.push_str(" title=\"");
                        escape(&mut self.out, &title);
                        self.out.push('"');
                    }
                    self.out.push('>');
                }
            }
            TagEnd::MetadataBlock(kind) => {
                if let Some((offset, body)) = self.metadata.take() {
                    self.front_matter(offset, kind, &body);
                }
            }
        }
    }

    fn front_matter(&mut self, offset: usize, kind: MetadataBlockKind, body: &str) {
        let pairs: Option<Vec<(&str, &str)>> = body
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (key, value) = line.split_once(':')?;
                (!key.starts_with(' ') && !key.trim().is_empty())
                    .then(|| (key.trim(), value.trim()))
            })
            .collect();
        match (self.options.front_matter, pairs) {
            (FrontMatter::Hide, _) => {}
            (FrontMatter::Table, Some(pairs)) if !pairs.is_empty() => {
                self.open("table", " front-matter", offset);
                self.out.push_str("><thead><tr>");
                for (key, _) in &pairs {
                    self.out.push_str("<th>");
                    escape(&mut self.out, key);
                    self.out.push_str("</th>");
                }
                self.out.push_str("</tr></thead><tbody><tr>");
                for (_, value) in &pairs {
                    self.out.push_str("<td>");
                    escape(&mut self.out, value.trim_matches(|c| c == '"' || c == '\''));
                    self.out.push_str("</td>");
                }
                self.out.push_str("</tr></tbody></table>\n");
            }
            _ => {
                let language = match kind {
                    MetadataBlockKind::YamlStyle => "yaml",
                    MetadataBlockKind::PlusesStyle => "toml",
                };
                self.open("pre", " front-matter", offset);
                let _ = write!(self.out, "><code class=\"language-{language}\">");
                escape(&mut self.out, body);
                self.out.push_str("</code></pre>\n");
            }
        }
    }
}

/// `markdown.preview.linkify`：把文字中的 `http(s)://…` 与 `www.…` 变成链接（去掉末尾标点、
/// 不配对的右括号），与 linkify-it 的常见行为一致。
fn linkify(out: &mut String, text: &str) {
    let mut rest = text;
    while let Some(start) = find_url(rest) {
        let (before, tail) = rest.split_at(start);
        escape(out, before);
        let mut end = tail
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(c, '<' | '>' | '"')
                    || "。，、；：？！）》」』】".contains(c)
            })
            .unwrap_or(tail.len());
        loop {
            let candidate = &tail[..end];
            let last = candidate.chars().last();
            let unbalanced = last == Some(')')
                && candidate.matches('(').count() < candidate.matches(')').count();
            if matches!(
                last,
                Some('.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*' | '_')
            ) || unbalanced
            {
                end -= last.unwrap().len_utf8();
            } else {
                break;
            }
        }
        let url = &tail[..end];
        let href = if url.starts_with("www.") {
            Cow::Owned(format!("http://{url}"))
        } else {
            Cow::Borrowed(url)
        };
        out.push_str("<a href=\"");
        escape(out, &href);
        out.push_str("\">");
        escape(out, url);
        out.push_str("</a>");
        rest = &tail[end..];
    }
    escape(out, rest);
}

fn find_url(text: &str) -> Option<usize> {
    let mut search = 0;
    while search < text.len() {
        let found = ["https://", "http://", "www."]
            .iter()
            .filter_map(|p| text[search..].find(p).map(|i| (i + search, p.len())))
            .min()?;
        let (at, len) = found;
        // 单词中间（例如 `abchttp://`）不算；后面至少要有一个字符。
        let boundary = text[..at]
            .chars()
            .last()
            .is_none_or(|c| !c.is_alphanumeric() && c != '/' && c != '.');
        let has_host = text[at + len..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric());
        if boundary && has_host {
            return Some(at);
        }
        search = at + len;
    }
    None
}

fn to_html(text: &str, options: &RenderOptions) -> String {
    let mut flags = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    if options.typographer {
        flags |= Options::ENABLE_SMART_PUNCTUATION;
    }
    if options.math {
        flags |= Options::ENABLE_MATH;
    }
    let mut writer = Writer {
        out: String::with_capacity(text.len() * 2),
        lines: line_starts(text),
        options,
        in_head: false,
        alignments: Vec::new(),
        cell: 0,
        image: None,
        no_linkify: 0,
        heading: None,
        slugs: BTreeMap::new(),
        metadata: None,
        code: None,
    };
    for (event, range) in Parser::new_ext(text, flags).into_offset_iter() {
        let offset = range.start;
        if let Some((_, _, body)) = writer.code.as_mut() {
            if let Event::Text(text) = &event {
                body.push_str(text);
                continue;
            }
        }
        if let Some((_, body)) = writer.metadata.as_mut() {
            if let Event::Text(text) = &event {
                body.push_str(text);
                continue;
            }
        }
        match event {
            Event::Start(tag) => writer.start(tag, offset),
            Event::End(tag) => writer.end(tag),
            Event::Text(text) => writer.text(&text),
            Event::Code(code) => {
                if let Some((_, _, _, _, buffer)) = writer.heading.as_mut() {
                    buffer.push_str(&code);
                }
                writer.out.push_str("<code>");
                escape(&mut writer.out, &code);
                writer.out.push_str("</code>");
            }
            Event::InlineMath(tex) => math(&mut writer.out, &tex, false),
            Event::DisplayMath(tex) => math(&mut writer.out, &tex, true),
            Event::Html(html) | Event::InlineHtml(html) => writer.out.push_str(&html),
            Event::FootnoteReference(name) => {
                writer
                    .out
                    .push_str("<sup class=\"footnote-reference\"><a href=\"#fn-");
                escape(&mut writer.out, &name);
                writer.out.push_str("\">");
                escape(&mut writer.out, &name);
                writer.out.push_str("</a></sup>");
            }
            Event::SoftBreak => writer
                .out
                .push_str(if options.breaks { "<br>\n" } else { "\n" }),
            Event::HardBreak => writer.out.push_str("<br>\n"),
            Event::Rule => {
                writer.open("hr", "", offset);
                writer.out.push_str(">\n");
            }
            Event::TaskListMarker(checked) => writer.out.push_str(if checked {
                "<input type=\"checkbox\" class=\"task-list-item-checkbox\" checked disabled> "
            } else {
                "<input type=\"checkbox\" class=\"task-list-item-checkbox\" disabled> "
            }),
        }
    }
    writer.out
}

fn math(out: &mut String, tex: &CowStr, display: bool) {
    out.push_str(if display {
        "<span class=\"rove-math rove-math-display\">"
    } else {
        "<span class=\"rove-math\">"
    });
    escape(out, tex);
    out.push_str("</span>");
}

/// 净化后可以留下的 CSS 属性（不含任何能带 `url()` 的属性）。
const STYLE_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "text-align",
    "vertical-align",
    "font-weight",
    "font-style",
    "font-size",
    "text-decoration",
    "width",
    "height",
    "max-width",
    "min-width",
    "display",
    "float",
    "margin",
    "margin-left",
    "margin-right",
    "margin-top",
    "margin-bottom",
    "padding",
    "padding-left",
    "padding-right",
    "padding-top",
    "padding-bottom",
    "border",
    "border-collapse",
    "white-space",
];

/// 本地文件主机 URL：`C:\a b\c.png` → `https://c.rove-file.local/a%20b/c.png`。
pub fn file_url(path: &Path) -> Option<(char, String)> {
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return None;
    };
    let drive = match prefix.kind() {
        Prefix::Disk(d) | Prefix::VerbatimDisk(d) => (d as char).to_ascii_lowercase(),
        _ => return None,
    };
    let mut url = format!("https://{drive}{FILE_HOST_SUFFIX}");
    let mut parts: Vec<String> = Vec::new();
    for component in components {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::Prefix(_) => return None,
        }
    }
    for part in parts {
        url.push('/');
        for byte in part.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=@".contains(&byte) {
                url.push(byte as char);
            } else {
                let _ = write!(url, "%{byte:02X}");
            }
        }
    }
    Some((drive, url))
}

/// 百分号解码（相对链接里的 `%20` 等）。
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|v| v as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(high << 4 | low);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 资源地址的分类。
#[derive(Debug, PartialEq)]
pub enum Target {
    /// 远程（含 `//host` 与 UNC 网络路径）。
    Remote,
    /// 本地文件（已解析为绝对路径）与片段。
    Local(PathBuf, Option<String>),
    /// 文档内锚点。
    Fragment(String),
    /// 打包资源、`data:` 等无需改写的地址。
    Inline,
    /// 其他协议（`mailto:` 等）或无法解析。
    Other,
}

/// 按文档目录解析一个链接或资源地址。
pub fn classify(value: &str, base: Option<&Path>) -> Target {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if let Some(fragment) = value.strip_prefix('#') {
        return Target::Fragment(percent_decode(fragment));
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        if lower.starts_with(ASSET_ORIGIN) || is_file_host(&lower) {
            return Target::Inline;
        }
        return Target::Remote;
    }
    if lower.starts_with("//") || lower.starts_with("\\\\") {
        return Target::Remote;
    }
    if lower.starts_with("data:") {
        return Target::Inline;
    }
    let path = if let Some(rest) = lower.strip_prefix("file:") {
        let rest = &value[value.len() - rest.len()..];
        let rest = rest.trim_start_matches('/');
        // `file://server/share` 是网络路径。
        if lower.starts_with("file://") && !lower.starts_with("file:///") && !has_drive(rest) {
            return Target::Remote;
        }
        PathBuf::from(percent_decode(rest))
    } else if value.contains(':') && !has_drive(value) {
        return Target::Other;
    } else {
        let (path, fragment) = match value.split_once('#') {
            Some((p, f)) => (p, Some(percent_decode(f))),
            None => (value, None),
        };
        let path = path.split_once('?').map_or(path, |(p, _)| p);
        let decoded = PathBuf::from(percent_decode(path));
        let absolute = if decoded.is_absolute() || has_drive(path) {
            decoded
        } else {
            match base {
                Some(base) => base.join(decoded),
                None => return Target::Other,
            }
        };
        return Target::Local(absolute, fragment);
    };
    Target::Local(path, None)
}

fn has_drive(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_file_host(lower: &str) -> bool {
    lower
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| host.ends_with(FILE_HOST_SUFFIX))
}

fn sanitize(
    html: &str,
    base: Option<PathBuf>,
    remote: Arc<AtomicUsize>,
    drives: Arc<Mutex<BTreeSet<char>>>,
) -> String {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags([
            "input", "section", "picture", "source", "video", "audio", "font", "dl", "dt", "dd",
            "tfoot",
        ])
        .add_generic_attributes(["class", "id", "align", "style", "data-line", "dir"])
        .add_tag_attributes("input", ["type", "checked", "disabled"])
        .add_tag_attributes("img", ["title", "loading"])
        .add_tag_attributes("a", ["title", "name"])
        .add_tag_attributes("ol", ["type", "reversed"])
        .add_tag_attributes("li", ["value"])
        .add_tag_attributes("details", ["open"])
        .add_tag_attributes("source", ["src", "srcset", "type", "media"])
        .add_tag_attributes("video", ["src", "poster", "controls", "width", "height"])
        .add_tag_attributes("audio", ["src", "controls"])
        .add_tag_attributes("font", ["color", "size"])
        .add_tag_attributes("td", ["width", "valign"])
        .add_tag_attributes("th", ["width", "valign"])
        .add_url_schemes(["data", "file"])
        .link_rel(None)
        .filter_style_properties(STYLE_PROPERTIES.iter().copied().collect())
        .attribute_filter(move |element, attribute, value| {
            let resource = matches!(
                (element, attribute),
                ("img" | "source" | "video" | "audio", "src")
                    | ("video", "poster")
                    | ("source" | "img", "srcset")
            );
            if element == "input" && attribute == "type" && value != "checkbox" {
                return None;
            }
            if !resource {
                return Some(Cow::Borrowed(value));
            }
            // srcset 只保留第一个候选，避免逐项改写。
            let value = if attribute == "srcset" {
                value
                    .split(',')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .split(' ')
                    .next()
                    .unwrap_or_default()
            } else {
                value
            };
            match classify(value, base.as_deref()) {
                Target::Remote => {
                    remote.fetch_add(1, Ordering::Relaxed);
                    Some(Cow::Borrowed(BLOCKED_IMAGE))
                }
                Target::Local(path, _) => match file_url(&path) {
                    Some((drive, url)) => {
                        drives.lock().unwrap().insert(drive);
                        Some(Cow::Owned(url))
                    }
                    None => None,
                },
                Target::Inline => Some(Cow::Owned(value.to_string())),
                Target::Fragment(_) | Target::Other => None,
            }
        });
    builder.clean(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(text: &str) -> String {
        render(text, Some(Path::new("C:\\docs")), &RenderOptions::default()).html
    }

    #[test]
    fn block_elements_carry_source_lines() {
        let out = html("# 标题\n\n第一段\n\n- a\n- b\n\n```rust\nfn main() {}\n```\n");
        assert!(
            out.contains("<h1 class=\"code-line\" data-line=\"0\" id=\"标题\">标题</h1>"),
            "{out}"
        );
        assert!(
            out.contains("<p class=\"code-line\" data-line=\"2\">第一段</p>"),
            "{out}"
        );
        assert!(
            out.contains("<li class=\"code-line\" data-line=\"4\">a</li>"),
            "{out}"
        );
        assert!(
            out.contains("<li class=\"code-line\" data-line=\"5\">b</li>"),
            "{out}"
        );
        assert!(out.contains("<pre class=\"code-line\" data-line=\"7\"><code class=\"language-rust\">fn main() {}\n</code></pre>"), "{out}");
    }

    #[test]
    fn gfm_tables_task_lists_alerts_and_math() {
        let out = html(
            "| a | b |\n|:-|-:|\n| 1 | 2 |\n\n- [x] 完成\n- [ ] 未完成\n\n> [!WARNING]\n> 小心\n\n$E=mc^2$ 与 $$\\sum$$\n",
        );
        assert!(
            out.contains("<table class=\"code-line\" data-line=\"0\">"),
            "{out}"
        );
        assert!(
            out.contains("<th style=\"text-align:left\">a</th>"),
            "{out}"
        );
        assert!(
            out.contains("<td style=\"text-align:right\">2</td>"),
            "{out}"
        );
        assert!(out.contains("<input type=\"checkbox\" class=\"task-list-item-checkbox\" checked=\"\" disabled=\"\">"), "{out}");
        assert!(
            out.contains("markdown-alert markdown-alert-warning"),
            "{out}"
        );
        assert!(
            out.contains("<p class=\"markdown-alert-title\">Warning</p>"),
            "{out}"
        );
        assert!(
            out.contains("<span class=\"rove-math\">E=mc^2</span>"),
            "{out}"
        );
        assert!(
            out.contains("<span class=\"rove-math rove-math-display\">\\sum</span>"),
            "{out}"
        );
    }

    #[test]
    fn mermaid_blocks_keep_their_source() {
        let out = html("```mermaid\ngraph TD; A-->B\n```\n");
        assert!(
            out.contains(
                "<pre class=\"code-line mermaid\" data-line=\"0\">graph TD; A--&gt;B\n</pre>"
            ),
            "{out}"
        );
    }

    #[test]
    fn scripts_and_event_handlers_are_removed() {
        let out = html(
            "<script>window.bad = 1</script>\n\n<img src=\"a.png\" onerror=\"alert(1)\">\n\n<a href=\"javascript:alert(1)\">x</a>\n\n<iframe src=\"https://example.com\"></iframe>\n\n<details><summary>更多</summary>内容</details>\n",
        );
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("onerror"), "{out}");
        assert!(!out.contains("javascript"), "{out}");
        assert!(!out.contains("iframe"), "{out}");
        assert!(
            out.contains("<details><summary>更多</summary>内容</details>"),
            "{out}"
        );
    }

    #[test]
    fn remote_resources_are_blocked_and_counted() {
        let text = "![a](https://example.com/a.png)\n\n<img src=\"//cdn.example.com/b.png\">\n\n![c](images/c%20d.png)\n";
        let blocked = render(text, Some(Path::new("C:\\docs")), &RenderOptions::default());
        assert_eq!(blocked.remote, 2);
        assert!(!blocked.html.contains("example.com"), "{}", blocked.html);
        assert!(blocked.html.contains(BLOCKED_IMAGE));
        assert!(
            blocked
                .html
                .contains("https://c.rove-file.local/docs/images/c%20d.png"),
            "{}",
            blocked.html
        );
        assert_eq!(blocked.drives, BTreeSet::from(['c']));
    }

    #[test]
    fn unc_paths_count_as_remote() {
        assert_eq!(classify("\\\\server\\share\\a.png", None), Target::Remote);
        assert_eq!(classify("file://server/share/a.png", None), Target::Remote);
        assert_eq!(
            classify("file:///D:/x/a.png", None),
            Target::Local(PathBuf::from("D:/x/a.png"), None)
        );
    }

    #[test]
    fn links_are_classified_relative_to_the_document() {
        assert_eq!(
            classify("other.md#用法", Some(Path::new("C:\\docs"))),
            Target::Local(PathBuf::from("C:\\docs\\other.md"), Some("用法".into()))
        );
        assert_eq!(classify("#top", None), Target::Fragment("top".into()));
        assert_eq!(classify("https://example.com", None), Target::Remote);
        assert_eq!(classify("mailto:a@b.c", None), Target::Other);
    }

    #[test]
    fn options_change_rendering() {
        let breaks = render(
            "a\nb\n",
            None,
            &RenderOptions {
                breaks: true,
                ..Default::default()
            },
        );
        assert!(breaks.html.contains("a<br>\nb"), "{}", breaks.html);
        let linkified = html("见 https://example.com/x). 与 www.rove.dev。\n");
        assert!(
            linkified.contains("<a href=\"https://example.com/x\">https://example.com/x</a>)."),
            "{linkified}"
        );
        assert!(
            linkified.contains("<a href=\"http://www.rove.dev\">www.rove.dev</a>"),
            "{linkified}"
        );
        let plain = render(
            "https://example.com\n",
            None,
            &RenderOptions {
                linkify: false,
                ..Default::default()
            },
        );
        assert!(!plain.html.contains("<a"), "{}", plain.html);
        let typographer = render(
            "\"quote\" -- dash\n",
            None,
            &RenderOptions {
                typographer: true,
                ..Default::default()
            },
        );
        assert!(typographer.html.contains('“'), "{}", typographer.html);
        let no_math = render(
            "$x$\n",
            None,
            &RenderOptions {
                math: false,
                ..Default::default()
            },
        );
        assert!(!no_math.html.contains("rove-math"));
    }

    #[test]
    fn front_matter_modes() {
        let text = "---\ntitle: 说明\nauthor: rove\n---\n# A\n";
        let table = html(text);
        assert!(table.contains("<th>title</th><th>author</th>"), "{table}");
        assert!(table.contains("<td>说明</td>"), "{table}");
        assert!(table.contains("data-line=\"4\""), "{table}");
        let hidden = render(
            text,
            None,
            &RenderOptions {
                front_matter: FrontMatter::Hide,
                ..Default::default()
            },
        );
        assert!(!hidden.html.contains("title"), "{}", hidden.html);
    }

    #[test]
    fn heading_slugs_are_unique() {
        let out = html("# Hello, World!\n# Hello, World!\n## 中文 标题\n");
        assert!(out.contains("id=\"hello-world\""), "{out}");
        assert!(out.contains("id=\"hello-world-1\""), "{out}");
        assert!(out.contains("id=\"中文-标题\""), "{out}");
    }

    #[test]
    fn line_index_covers_long_documents() {
        let text: String = (0..2500).map(|i| format!("段落 {i}\n\n")).collect();
        let out = render(&text, None, &RenderOptions::default());
        assert!(out.html.contains("data-line=\"4998\""));
        assert_eq!(out.lines, 5000);
    }
}
