//! 用例文件格式（`tests/nav-cases/*.toml`）与加载、静态校验。
//!
//! 本模块不感知索引格式与查询接口：它只描述“在哪里、期望跳到哪里”。
//! 字段含义见 `tests/nav-cases/README.md`。

use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

/// 场景分类。键名写在 TOML 里，标签用于报告。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Var,
    Chain,
    Overload,
    Extension,
    Generic,
    Inheritance,
    Interface,
    Partial,
    Lambda,
    Conditional,
    External,
    Local,
    Event,
    Operator,
    Indexer,
    TypeRef,
}

impl Category {
    pub const ALL: [Category; 16] = [
        Category::Var,
        Category::Chain,
        Category::Overload,
        Category::Extension,
        Category::Generic,
        Category::Inheritance,
        Category::Interface,
        Category::Partial,
        Category::Lambda,
        Category::Conditional,
        Category::External,
        Category::Local,
        Category::Event,
        Category::Operator,
        Category::Indexer,
        Category::TypeRef,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Category::Var => "var",
            Category::Chain => "chain",
            Category::Overload => "overload",
            Category::Extension => "extension",
            Category::Generic => "generic",
            Category::Inheritance => "inheritance",
            Category::Interface => "interface",
            Category::Partial => "partial",
            Category::Lambda => "lambda",
            Category::Conditional => "conditional",
            Category::External => "external",
            Category::Local => "local",
            Category::Event => "event",
            Category::Operator => "operator",
            Category::Indexer => "indexer",
            Category::TypeRef => "type_ref",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Var => "var 推断",
            Category::Chain => "链式调用",
            Category::Overload => "方法重载",
            Category::Extension => "扩展方法",
            Category::Generic => "泛型",
            Category::Inheritance => "继承与重写",
            Category::Interface => "接口实现",
            Category::Partial => "partial",
            Category::Lambda => "Lambda / LINQ",
            Category::Conditional => "条件编译",
            Category::External => "外部符号",
            Category::Local => "局部变量与参数",
            Category::Event => "事件 / 委托",
            Category::Operator => "运算符重载",
            Category::Indexer => "索引器",
            Category::TypeRef => "类型引用（#37 验收）",
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// `#if` 的哪一侧（只有 `conditional` 分类需要）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Branch {
    /// 按项目宏求值后生效的一侧。
    Active,
    /// 不生效的一侧（`#else` 或 `#if !UNITY_EDITOR` 等）。
    Inactive,
}

/// 一个位置锚点：相对路径 + 1 起算的行号 + 该行去掉首尾空白后的文本。
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineAnchor {
    pub file: String,
    pub line: u32,
    pub anchor: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// 源码中的定义位置（partial 类型等可能有多处，需全部列出）。
    #[serde(default)]
    pub defs: Vec<LineAnchor>,
    /// 外部符号的文档注释 ID（如 `M:UnityEngine.Component.GetComponent``1`）。
    pub external: Option<String>,
    /// 精确的引用数量（不含定义处），只用于引用较少的符号。
    pub refs: Option<u32>,
    /// 引用列表中必须包含的位置（引用多的符号用它代替数量）。
    #[serde(default)]
    pub refs_include: Vec<LineAnchor>,
    /// 期望的精确度标记（`exact` / `approximate` / `syntax`）；查询来源不报告精确度时不检查。
    pub precision: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub category: Category,
    /// 只有 `conditional` 分类填写。
    pub branch: Option<Branch>,
    /// 相对项目根目录的路径，斜杠分隔。
    pub file: String,
    /// 1 起算的行号。
    pub line: u32,
    /// 1 起算的列号，按 Unicode 字符计（制表符算 1 列），指向 `token` 的第一个字符。
    pub col: u32,
    /// 光标所在的标识符或运算符文本。
    pub token: String,
    /// 该行去掉首尾空白后的完整文本，用于代码变动后的重定位。
    pub anchor: String,
    /// 为什么选这个点（一句话）。
    pub why: String,
    /// 期望值的来源（如何确认的）。
    pub source: String,
    pub expect: Expect,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFileRaw {
    project: String,
    #[serde(default, rename = "case")]
    cases: Vec<Case>,
}

/// 测试项目槽位，对应 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Project {
    Medium,
    Large,
}

impl Project {
    pub const ALL: [Project; 2] = [Project::Medium, Project::Large];

    pub fn key(self) -> &'static str {
        match self {
            Project::Medium => "medium",
            Project::Large => "large",
        }
    }

    pub fn parse(s: &str) -> Option<Project> {
        match s {
            "medium" => Some(Project::Medium),
            "large" => Some(Project::Large),
            _ => None,
        }
    }
}

pub struct CaseFile {
    pub path: PathBuf,
    pub project: Project,
    pub cases: Vec<Case>,
}

pub fn parse_case_file(path: &Path, text: &str) -> Result<CaseFile, String> {
    let raw: CaseFileRaw =
        toml::from_str(text).map_err(|e| format!("{}: TOML 解析失败：{e}", path.display()))?;
    let project = Project::parse(&raw.project).ok_or_else(|| {
        format!(
            "{}: project 必须是 medium 或 large，实际为 {:?}",
            path.display(),
            raw.project
        )
    })?;
    Ok(CaseFile {
        path: path.to_owned(),
        project,
        cases: raw.cases,
    })
}

/// 读取目录下全部 `*.toml`（按文件名排序）。
pub fn load_dir(dir: &Path) -> Result<Vec<CaseFile>, String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("读取用例目录 {} 失败：{e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p)
                .map_err(|e| format!("读取 {} 失败：{e}", p.display()))?;
            parse_case_file(p, &text)
        })
        .collect()
}

fn check_rel_path(what: &str, p: &str, errs: &mut Vec<String>) {
    let bytes = p.as_bytes();
    let absolute = p.starts_with('/')
        || p.starts_with('\\')
        || (bytes.len() >= 2 && bytes[1] == b':')
        || p.contains("://");
    if absolute {
        errs.push(format!("{what}: 不允许绝对路径：{p}"));
    }
    if p.contains('\\') {
        errs.push(format!("{what}: 路径须用斜杠分隔：{p}"));
    }
    if p.split('/').any(|c| c == "..") {
        errs.push(format!("{what}: 路径不能含 ..：{p}"));
    }
}

/// 单个用例的静态校验（不读项目文件）。
pub fn lint_case(c: &Case) -> Vec<String> {
    let mut errs = Vec::new();
    let who = format!("用例 {}", c.id);
    check_rel_path(&who, &c.file, &mut errs);
    if c.line == 0 || c.col == 0 {
        errs.push(format!("{who}: line / col 从 1 起算"));
    }
    if c.token.is_empty() || c.token.trim() != c.token {
        errs.push(format!("{who}: token 不能为空或带首尾空白"));
    }
    if c.anchor.is_empty() || c.anchor.trim() != c.anchor {
        errs.push(format!("{who}: anchor 须为去掉首尾空白后的整行文本"));
    } else if !c.anchor.contains(&c.token) {
        errs.push(format!("{who}: anchor 中找不到 token {:?}", c.token));
    }
    if c.why.trim().is_empty() || c.source.trim().is_empty() {
        errs.push(format!("{who}: why 与 source 必填"));
    }
    match (c.category, c.branch) {
        (Category::Conditional, None) => {
            errs.push(format!("{who}: conditional 分类必须填写 branch"))
        }
        (Category::Conditional, Some(_)) | (_, None) => {}
        (_, Some(_)) => errs.push(format!("{who}: 只有 conditional 分类可以填写 branch")),
    }
    let e = &c.expect;
    match (e.defs.is_empty(), e.external.is_some()) {
        (true, false) => errs.push(format!("{who}: expect.defs 与 expect.external 必须填一个")),
        (false, true) => errs.push(format!("{who}: expect.defs 与 expect.external 只能填一个")),
        _ => {}
    }
    if e.refs.is_none() && e.refs_include.is_empty() {
        errs.push(format!(
            "{who}: expect.refs 与 expect.refs_include 至少填一个"
        ));
    }
    if let Some(n) = e.refs {
        if n >= 20 {
            errs.push(format!(
                "{who}: 引用 ≥ 20 的符号不要断言精确数量（{n}），改用 refs_include"
            ));
        }
    }
    if let Some(p) = &e.precision {
        if !matches!(p.as_str(), "exact" | "approximate" | "syntax") {
            errs.push(format!(
                "{who}: precision 只能是 exact / approximate / syntax"
            ));
        }
    }
    for a in e.defs.iter().chain(&e.refs_include) {
        check_rel_path(&who, &a.file, &mut errs);
        if a.line == 0 || a.anchor.is_empty() || a.anchor.trim() != a.anchor {
            errs.push(format!(
                "{who}: 期望位置 {}:{} 的 line 从 1 起算、anchor 为整行去空白文本",
                a.file, a.line
            ));
        }
    }
    errs
}

/// 每个项目每类至少多少个（issue #39：每类至少 5 个）。
pub const MIN_PER_CATEGORY: usize = 5;

/// 整套用例的静态校验：id 唯一、单个用例合法、分类覆盖。
/// `require_full` 为真时还检查“每个项目 100 个、每类至少 5 个、条件编译两侧都有”。
pub fn lint_all(files: &[CaseFile], require_full: bool) -> Vec<String> {
    let mut errs = Vec::new();
    let mut ids = HashSet::new();
    let mut per: BTreeMap<(Project, Category), usize> = BTreeMap::new();
    let mut per_project: BTreeMap<Project, usize> = BTreeMap::new();
    let mut branches: BTreeMap<Project, HashSet<&'static str>> = BTreeMap::new();
    for f in files {
        for c in &f.cases {
            if !ids.insert(c.id.clone()) {
                errs.push(format!("{}: 用例 id 重复：{}", f.path.display(), c.id));
            }
            errs.extend(lint_case(c));
            *per.entry((f.project, c.category)).or_default() += 1;
            *per_project.entry(f.project).or_default() += 1;
            if let Some(b) = c.branch {
                branches.entry(f.project).or_default().insert(match b {
                    Branch::Active => "active",
                    Branch::Inactive => "inactive",
                });
            }
        }
    }
    if require_full {
        for p in Project::ALL {
            let n = per_project.get(&p).copied().unwrap_or(0);
            if n != 100 {
                errs.push(format!("{} 项目应有 100 个用例，实际 {n}", p.key()));
            }
            for cat in Category::ALL {
                let n = per.get(&(p, cat)).copied().unwrap_or(0);
                if n < MIN_PER_CATEGORY {
                    errs.push(format!(
                        "{} 项目的 {} 分类只有 {n} 个，至少 {MIN_PER_CATEGORY} 个",
                        p.key(),
                        cat.key()
                    ));
                }
            }
            let b = branches.get(&p);
            for side in ["active", "inactive"] {
                if !b.is_some_and(|b| b.contains(side)) {
                    errs.push(format!("{} 项目缺少条件编译 {side} 一侧的用例", p.key()));
                }
            }
        }
    }
    errs
}
