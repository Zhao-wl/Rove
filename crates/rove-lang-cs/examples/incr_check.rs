//! #35 增量索引的一致性检查与性能测量（只在本地跑，需要真实 Unity 项目与构建好的索引进程）。
//!
//! ```text
//! cargo run --release -p rove-lang-cs --example incr_check -- --project large --edits 1000 --seed 42
//! cargo run --release -p rove-lang-cs --example incr_check -- --project large --latency 50 --edits 0
//! ```
//!
//! - 项目：`--project medium|large`（读 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE`，未设置时跳过）。
//!   **不会改动原项目**：先把 csproj 列出的源文件、项目内的 DLL 与 asmdef/asmref 复制到 `--scratch`
//!   （默认 `target/incr-check/<项目>`），之后所有编辑都在副本上进行。
//! - 一致性（`--edits N`，默认 1000）：固定随机种子（`--seed`，不给时随机并打印）生成 N 次编辑——插入行、
//!   删除语句、改签名（改名 / 改可见性 / 改参数类型）、新增文件、删除文件、重命名文件——逐次通知常驻进程；
//!   每 `--checkpoint K` 次（默认 100）等后台队列清空，再对副本做一次全量索引，逐文件比较
//!   “基础索引 + 覆盖层”与全量结果：每个出现位置的（行, 列, 长度, 角色, 符号键, 种类）、不生效区间、
//!   文件哈希，以及全部关系。不一致时从上一个通过的检查点二分重放，定位到第一次出错的编辑。
//! - 延迟（`--latency N`）：在副本上做 N 次“普通保存”（随机文件里插一行注释），记录索引进程的重绑耗时、
//!   往返耗时，以及读覆盖层、合并、在该文件上查询到新结果的端到端耗时，给出 p50 / p99；另报冷启动
//!   （第一次重绑要预热项目）、常驻进程预热后与空闲修剪后的内存，以及全量索引的峰值内存。
//! - 结果写 `target/incr-check/report-<项目>.json`。
use rove_base::Utf8Column;
use rove_lang_cs::overlay::{read_journal, L2View, Overlay};
use rove_lang_cs::CsIndex;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn dll() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/rove-indexer-cs/bin/Release/net9.0/rove-indexer-cs.dll")
}

// ───────────── 项目副本 ─────────────

fn attr_values<'a>(text: &'a str, start: &str, end: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(start) {
        rest = &rest[i + start.len()..];
        if let Some(j) = rest.find(end) {
            out.push(&rest[..j]);
            rest = &rest[j..];
        }
    }
    out
}

/// 复制 csproj、它们列出的项目内文件（源码、DLL、分析器）以及 Assets/Packages 下的 asmdef / asmref。
fn copy_project(src: &Path, dst: &Path) -> std::io::Result<usize> {
    let mut files = BTreeSet::new();
    for entry in fs::read_dir(src)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "csproj") {
            let text = fs::read_to_string(&path)?;
            files.insert(PathBuf::from(path.file_name().unwrap()));
            let mut listed: Vec<&str> = attr_values(&text, "Include=\"", "\"");
            listed.extend(attr_values(&text, "<HintPath>", "</HintPath>"));
            for item in listed {
                let rel = PathBuf::from(item.replace('\\', "/"));
                if rel.is_relative() && !item.starts_with("..") {
                    files.insert(rel);
                }
            }
        }
    }
    fn walk(dir: &Path, root: &Path, out: &mut BTreeSet<PathBuf>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else if p
                .extension()
                .is_some_and(|x| x == "asmdef" || x == "asmref")
            {
                out.insert(p.strip_prefix(root).unwrap().to_owned());
            }
        }
    }
    for d in ["Assets", "Packages"] {
        walk(&src.join(d), src, &mut files);
    }
    let mut copied = 0;
    for rel in files {
        let from = src.join(&rel);
        if !from.is_file() {
            continue;
        }
        let to = dst.join(&rel);
        fs::create_dir_all(to.parent().unwrap())?;
        fs::copy(&from, &to)?;
        copied += 1;
    }
    Ok(copied)
}

/// 副本里可编辑的源文件（Assets 下、csproj 列出的）。
fn editable_sources(root: &Path) -> Vec<String> {
    let mut out = BTreeSet::new();
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "csproj") {
            let text = fs::read_to_string(&path).unwrap_or_default();
            for item in attr_values(&text, "Compile Include=\"", "\"") {
                let rel = item.replace('\\', "/");
                if rel.starts_with("Assets/") && rel.ends_with(".cs") && root.join(&rel).is_file() {
                    out.insert(rel);
                }
            }
        }
    }
    out.into_iter().collect()
}

// ───────────── 常驻进程与全量索引 ─────────────

struct Resident {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
    next: u64,
}

impl Resident {
    fn start(root: &Path, base: &Path, journal: &Path, duty: f64) -> Self {
        let mut child = Command::new("dotnet")
            .arg(dll())
            .arg("--serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start indexer");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let log = root.parent().unwrap().join("resident.log");
        std::thread::spawn(move || {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .unwrap();
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = writeln!(file, "{line}");
            }
        });
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(v);
                }
            }
        });
        let mut r = Self {
            child,
            stdin,
            rx,
            next: 1,
        };
        r.recv(|m| m["method"] == "hello");
        r.call(
            "initialize",
            json!({ "protocol": 1, "project": root, "base": base, "journal": journal,
                    "background_duty": duty, "trim_seconds": 3600, "idle_exit_seconds": 86400 }),
        );
        r
    }

    fn recv(&mut self, f: impl Fn(&Value) -> bool) -> Value {
        loop {
            let msg = self
                .rx
                .recv_timeout(Duration::from_secs(3600))
                .expect("indexer message");
            if f(&msg) {
                return msg;
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        )
        .unwrap();
        self.stdin.flush().unwrap();
        let msg = self.recv(|m| m["id"] == id);
        assert!(msg.get("error").is_none(), "{method}: {msg}");
        msg["result"].clone()
    }

    fn changed(&mut self, paths: &[String]) -> Value {
        let changes: Vec<Value> = paths.iter().map(|p| json!({ "path": p })).collect();
        self.call("changed", json!({ "changes": changes }))
    }

    fn stop(mut self) {
        let _ = self.call("shutdown", json!({}));
        let _ = self.child.wait();
    }
}

/// 一次性全量索引（与 #36 的构建同一入口），返回报告里的峰值工作集（MB）与耗时。
fn full_index(root: &Path, out: &Path) -> (f64, Duration) {
    let started = Instant::now();
    let output = Command::new("dotnet")
        .arg(dll())
        .arg(root)
        .arg(out)
        .output()
        .expect("run indexer");
    if !output.status.success() {
        let err = out.with_extension("err.log");
        let _ = fs::write(&err, &output.stderr);
        panic!(
            "full index failed ({:?}), see {}",
            output.status,
            err.display()
        );
    }
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (
        report["peak_working_set_mb"].as_f64().unwrap_or(0.0),
        started.elapsed(),
    )
}

fn merged(base: &Path, journal: &Path) -> (CsIndex, Option<Overlay>) {
    let index = CsIndex::open(base).unwrap();
    let (segments, _) = read_journal(journal, &index.identity()).unwrap();
    let overlay = (!segments.is_empty()).then(|| Overlay::build(&index, &segments));
    (index, overlay)
}

// ───────────── 比较 ─────────────

type FileFacts = (u64, Vec<(u32, u32)>, Vec<(u32, u64, u32, u16, String, u8)>);
type Relations = BTreeSet<(String, String, u32)>;

fn facts(view: &L2View) -> (BTreeMap<String, FileFacts>, Relations) {
    let mut files = BTreeMap::new();
    for f in 0..view.file_count() {
        let Some(path) = view.file_path(f) else {
            continue;
        };
        let mut occs: Vec<_> = view
            .file_occurrences(f)
            .into_iter()
            .map(|o| {
                (
                    o.line,
                    o.col.0,
                    o.len,
                    o.role,
                    view.key_text(o.symbol).unwrap_or_default(),
                    view.kind(o.symbol).unwrap_or(0),
                )
            })
            .collect();
        occs.sort();
        let inactive = view
            .inactive_lines(f)
            .iter()
            .map(|r| (r.start, r.end))
            .collect();
        files.insert(
            path.to_owned(),
            (view.file_hash(f).unwrap_or(0), inactive, occs),
        );
    }
    let mut relations = BTreeSet::new();
    for s in 0..view.symbol_count() {
        for r in view.bases(s) {
            if let (Some(a), Some(b)) = (view.key_text(r.from), view.key_text(r.to)) {
                relations.insert((a, b, r.kind));
            }
        }
    }
    (files, relations)
}

/// 返回不一致的描述（空 = 一致）。
fn compare(reference: &L2View, incremental: &L2View) -> Vec<String> {
    let (rf, rr) = facts(reference);
    let (inf, ir) = facts(incremental);
    let mut diffs = Vec::new();
    for path in rf.keys().chain(inf.keys()).collect::<BTreeSet<_>>() {
        match (rf.get(path), inf.get(path)) {
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => {
                let what = if a.0 != b.0 {
                    format!("hash {:x} vs {:x}", a.0, b.0)
                } else if a.1 != b.1 {
                    format!("inactive {:?} vs {:?}", a.1, b.1)
                } else {
                    let first = a
                        .2
                        .iter()
                        .zip(&b.2)
                        .find(|(x, y)| x != y)
                        .map(|(x, y)| format!("full {x:?} vs incremental {y:?}"))
                        .unwrap_or_else(|| format!("{} vs {} occurrences", a.2.len(), b.2.len()));
                    first
                };
                diffs.push(format!("{path}: {what}"));
            }
            (Some(_), None) => diffs.push(format!("{path}: missing in incremental")),
            (None, Some(_)) => diffs.push(format!("{path}: only in incremental")),
            (None, None) => {}
        }
    }
    for r in rr.symmetric_difference(&ir).take(20) {
        let side = if rr.contains(r) {
            "full only"
        } else {
            "incremental only"
        };
        diffs.push(format!("relation {side}: {r:?}"));
    }
    diffs
}

// ───────────── 编辑 ─────────────

#[derive(Clone, Debug)]
struct Edit {
    n: usize,
    kind: &'static str,
    /// 编辑前后的文件内容（None = 不存在）。
    before: Vec<(String, Option<Vec<u8>>)>,
    after: Vec<(String, Option<Vec<u8>>)>,
}

impl Edit {
    fn paths(&self) -> Vec<String> {
        self.after.iter().map(|(p, _)| p.clone()).collect()
    }
}

fn apply(root: &Path, files: &[(String, Option<Vec<u8>>)]) {
    for (path, content) in files {
        let full = root.join(path);
        match content {
            Some(bytes) => {
                fs::create_dir_all(full.parent().unwrap()).unwrap();
                fs::write(&full, bytes).unwrap();
            }
            None => {
                let _ = fs::remove_file(&full);
            }
        }
    }
}

fn is_method_line(l: &str) -> bool {
    let t = l.trim();
    (t.starts_with("public ")
        || t.starts_with("private ")
        || t.starts_with("protected ")
        || t.starts_with("internal "))
        && t.contains('(')
        && !t.ends_with(';')
        && !t.contains(" class ")
        && !t.contains(" struct ")
        && !t.contains(" interface ")
        && !t.contains(" delegate ")
        && !t.contains('=')
        && !t.contains(" new ")
}

/// 生成下一次编辑（不落盘）。`sources` 随新增 / 删除 / 重命名更新。
fn next_edit(
    rng: &mut Rng,
    root: &Path,
    n: usize,
    sources: &mut Vec<String>,
    created: &mut Vec<String>,
) -> Edit {
    let read = |p: &str| fs::read(root.join(p)).ok();
    loop {
        let roll = rng.below(100);
        let pick = sources[rng.below(sources.len())].clone();
        let Some(bytes) = read(&pick) else { continue };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
        let single = |kind, path: &str, after: String| Edit {
            n,
            kind,
            before: vec![(path.to_owned(), Some(bytes.clone()))],
            after: vec![(path.to_owned(), Some(after.into_bytes()))],
        };
        match roll {
            0..=34 => {
                let at = rng.below(lines.len() + 1);
                lines.insert(at, format!("// incr-check edit {n}\n"));
                return single("insert", &pick, lines.concat());
            }
            35..=59 => {
                let candidates: Vec<usize> = (0..lines.len())
                    .filter(|&i| {
                        let t = lines[i].trim();
                        t.ends_with(';')
                            && !t.contains('{')
                            && !t.contains('}')
                            && !t.starts_with("using ")
                    })
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                lines.remove(candidates[rng.below(candidates.len())]);
                return single("delete-line", &pick, lines.concat());
            }
            60..=79 => {
                let candidates: Vec<usize> = (0..lines.len())
                    .filter(|&i| is_method_line(&lines[i]))
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let i = candidates[rng.below(candidates.len())];
                let line = lines[i].clone();
                let changed = match rng.below(3) {
                    0 => {
                        let paren = line.find('(').unwrap();
                        let start = line[..paren]
                            .trim_end()
                            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .map_or(0, |p| p + 1);
                        let end = line[..paren].trim_end().len();
                        if start >= end {
                            continue;
                        }
                        format!(
                            "{}{}X{}{}",
                            &line[..start],
                            &line[start..end],
                            n,
                            &line[end..]
                        )
                    }
                    1 if line.contains("public ") => line.replacen("public ", "internal ", 1),
                    1 => line
                        .replacen("private ", "public ", 1)
                        .replacen("protected ", "public ", 1)
                        .replacen("internal ", "public ", 1),
                    _ if line.contains("(int ") => line.replacen("(int ", "(long ", 1),
                    _ if line.contains("(float ") => line.replacen("(float ", "(double ", 1),
                    _ => line
                        .replacen('(', "(int incrCheck, ", 1)
                        .replace(", )", ")"),
                };
                if changed == line {
                    continue;
                }
                lines[i] = changed;
                return single("signature", &pick, lines.concat());
            }
            80..=89 => {
                let path = format!("Assets/IncrCheck/Gen{n}.cs");
                let body = if rng.below(2) == 0 {
                    format!("namespace IncrCheck\n{{\n    public static class Gen{n}\n    {{\n        public static int Value() => {n};\n    }}\n}}\n")
                } else {
                    let other = created
                        .last()
                        .and_then(|p| p.rsplit('/').next())
                        .and_then(|f| f.strip_suffix(".cs"))
                        .map(str::to_owned);
                    let call = other.map_or("0".to_owned(), |o| {
                        format!("{}.Value()", o.trim_end_matches('b'))
                    });
                    format!("using System.Collections.Generic;\nnamespace IncrCheck\n{{\n    public class Gen{n}\n    {{\n        public List<int> Items = new List<int>();\n        public int Value() => Items.Count + {call};\n    }}\n}}\n")
                };
                created.push(path.clone());
                sources.push(path.clone());
                return Edit {
                    n,
                    kind: "new-file",
                    before: vec![(path.clone(), None)],
                    after: vec![(path, Some(body.into_bytes()))],
                };
            }
            90..=94 => {
                let Some(victim) =
                    (!created.is_empty()).then(|| created.remove(rng.below(created.len())))
                else {
                    continue;
                };
                sources.retain(|s| *s != victim);
                return Edit {
                    n,
                    kind: "delete-file",
                    before: vec![(victim.clone(), read(&victim))],
                    after: vec![(victim, None)],
                };
            }
            _ => {
                let Some(i) = (!created.is_empty()).then(|| rng.below(created.len())) else {
                    continue;
                };
                let from = created[i].clone();
                let to = from.replace(".cs", "b.cs");
                if to == from || sources.contains(&to) {
                    continue;
                }
                created[i] = to.clone();
                sources.retain(|s| *s != from);
                sources.push(to.clone());
                let content = read(&from);
                return Edit {
                    n,
                    kind: "rename-file",
                    before: vec![(from.clone(), content.clone()), (to.clone(), None)],
                    after: vec![(from, None), (to, content)],
                };
            }
        }
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let which = arg(&args, "--project").unwrap_or_else(|| "large".into());
    let var = match which.as_str() {
        "medium" => "ROVE_TEST_UNITY_MEDIUM",
        "large" => "ROVE_TEST_UNITY_LARGE",
        other => panic!("--project medium|large, got {other}"),
    };
    let Some(source) = std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
    else {
        eprintln!("已跳过：{var} 未设置或目录不存在");
        return;
    };
    if !dll().is_file() {
        eprintln!("已跳过：先 dotnet build tools/rove-indexer-cs -c Release");
        return;
    }
    let edits: usize = arg(&args, "--edits").map_or(1000, |v| v.parse().unwrap());
    let latency: usize = arg(&args, "--latency").map_or(0, |v| v.parse().unwrap());
    let every: usize = arg(&args, "--checkpoint").map_or(100, |v| v.parse().unwrap());
    let seed: u64 = arg(&args, "--seed").map_or_else(
        || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
                | 1
        },
        |v| v.parse().unwrap(),
    );
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/incr-check");
    let scratch = arg(&args, "--scratch").map_or_else(|| target.join(&which), PathBuf::from);
    println!("seed {seed}（用 --seed {seed} 复现）");

    // 1. 副本 + 基础索引
    let root = scratch.join("project");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&root).unwrap();
    let started = Instant::now();
    let copied = copy_project(&source, &root).unwrap();
    println!(
        "copied {copied} files in {:.1}s",
        started.elapsed().as_secs_f64()
    );
    let base = scratch.join("base.rvcsidx");
    let (peak_mb, full_time) = full_index(&root, &base);
    println!(
        "full index: {:.1}s, peak working set {peak_mb:.0} MB",
        full_time.as_secs_f64()
    );
    let journal = scratch.join("base.rvcsovl");
    let mut report = json!({ "project": which, "seed": seed, "full_index_s": full_time.as_secs_f64(), "full_index_peak_mb": peak_mb });
    let mut resident = Resident::start(&root, &base, &journal, 1.0);
    let mut sources = editable_sources(&root);
    let mut rng = Rng(seed);

    // 2. 延迟
    if latency > 0 {
        let mut pick = || loop {
            let p = sources[rng.below(sources.len())].clone();
            if fs::metadata(root.join(&p)).is_ok_and(|m| m.len() < 64 << 10) {
                return p;
            }
        };
        let touch = |p: &str, n: usize| {
            let full = root.join(p);
            let mut text = fs::read_to_string(&full).unwrap();
            let at = text.find('\n').map_or(0, |i| i + 1);
            text.insert_str(at, &format!("// latency {n}\n"));
            fs::write(full, text).unwrap();
        };
        let files: Vec<String> = (0..latency).map(|_| pick()).collect();
        // 第 1 轮：每个文件第一次保存（所属项目还没预热时含预热）
        let mut cold = Vec::new();
        for (n, p) in files.iter().enumerate() {
            touch(p, n);
            let r = resident.changed(std::slice::from_ref(p));
            cold.push(r["elapsed_ms"].as_f64().unwrap_or(0.0));
            resident.call("wait_idle", json!({}));
        }
        let warm_status = resident.call("status", json!({}));
        // 第 2 轮：同一批文件再各保存一次（项目已预热：日常编辑的情形）
        let (mut indexer, mut round, mut visible) = (Vec::new(), Vec::new(), Vec::new());
        for (n, p) in files.iter().enumerate() {
            touch(p, latency + n);
            let t = Instant::now();
            let r = resident.changed(std::slice::from_ref(p));
            let rt = t.elapsed();
            let (index, overlay) = merged(&base, &journal);
            let view = L2View::new(&index, overlay.as_ref());
            let file = view.file_id(p).expect("file in index");
            let occ = view.file_occurrences(file);
            if let Some(o) = occ.first() {
                let _ = view
                    .occurrence_at(file, o.line, Utf8Column(o.col.0))
                    .map(|o| view.occurrences(o.symbol));
            }
            let vis = t.elapsed();
            indexer.push(r["elapsed_ms"].as_f64().unwrap_or(0.0));
            round.push(rt.as_secs_f64() * 1000.0);
            visible.push(vis.as_secs_f64() * 1000.0);
            resident.call("wait_idle", json!({}));
        }
        for v in [&mut cold, &mut indexer, &mut round, &mut visible] {
            v.sort_by(f64::total_cmp);
        }
        println!(
            "first save per file (project warm-up included, ms): p50 {:.0} p99 {:.0} max {:.0}",
            percentile(&cold, 0.5),
            percentile(&cold, 0.99),
            cold.last().copied().unwrap_or(0.0)
        );
        println!(
            "single-file update over {latency} saves, warm (ms): indexer p50 {:.0} p99 {:.0}; round trip p50 {:.0} p99 {:.0}; visible to queries p50 {:.0} p99 {:.0}",
            percentile(&indexer, 0.5), percentile(&indexer, 0.99), percentile(&round, 0.5), percentile(&round, 0.99),
            percentile(&visible, 0.5), percentile(&visible, 0.99)
        );
        report["latency"] = json!({
            "samples": latency,
            "first_save_ms": { "p50": percentile(&cold, 0.5), "p99": percentile(&cold, 0.99), "max": cold.last() },
            "indexer_ms": { "p50": percentile(&indexer, 0.5), "p99": percentile(&indexer, 0.99), "max": indexer.last() },
            "round_trip_ms": { "p50": percentile(&round, 0.5), "p99": percentile(&round, 0.99) },
            "visible_ms": { "p50": percentile(&visible, 0.5), "p99": percentile(&visible, 0.99), "max": visible.last() },
            "resident_warm_mb": warm_status["working_set_mb"],
            "warm_projects": warm_status["warm_projects"].as_array().map_or(0, Vec::len),
        });
        // 空闲修剪后的内存：同一个进程，trim_seconds 到期后丢掉预热的编译
        resident.call("set_trim", json!({ "trim_seconds": 1 }));
        std::thread::sleep(Duration::from_secs(4));
        let idle_status = resident.call("status", json!({}));
        println!(
            "resident memory: warm {} MB ({} projects warm), idle after trim {} MB, full index peak {peak_mb:.0} MB",
            warm_status["working_set_mb"],
            report["latency"]["warm_projects"],
            idle_status["working_set_mb"]
        );
        report["latency"]["resident_idle_mb"] = idle_status["working_set_mb"].clone();
    }

    // 3. 一致性
    let mut history: Vec<Edit> = Vec::new();
    let mut created = Vec::new();
    let mut passed = 0usize;
    let mut last_good: Option<(usize, PathBuf)> = None;
    let mut failure = None;
    let mut fg = Vec::new();
    let started = Instant::now();
    for n in 1..=edits {
        let edit = next_edit(&mut rng, &root, n, &mut sources, &mut created);
        apply(&root, &edit.after);
        let r = resident.changed(&edit.paths());
        fg.push(r["elapsed_ms"].as_f64().unwrap_or(0.0));
        // 编辑日志：复现与排查用（同一 seed 生成同一串编辑）
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(scratch.join("edits.log"))
            .unwrap();
        let _ = writeln!(
            log,
            "{n}	{}	{}	{} ms	queued {}",
            edit.kind,
            edit.paths().join(" "),
            r["elapsed_ms"],
            r["impact"]["queued"]
        );
        history.push(edit);
        if n % every == 0 || n == edits {
            resident.call("wait_idle", json!({}));
            let reference = scratch.join(format!("full-{n}.rvcsidx"));
            full_index(&root, &reference);
            let full = CsIndex::open(&reference).unwrap();
            let (index, overlay) = merged(&base, &journal);
            let diffs = compare(
                &L2View::new(&full, None),
                &L2View::new(&index, overlay.as_ref()),
            );
            println!(
                "checkpoint {n}: {} ({:.0}s elapsed, overlay {} files)",
                if diffs.is_empty() {
                    "consistent".to_owned()
                } else {
                    format!("{} differences", diffs.len())
                },
                started.elapsed().as_secs_f64(),
                overlay.as_ref().map_or(0, |o| o.file_count())
            );
            drop(full);
            if diffs.is_empty() {
                passed = n;
                if let Some((_, old)) = last_good.replace((n, reference)) {
                    let _ = fs::remove_file(old);
                }
            } else {
                for d in diffs.iter().take(20) {
                    println!("  {d}");
                }
                failure = Some((n, diffs));
                break;
            }
        }
    }
    resident.stop();
    fg.sort_by(f64::total_cmp);
    report["consistency"] = json!({
        "edits": history.len(), "checkpoint_every": every, "consistent_through": passed,
        "foreground_ms": { "p50": percentile(&fg, 0.5), "p99": percentile(&fg, 0.99) },
        "kinds": history.iter().fold(BTreeMap::<&str, usize>::new(), |mut m, e| { *m.entry(e.kind).or_default() += 1; m }),
    });

    // 4. 不一致：从上一个通过的检查点二分重放，定位第一次出错的编辑
    if let Some((bad, diffs)) = failure {
        let (good, good_index) = last_good.clone().unwrap_or((0, base.clone()));
        println!(
            "bisecting edits {}..={bad} from checkpoint {good}",
            good + 1
        );
        let probe = |m: usize| -> bool {
            // 回到检查点 good 的状态，用它的全量索引做基础，只重放 good+1..=m
            for e in history[good..].iter().rev() {
                apply(&root, &e.before);
            }
            let journal = scratch.join(format!("probe-{m}.rvcsovl"));
            let _ = fs::remove_file(&journal);
            let mut r = Resident::start(&root, &good_index, &journal, 1.0);
            for e in &history[good..m] {
                apply(&root, &e.after);
                r.changed(&e.paths());
            }
            r.call("wait_idle", json!({}));
            r.stop();
            let reference = scratch.join(format!("probe-{m}.rvcsidx"));
            full_index(&root, &reference);
            let full = CsIndex::open(&reference).unwrap();
            let (index, overlay) = merged(&good_index, &journal);
            let ok = compare(
                &L2View::new(&full, None),
                &L2View::new(&index, overlay.as_ref()),
            )
            .is_empty();
            println!(
                "  replay through edit {m}: {}",
                if ok { "consistent" } else { "inconsistent" }
            );
            ok
        };
        let (mut lo, mut hi) = (good, bad);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if probe(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let culprit = &history[hi - 1];
        println!(
            "first inconsistent edit: #{} ({}) on {:?}",
            culprit.n,
            culprit.kind,
            culprit.paths()
        );
        report["consistency"]["first_bad_edit"] = json!({ "n": culprit.n, "kind": culprit.kind, "paths": culprit.paths(), "differences": diffs.iter().take(50).collect::<Vec<_>>() });
    }
    let out = target.join(format!("report-{which}.json"));
    fs::create_dir_all(&target).unwrap();
    fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("report: {}", out.display());
    if report["consistency"].get("first_bad_edit").is_some() {
        std::process::exit(1);
    }
}
