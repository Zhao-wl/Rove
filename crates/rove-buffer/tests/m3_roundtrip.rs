//! #51 M3 集成验收：**真实 Unity 项目文件**上的编码往返与保存失败路径。
//!
//! 与 #49 自己的入口测试（`tests/save.rs`）的区别：那里的文件全是构造出来的（内容短、编码干净、
//! 换行一致）；本 issue 的价值在于真实项目里那些奇怪的文件——半截 BOM、末尾没有换行、
//! 几万字符的单行、空文件、同一个文件里混着三种换行符。所以这里**先扫真实项目、再按扫到的东西
//! 组矩阵**，扫不到的类别如实记为「项目里没有」，由构造文件补上并在报告中注明。
//!
//! 原则：
//!
//! - **一个字节都不往测试项目里写**。所有往返都在临时目录的副本上做。
//! - 环境变量 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE` 未设置时**跳过而不是失败**
//!   （AGENTS.md §8），原因打到 stderr。
//! - 判据是**字节级**的：保存后的字节与原字节的差异必须恰好等于那一处改动。
//!
//! 运行：
//!
//! ```powershell
//! cargo test -p rove-buffer --test m3_roundtrip -j 1 -- --nocapture
//! ```
//!
//! `--nocapture` 会打印每个类别实际选中的文件（只打印工作区相对路径，不打印项目根目录）与扫描统计，
//! 这些数字就是 `docs/M3_ACCEPTANCE_REPORT.md` §3 那张表的来源。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rove_base::{ByteOffset, Cancellation, Events, Task, Value};
use rove_buffer::edit::{EditOptions, Selection, Selections};
use rove_buffer::save::{
    self, DiskChange, DiskStamp, SaveOptions, SaveOutcome, SaveRequest, SelfWrites, TextFile,
    UnencodableChoice, WriteOptions,
};
use rove_buffer::Encoding;
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings};
use rove_test_support::UnityProject;

// --------------------------------------------------------------------------------- 设置

struct TestSettings(DefaultSettings);

impl TestSettings {
    fn new() -> Self {
        let mut registry = SettingRegistry::default();
        rove_buffer::settings::register(&mut registry).unwrap();
        save::settings::register(&mut registry).unwrap();
        Self(DefaultSettings(registry))
    }
}

impl Settings for TestSettings {
    fn read(&self, key: &str, scope: &Scope) -> rove_base::Result<Value> {
        self.0.read(key, scope)
    }
    fn write(&self, _: Arc<str>, _: Scope, _: Value, _: Cancellation) -> Task<()> {
        Box::pin(async { Err(rove_base::Error::Unsupported) })
    }
    fn changes(&self) -> &dyn Events<rove_config::SettingsChange> {
        &rove_base::NoEvents
    }
}

// --------------------------------------------------------------------------------- 扫描真实项目

/// 往返矩阵的类别。顺序就是报告里表格的顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    /// UTF-8 无 BOM、全 LF。
    Utf8LF,
    /// UTF-8 无 BOM、全 CRLF（Unity 项目里最常见的一类）。
    Utf8Crlf,
    /// UTF-8 带 BOM（Visual Studio 新建的 `.cs` 默认就是这个）。
    Utf8Bom,
    /// UTF-16 LE / BE（带 BOM）。
    Utf16,
    /// 不是合法 UTF-8：GBK 等传统中文编码。
    NonUtf8,
    /// 同一个文件里不止一种行终止符。
    MixedEol,
    /// 末尾没有换行。
    NoFinalNewline,
    /// 空文件（0 字节）。
    Empty,
    /// 只有一行、但很大（≥ 64 KiB，典型的是压成一行的 `.json` / `.asset`）。
    OneBigLine,
}

impl Class {
    const ALL: [Class; 9] = [
        Class::Utf8LF,
        Class::Utf8Crlf,
        Class::Utf8Bom,
        Class::Utf16,
        Class::NonUtf8,
        Class::MixedEol,
        Class::NoFinalNewline,
        Class::Empty,
        Class::OneBigLine,
    ];

    fn label(self) -> &'static str {
        match self {
            Class::Utf8LF => "UTF-8 无 BOM + LF",
            Class::Utf8Crlf => "UTF-8 无 BOM + CRLF",
            Class::Utf8Bom => "UTF-8 带 BOM",
            Class::Utf16 => "UTF-16 LE / BE",
            Class::NonUtf8 => "非 UTF-8（GBK 等）",
            Class::MixedEol => "混合换行符",
            Class::NoFinalNewline => "末尾无换行",
            Class::Empty => "空文件",
            Class::OneBigLine => "只有一行的大文件",
        }
    }
}

/// 扫描结果：每个类别选中的样本（工作区相对路径 + 原始字节）与命中计数。
struct Survey {
    picked: BTreeMap<Class, (String, Vec<u8>)>,
    counts: BTreeMap<Class, usize>,
    scanned: usize,
}

/// 单个文件可能同时落进多个类别（例如 UTF-8 CRLF 且末尾无换行）。
fn classify(bytes: &[u8]) -> Vec<Class> {
    let mut out = Vec::new();
    if bytes.is_empty() {
        out.push(Class::Empty);
        return out;
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        out.push(Class::Utf16);
        return out;
    }
    let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let body = if bom { &bytes[3..] } else { bytes };
    if std::str::from_utf8(body).is_err() {
        out.push(Class::NonUtf8);
        return out;
    }
    // 行终止符：CRLF / 单独的 LF / 单独的 CR。
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < body.len() {
        match body[i] {
            b'\r' if body.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 2;
                continue;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds = usize::from(crlf > 0) + usize::from(lf > 0) + usize::from(cr > 0);
    if kinds > 1 {
        out.push(Class::MixedEol);
    }
    if bom {
        out.push(Class::Utf8Bom);
    } else if kinds == 1 && crlf > 0 {
        out.push(Class::Utf8Crlf);
    } else if kinds == 1 && lf > 0 {
        out.push(Class::Utf8LF);
    }
    if !body.ends_with(b"\n") {
        out.push(Class::NoFinalNewline);
    }
    if kinds == 0 && body.len() >= 64 * 1024 {
        out.push(Class::OneBigLine);
    }
    out
}

/// Unity 生成目录与二进制目录：扫描时跳过（`Library` 里有几十万个文件）。
fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        "Library" | "Temp" | "obj" | "Logs" | "Build" | "Builds" | ".git" | ".vs" | "node_modules"
    )
}

/// 只看文本类扩展名；二进制（`.png` / `.dll`）没有往返的意义。
fn text_like(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "cs" | "txt"
            | "json"
            | "shader"
            | "cginc"
            | "hlsl"
            | "xml"
            | "md"
            | "asmdef"
            | "uss"
            | "uxml"
            | "yaml"
            | "yml"
    )
}

/// 深度优先扫描，最多看 `budget` 个文件、每个文件最多读 `MAX_BYTES`。
fn survey(root: &Path, budget: usize) -> Survey {
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let mut picked: BTreeMap<Class, (String, Vec<u8>)> = BTreeMap::new();
    let mut counts: BTreeMap<Class, usize> = BTreeMap::new();
    let mut scanned = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if scanned >= budget {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if scanned >= budget {
                break;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let name = entry.file_name();
                if !skip_dir(&name.to_string_lossy()) {
                    stack.push(path);
                }
                continue;
            }
            if !kind.is_file() || !text_like(&path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() > MAX_BYTES {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            scanned += 1;
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            for class in classify(&bytes) {
                *counts.entry(class).or_default() += 1;
                // 每类留第一个足够小的样本（往返要重新编码整篇，别挑 4 MB 的）。
                if !picked.contains_key(&class) && bytes.len() <= 512 * 1024 {
                    picked.insert(class, (rel.clone(), bytes.clone()));
                }
            }
        }
    }
    Survey {
        picked,
        counts,
        scanned,
    }
}

// --------------------------------------------------------------------------------- 往返

fn load(path: &Path, settings: &dyn Settings) -> TextFile {
    let language = match path.extension().and_then(|e| e.to_str()) {
        Some("cs") => "csharp",
        Some("json") | Some("asmdef") => "json",
        Some("md") => "markdown",
        _ => "plaintext",
    };
    TextFile::load(
        path,
        language,
        settings,
        Arc::from([]),
        EditOptions::default(),
    )
    .unwrap()
}

fn encode_as(file: &TextFile, text: &str) -> Vec<u8> {
    let rope = rove_buffer::RopeText::from_text(text);
    save::encode(
        &rope,
        file.encoding(),
        file.bom(),
        save::Unencodable::Reject,
    )
    .unwrap()
}

fn type_at(file: &mut TextFile, at: u64, text: &str, now: Instant) {
    let session = file.session_mut();
    session.set_selections(Selections::single(Selection::cursor(ByteOffset(at))));
    session.type_text(text, now).unwrap();
}

fn save_default(file: &mut TextFile) -> SaveOutcome {
    file.save(
        &SaveRequest::default(),
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    )
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take(a.len().min(b.len()))
        .take_while(|(x, y)| x == y)
        .count()
}

/// 插入点：中间那一行的行首（不是文件头，这样前缀和后缀都要逐字节相同才算过）。
fn middle_line_start(text: &str) -> u64 {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .filter(|&i| i < text.len())
        .collect();
    starts.get(starts.len() / 2).copied().unwrap_or(0) as u64
}

/// 临时目录里没有 `.tmp` 残留（原子写入的临时文件应当全部清掉）。
fn tmp_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".tmp"))
                .collect()
        })
        .unwrap_or_default()
}

/// 一个样本的往返：副本 → 打开 → 中间行插入 `MARK` → 保存 → 与原字节比较。
///
/// 返回「保存后的字节数 − 原字节数」，报告里用来核对「差异恰好是那一处改动」。
fn round_trip(dir: &Path, name: &str, original: &[u8], settings: &dyn Settings) -> i64 {
    const MARK: &str = "RoveM3";
    let path = dir.join(name);
    std::fs::write(&path, original).unwrap();
    let mut file = load(&path, settings);

    // 没有改动时保存什么也不做：磁盘字节一个都不能变。
    assert_eq!(save_default(&mut file), SaveOutcome::NotDirty, "{name}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "{name}: 未改动却写了盘"
    );

    let before = file.session().contents();
    let at = middle_line_start(&before);
    type_at(&mut file, at, MARK, Instant::now());
    assert!(file.is_dirty(), "{name}");
    assert!(
        matches!(save_default(&mut file), SaveOutcome::Saved { .. }),
        "{name}: 保存失败"
    );
    assert!(!file.is_dirty(), "{name}");

    let saved = std::fs::read(&path).unwrap();
    let mut expected_text = String::with_capacity(before.len() + MARK.len());
    expected_text.push_str(&before[..at as usize]);
    expected_text.push_str(MARK);
    expected_text.push_str(&before[at as usize..]);
    assert_eq!(
        saved,
        encode_as(&file, &expected_text),
        "{name}: 字节不一致"
    );

    // 差异恰好是那一处改动：公共前缀 + 公共后缀覆盖了其余全部字节。
    let inserted = encode_as(&file, MARK).len() - encode_as(&file, "").len();
    let prefix = common_prefix(original, &saved);
    let suffix = common_suffix(original, &saved);
    assert!(
        prefix + suffix >= original.len(),
        "{name}: 改动处之外也变了（前缀 {prefix} 后缀 {suffix} 原长 {}）",
        original.len()
    );
    assert!(tmp_files(dir).is_empty(), "{name}: 留下了临时文件");
    assert_eq!(
        saved.len() as i64 - original.len() as i64,
        inserted as i64,
        "{name}: 长度变化不等于插入的字节数"
    );
    saved.len() as i64 - original.len() as i64
}

// --------------------------------------------------------------------------------- 测试

/// 目标 2 的主体：两个真实项目里逐类别取真实文件做字节级往返。
///
/// 项目里没有的类别（M1 验收已经发现两个项目的源码**全是 UTF-8**）由 `构造补齐` 一组覆盖，
/// 并在 stderr 上写明「项目里 0 个」，报告照抄这个数字。
#[test]
fn m3_real_project_encoding_round_trip_is_byte_identical() {
    let settings = TestSettings::new();
    let dir = tempfile::tempdir().unwrap();
    let mut any = false;
    for project in UnityProject::ALL {
        let Some(root) = project.path() else { continue };
        any = true;
        let survey = survey(&root, 6000);
        eprintln!(
            "[M3 往返] {} 项目：扫描 {} 个文本文件",
            project.label(),
            survey.scanned
        );
        for class in Class::ALL {
            let hits = survey.counts.get(&class).copied().unwrap_or(0);
            match survey.picked.get(&class) {
                Some((rel, bytes)) => {
                    let name = format!(
                        "{:?}_{}_{}",
                        class,
                        project.label(),
                        rel.rsplit('/').next().unwrap_or("f")
                    );
                    let delta = round_trip(dir.path(), &name, bytes, &settings);
                    eprintln!(
                        "[M3 往返] {:<22} 命中 {:>5} 个｜样本 {} （{} B）｜往返后长度 {:+} B｜零差异",
                        class.label(),
                        hits,
                        rel,
                        bytes.len(),
                        delta
                    );
                }
                None => eprintln!(
                    "[M3 往返] {:<22} 命中 {:>5} 个｜项目里没有，由构造文件补齐（见 constructed 组）",
                    class.label(),
                    hits
                ),
            }
        }
    }
    if !any {
        eprintln!("已跳过 m3_real_project_encoding_round_trip：两个 Unity 项目都没有配置");
    }
}

/// 真实项目里没有的编码：拿**真实文件的内容**换一种编码写出来再往返。
///
/// 内容是真的（带中文注释的 `.cs`），编码是构造的；这样既覆盖了 GBK / UTF-16 / 混合换行，
/// 又没有把假数据当成真实样本。找不到带中文的真实文件时退回一段固定文本，并在日志里说明。
#[test]
fn m3_real_content_in_gbk_utf16_and_mixed_eol_round_trips() {
    let settings = TestSettings::new();
    let dir = tempfile::tempdir().unwrap();
    let (source, text) = chinese_source();
    eprintln!(
        "[M3 往返] 构造组的内容来源：{source}（{} 字符）",
        text.chars().count()
    );

    // GBK + CRLF、UTF-16 LE、UTF-16 BE：真实内容 + 构造编码。
    let crlf = to_crlf(&text);
    for (name, encoding, bom, body) in [
        ("constructed_gbk_crlf.cs", Encoding::Gbk, false, &crlf),
        ("constructed_utf16le.cs", Encoding::Utf16Le, true, &crlf),
        ("constructed_utf16be.cs", Encoding::Utf16Be, true, &text),
    ] {
        let rope = rove_buffer::RopeText::from_text(body);
        let bytes = save::encode(&rope, &encoding, bom, save::Unencodable::Reject).unwrap();
        let delta = round_trip(dir.path(), name, &bytes, &settings);
        eprintln!(
            "[M3 往返] {name:<26}｜{} B｜往返后长度 {delta:+} B｜零差异",
            bytes.len()
        );
    }

    // 混合换行符：真实内容，逐行轮换 LF / CRLF / CR。保存后仍然混合、未被顺手归一。
    let mixed = mix_eol(&text);
    let bytes = mixed.as_bytes().to_vec();
    let delta = round_trip(dir.path(), "constructed_mixed_eol.cs", &bytes, &settings);
    eprintln!(
        "[M3 往返] constructed_mixed_eol.cs ｜{} B｜往返后长度 {delta:+} B｜零差异",
        bytes.len()
    );

    // 末尾无换行 / 空文件 / 单行大文件的构造版本（真实项目里有没有都测一遍）。
    for (name, bytes) in [
        (
            "constructed_no_final_newline.cs",
            text.trim_end().as_bytes().to_vec(),
        ),
        ("constructed_empty.cs", Vec::new()),
        (
            "constructed_one_big_line.json",
            one_big_line(&text).into_bytes(),
        ),
    ] {
        let delta = round_trip(dir.path(), name, &bytes, &settings);
        eprintln!(
            "[M3 往返] {name:<32}｜{} B｜往返后长度 {delta:+} B｜零差异",
            bytes.len()
        );
    }
}

/// 真实项目里找一个带中文注释的 `.cs`，内容原样返回（UTF-8）。
fn chinese_source() -> (String, String) {
    for project in UnityProject::ALL {
        let Some(root) = project.path() else { continue };
        let mut stack = vec![root.clone()];
        let mut scanned = 0usize;
        while let Some(dir) = stack.pop() {
            if scanned >= 4000 {
                break;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if !skip_dir(&entry.file_name().to_string_lossy()) {
                        stack.push(path);
                    }
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("cs") {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                if meta.len() > 128 * 1024 || meta.len() < 200 {
                    continue;
                }
                scanned += 1;
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(text) = String::from_utf8(bytes) else {
                    continue;
                };
                // 要有中文（GBK 能编码）、不能有 emoji 之类 GBK 编不出的字符。
                let has_cjk = text.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
                let encodable = !text.chars().any(|c| c as u32 > 0xFFFF);
                if has_cjk && encodable {
                    let rel = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    return (
                        format!("{} 项目的 {}", project.label(), rel),
                        normalize_lf(&text),
                    );
                }
            }
        }
    }
    eprintln!(
        "已跳过「真实中文源码」：两个项目里都没找到带中文注释且 GBK 可编码的 .cs，改用固定文本"
    );
    (
        "固定文本（项目里没找到合适的真实文件）".into(),
        "// 中文注释：玩家控制器\nclass 玩家 {\n    int 生命值 = 100;\n}\n".into(),
    )
}

fn normalize_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn to_crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

/// 逐行轮换 LF / CRLF / CR。
fn mix_eol(text: &str) -> String {
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        out.push_str(line);
        out.push_str(match i % 3 {
            0 => "\n",
            1 => "\r\n",
            _ => "\r",
        });
    }
    out
}

/// 把真实内容压成一行（≥ 64 KiB），模拟 Unity 里压扁的 `.json` / `.asset`。
fn one_big_line(text: &str) -> String {
    let one = text.replace(['\n', '\r'], " ");
    let repeat = (64 * 1024 / one.len().max(1)) + 2;
    one.repeat(repeat)
}

/// 目标 2 最后一条：GBK 文件里输入 emoji，三个选项各走一遍。
///
/// 内容仍然来自真实文件（转成 GBK）；判据是 `UX_SPEC` §3.5 的三条分支。
#[test]
fn m3_gbk_file_with_emoji_walks_all_three_choices() {
    let settings = TestSettings::new();
    let dir = tempfile::tempdir().unwrap();
    let (_, text) = chinese_source();
    let crlf = to_crlf(&text);
    let rope = rove_buffer::RopeText::from_text(&crlf);
    let original = save::encode(&rope, &Encoding::Gbk, false, save::Unencodable::Reject).unwrap();

    // 分支 1「取消」：弹模态框，磁盘一个字节都没动。
    let path = dir.path().join("emoji_ask.cs");
    std::fs::write(&path, &original).unwrap();
    let mut file = load(&path, &settings);
    let at = middle_line_start(&file.session().contents());
    type_at(&mut file, at, "😀", Instant::now());
    let outcome = save_default(&mut file);
    assert!(
        matches!(&outcome, SaveOutcome::NeedsEncodingDecision { count: 1, encoding } if *encoding == Encoding::Gbk),
        "取消分支：期望弹模态框，实际 {outcome:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original, "取消分支不该写盘");
    assert!(file.is_dirty(), "取消分支应保留未保存修改");

    // 分支 2「仍然保存（替换为 ?）」：仍是 GBK，emoji 变成 ?。
    let saved = file.save(
        &SaveRequest {
            unencodable: UnencodableChoice::Replace,
            ..SaveRequest::default()
        },
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    );
    assert!(
        matches!(
            saved,
            SaveOutcome::Saved {
                encoding_changed: false,
                ..
            }
        ),
        "{saved:?}"
    );
    assert_eq!(file.encoding(), &Encoding::Gbk);
    // 重新打开（仍按 GBK 识别）：emoji 已经变成 `?`。
    let decoded = load(&path, &settings).session().contents();
    assert!(decoded.contains('?'), "替换分支：期望出现 ?");
    assert!(!decoded.contains('😀'), "替换分支：GBK 里不可能有 emoji");

    // 分支 3「改用 UTF-8 保存」：编码变成 UTF-8，emoji 原样保留。
    let path = dir.path().join("emoji_utf8.cs");
    std::fs::write(&path, &original).unwrap();
    let mut file = load(&path, &settings);
    let at = middle_line_start(&file.session().contents());
    type_at(&mut file, at, "😀", Instant::now());
    let saved = file.save(
        &SaveRequest {
            encoding: Some((Encoding::Utf8, false)),
            ..SaveRequest::default()
        },
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    );
    assert!(
        matches!(
            saved,
            SaveOutcome::Saved {
                encoding_changed: true,
                ..
            }
        ),
        "{saved:?}"
    );
    assert_eq!(file.encoding(), &Encoding::Utf8);
    assert!(String::from_utf8(std::fs::read(&path).unwrap())
        .unwrap()
        .contains('😀'));
}

// --------------------------------------------------------------------------------- 外部修改与保存冲突

fn disk(path: &Path) -> Option<(DiskStamp, Vec<u8>)> {
    DiskStamp::read(path).unwrap()
}

/// 真实文件的副本上：外部改写 100 次 + 自己连续保存 20 次。
///
/// 判据（issue 目标 3 的前两条）：最终内容与磁盘一致；「需要决定」只报告一次 `first = true`
/// （提示条不堆积）；自己的 20 次写入一次都没被当成外部修改。
#[test]
fn m3_real_file_external_storm_and_consecutive_saves() {
    let settings = TestSettings::new();
    let (_, text) = chinese_source();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Storm.cs");
    std::fs::write(&path, text.as_bytes()).unwrap();

    // ---- 外部修改风暴：有本地修改时连续 100 次外部改写。
    let mut file = load(&path, &settings);
    type_at(&mut file, 0, "// local\n", Instant::now());
    let mut firsts = 0usize;
    for i in 0..100 {
        std::fs::write(&path, format!("// external {i}\n{text}")).unwrap();
        match file
            .disk_changed(disk(&path), &settings, Instant::now())
            .unwrap()
        {
            DiskChange::NeedsDecision { first } => firsts += usize::from(first),
            other => panic!("第 {i} 次外部改写期望 NeedsDecision，实际 {other:?}"),
        }
    }
    assert_eq!(firsts, 1, "100 次外部改写只应弹一次提示条");
    // 「重新加载」：最终内容与磁盘完全一致。
    file.revert(&settings, Instant::now()).unwrap();
    assert_eq!(
        file.session().contents(),
        normalize_lf(&format!("// external 99\n{text}")),
        "重新加载后与磁盘不一致"
    );
    assert!(!file.is_dirty());
    assert!(!file.has_pending_external_change());

    // ---- 连续保存 20 次：自己的写入一次都不能被当成外部修改。
    let mut self_writes = SelfWrites::new(Duration::from_secs(5));
    let mut false_reports = 0usize;
    for i in 0..20 {
        type_at(&mut file, 0, &format!("// save {i}\n"), Instant::now());
        let SaveOutcome::Saved { hash, .. } = save_default(&mut file) else {
            panic!("第 {i} 次保存失败");
        };
        self_writes.record(&path, hash, Instant::now());
        // 文件监听把自己的写入报上来：去重器必须认出来。
        if !self_writes.is_own(&path, Some(hash), Instant::now()) {
            false_reports += 1;
        }
        // 即使漏过去重器，TextFile 也认得这份内容（磁盘标记相同）。
        match file
            .disk_changed(disk(&path), &settings, Instant::now())
            .unwrap()
        {
            DiskChange::Unchanged => {}
            other => panic!("第 {i} 次自己保存被当成了外部修改：{other:?}"),
        }
    }
    assert_eq!(false_reports, 0, "自写去重漏报 {false_reports} 次");
    eprintln!("[M3 外部修改] 100 次外部改写只弹 1 次提示条；20 次连续保存 0 次误报");
}

/// 目标 6 的两条保存耗时：1 MB 与 10 MB 文件各保存 5 次，报告中位数。
///
/// 只**打印**不断言：本机同时开着 Unity 等程序，负载较高，绝对值只作相对参考
/// （AGENTS.md §8.1：性能判定走 `cargo xtask bench` 与 `perf/budgets.toml`，这里不设门槛）。
#[test]
fn m3_save_timing_for_one_and_ten_megabytes() {
    let settings = TestSettings::new();
    let dir = tempfile::tempdir().unwrap();
    let (_, sample) = chinese_source();
    for (label, target) in [("1 MB", 1 << 20), ("10 MB", 10 << 20)] {
        let mut text = String::with_capacity(target + sample.len());
        while text.len() < target {
            text.push_str(&sample);
        }
        let path = dir.path().join(format!("Big{}.cs", text.len()));
        std::fs::write(&path, text.as_bytes()).unwrap();
        let mut file = load(&path, &settings);
        let mut samples = Vec::new();
        for i in 0..5 {
            type_at(&mut file, 0, &format!("// {i}\n"), Instant::now());
            let started = Instant::now();
            let outcome = save_default(&mut file);
            samples.push(started.elapsed());
            assert!(matches!(outcome, SaveOutcome::Saved { .. }), "{outcome:?}");
        }
        samples.sort();
        eprintln!(
            "[M3 保存耗时] {label}（实际 {} B）：5 次保存（变换 + 编码 + 原子写入）中位数 {:?}，最大 {:?}",
            text.len(),
            samples[samples.len() / 2],
            samples.last().unwrap()
        );
    }
}

/// 真实文件副本上的四条失败路径：保存冲突、只读属性、被占用、被删除。
///
/// 「被占用」用**同进程持有独占句柄**模拟 Unity 导入期间的占用（真实 Unity 导入见报告的人工清单）。
#[test]
fn m3_real_file_failure_paths() {
    let settings = TestSettings::new();
    let (_, text) = chinese_source();
    let dir = tempfile::tempdir().unwrap();

    // ---- 保存冲突：外部改了磁盘，本地也改了 → Conflict；「覆盖」后写入成功。
    let path = dir.path().join("Conflict.cs");
    std::fs::write(&path, text.as_bytes()).unwrap();
    let mut file = load(&path, &settings);
    type_at(&mut file, 0, "// mine\n", Instant::now());
    std::fs::write(&path, format!("// theirs\n{text}")).unwrap();
    assert_eq!(save_default(&mut file), SaveOutcome::Conflict);
    assert!(file.is_dirty(), "冲突后本地修改必须保留");
    let outcome = file.save(
        &SaveRequest {
            overwrite_conflict: true,
            ..SaveRequest::default()
        },
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    );
    assert!(matches!(outcome, SaveOutcome::Saved { .. }), "{outcome:?}");

    // ---- 只读属性：报告 ReadOnly（没有提权入口），「覆盖」清除属性后写入。
    let path = dir.path().join("ReadOnly.cs");
    std::fs::write(&path, text.as_bytes()).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms).unwrap();
    let mut file = load(&path, &settings);
    type_at(&mut file, 0, "// mine\n", Instant::now());
    assert_eq!(save_default(&mut file), SaveOutcome::ReadOnly);
    let outcome = file.save(
        &SaveRequest {
            clear_read_only: true,
            ..SaveRequest::default()
        },
        &SaveOptions::default(),
        None,
        &WriteOptions::default(),
        Instant::now(),
    );
    assert!(matches!(outcome, SaveOutcome::Saved { .. }), "{outcome:?}");

    // ---- 被占用（模拟 Unity 导入）：自动重试一次后报 Busy；释放后重试成功。
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let path = dir.path().join("Busy.cs");
        std::fs::write(&path, text.as_bytes()).unwrap();
        let mut file = load(&path, &settings);
        type_at(&mut file, 0, "// mine\n", Instant::now());
        let hold = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let write = WriteOptions {
            busy_retry_delay: Duration::from_millis(50),
            ..WriteOptions::default()
        };
        let outcome = file.save(
            &SaveRequest::default(),
            &SaveOptions::default(),
            None,
            &write,
            Instant::now(),
        );
        assert_eq!(
            outcome,
            SaveOutcome::Busy { attempts: 2 },
            "占用时应报 Busy"
        );
        assert!(file.is_dirty());
        drop(hold);
        let outcome = file.save(
            &SaveRequest::default(),
            &SaveOptions::default(),
            None,
            &write,
            Instant::now(),
        );
        assert!(
            matches!(outcome, SaveOutcome::Saved { .. }),
            "释放后应保存成功：{outcome:?}"
        );
    }

    // ---- 被删除：标记为已删除、内容保留，下一次保存等同新建。
    let path = dir.path().join("Deleted.cs");
    std::fs::write(&path, text.as_bytes()).unwrap();
    let mut file = load(&path, &settings);
    type_at(&mut file, 0, "// mine\n", Instant::now());
    std::fs::remove_file(&path).unwrap();
    match file.disk_changed(None, &settings, Instant::now()).unwrap() {
        DiskChange::Deleted { first: true } => {}
        other => panic!("期望 Deleted{{first:true}}，实际 {other:?}"),
    }
    // 再报一次（风暴）不应再次弹提示。
    match file.disk_changed(None, &settings, Instant::now()).unwrap() {
        DiskChange::Deleted { first: false } => {}
        other => panic!("重复报告删除时 first 应为 false，实际 {other:?}"),
    }
    assert!(file.is_deleted());
    assert!(file.session().contents().starts_with("// mine"));
    assert!(matches!(save_default(&mut file), SaveOutcome::Saved { .. }));
    assert!(path.exists(), "保存应把文件重新建出来");
    assert!(!file.is_deleted());
    eprintln!("[M3 失败路径] 冲突 / 只读属性 / 占用 / 删除四条路径逐条通过，无提权入口");
}
