//! #35 入口集成测试：真实的常驻索引进程（`rove-indexer-cs --serve`）。
//!
//! - `lifecycle_forwards_changes_to_the_resident_indexer`：启动生命周期（全量构建）→ 查询 → 改文件并通知 →
//!   常驻进程重绑、写覆盖层 → 查询得到新结果 → 改签名后其他文件的引用在后台重绑后更新 → 新增 / 删除 /
//!   重命名文件 → `.csproj` 变化降级为全量重建 → 关闭（常驻进程退出）。
//! - `protocol_*`：直接对进程说 JSON-RPC：被独占的文件稍后重试成功、空闲修剪与自动退出、全量索引消息、
//!   项目结构文件要求全量重建。
//! - `self_test`：影响分析判定规则等纯函数（`--self-test`）。
//!
//! 需要先 `dotnet build tools/rove-indexer-cs -c Release`；没有构建产物时打印“已跳过”并通过。
//! 运行：`cargo test -p rove-lang-cs --test incremental -- --nocapture`。
use rove_base::{Cancellation, Line, Position, Resource, Utf8Column, Version, WorkspacePath};
use rove_lang_cs::lifecycle::{
    process::IndexerComponent, IndexLifecycle, IndexState, LifecycleConfig, ProcessBuilder,
    SourceChange,
};
use rove_lang_cs::query::{NavigationKind, Precision, QueryPoint};
use rove_lang_cs::router::{BufferStates, CsRouter, RoutedTarget, SemanticSource};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(120);

fn indexer_dll() -> Option<PathBuf> {
    let dll = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/rove-indexer-cs/bin/Release/net9.0/rove-indexer-cs.dll");
    if dll.is_file() {
        Some(dll)
    } else {
        eprintln!(
            "已跳过：没有找到 {}（先运行 dotnet build tools/rove-indexer-cs -c Release）",
            dll.display()
        );
        None
    }
}

fn net9_corelib() -> PathBuf {
    let runtimes = Command::new("dotnet")
        .arg("--list-runtimes")
        .output()
        .unwrap();
    String::from_utf8(runtimes.stdout)
        .unwrap()
        .lines()
        .find_map(|line| {
            let (version, folder) = line
                .strip_prefix("Microsoft.NETCore.App ")?
                .split_once(" [")?;
            let path = Path::new(folder.strip_suffix(']')?)
                .join(version)
                .join("System.Private.CoreLib.dll");
            (version.starts_with("9.") && path.is_file()).then_some(path)
        })
        .expect(".NET 9 runtime")
}

const LIB_CS: &str = "namespace Game.Lib\n{\n    public static class Helper\n    {\n        public static int Twice(int value) => value * 2;\n    }\n}\n";
const LIB_CS_LONG: &str = "namespace Game.Lib\n{\n    public static class Helper\n    {\n        public static long Twice(long value) => value * 2;\n    }\n}\n";
const B_CS: &str = "namespace Game\n{\n    public class Enemy\n    {\n        public int Hp;\n        public void Run() { Hp--; }\n    }\n}\n";
const A_CS: &str = "using Game.Lib;\nnamespace Game\n{\n    public class Player\n    {\n        public void Tick(Enemy e)\n        {\n            e.Run();\n            var n = Helper.Twice(e.Hp);\n        }\n    }\n}\n";

/// 两个程序集：Lib（asmdef）与 Assembly-CSharp（引用 Lib）。
fn write_project(root: &Path) {
    let corelib = net9_corelib();
    let corelib = corelib.to_string_lossy();
    fs::create_dir_all(root.join("Assets/Lib")).unwrap();
    fs::write(root.join("Assets/Lib/Lib.asmdef"), r#"{ "name": "Lib" }"#).unwrap();
    fs::write(root.join("Assets/Lib/Helper.cs"), LIB_CS).unwrap();
    fs::write(root.join("Assets/A.cs"), A_CS).unwrap();
    fs::write(root.join("Assets/B.cs"), B_CS).unwrap();
    fs::write(
        root.join("Lib.csproj"),
        format!(
            r#"<Project><PropertyGroup><AssemblyName>Lib</AssemblyName></PropertyGroup><ItemGroup><Compile Include="Assets/Lib/Helper.cs" /><Reference Include="System.Private.CoreLib"><HintPath>{corelib}</HintPath></Reference></ItemGroup></Project>"#
        ),
    )
    .unwrap();
    fs::write(
        root.join("Assembly-CSharp.csproj"),
        format!(
            r#"<Project><PropertyGroup><AssemblyName>Assembly-CSharp</AssemblyName></PropertyGroup><ItemGroup><Compile Include="Assets/A.cs" /><Compile Include="Assets/B.cs" /><Reference Include="System.Private.CoreLib"><HintPath>{corelib}</HintPath></Reference><ProjectReference Include="Lib.csproj" /></ItemGroup></Project>"#
        ),
    )
    .unwrap();
}

fn point(path: &str, line: u32, col: u32) -> QueryPoint {
    QueryPoint {
        resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
        position: Position {
            line: Line(u64::from(line)),
            column: Utf8Column(u64::from(col)),
        },
        version: Version(0),
    }
}

/// (行, 列) of the nth occurrence of `word` in `text`.
fn find(text: &str, word: &str) -> (u32, u32) {
    for (i, line) in text.lines().enumerate() {
        if let Some(c) = line.find(word) {
            return (i as u32, c as u32);
        }
    }
    panic!("{word} not found");
}

/// 导航结果（路径, 行, 列）；有任何一条不是精确结果（文件更新中）时返回 `None`。
fn query(
    router: &CsRouter,
    kind: NavigationKind,
    path: &str,
    at: (u32, u32),
) -> Option<Vec<(String, u64, u64)>> {
    let result = router
        .navigate_detailed(kind, &point(path, at.0, at.1), &Cancellation::default())
        .ok()?;
    if result.precision != Precision::Exact {
        return None;
    }
    let mut out: Vec<(String, u64, u64)> = result
        .value
        .iter()
        .map(|t: &RoutedTarget| {
            let s = t.target.selection.start();
            let path = match &t.target.resource {
                Resource::Workspace(p) => p.as_str().to_owned(),
                other => format!("{other:?}"),
            };
            (path, s.line.0, s.column.0)
        })
        .collect();
    out.sort();
    Some(out)
}

fn targets(
    router: &CsRouter,
    kind: NavigationKind,
    path: &str,
    at: (u32, u32),
) -> Vec<(String, u64, u64)> {
    query(router, kind, path, at).expect("exact results")
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn lifecycle_forwards_changes_to_the_resident_indexer() {
    let Some(dll) = indexer_dll() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Game");
    write_project(&root);
    let cache_root = dir.path().join("cache");
    let component = IndexerComponent::from_path(&dll).unwrap();
    let builder =
        Arc::new(ProcessBuilder::new(move || Some(component.clone())).with_resident(true));
    let lifecycle = Arc::new(IndexLifecycle::start(
        root.clone(),
        LifecycleConfig::new(true, Ok(cache_root.clone())),
        builder,
    ));
    let status = lifecycle.wait_until(WAIT, |s| s.state == IndexState::Ready);
    assert_eq!(status.state, IndexState::Ready, "{status:?}");
    let generation = status.info.generation;
    let semantic: Arc<dyn SemanticSource> = lifecycle.clone();
    let router = CsRouter::new(semantic, Arc::new(BufferStates::new())).with_root(&root);
    let run_call = find(A_CS, "Run");
    assert_eq!(
        targets(&router, NavigationKind::Definition, "Assets/A.cs", run_call),
        vec![("Assets/B.cs".into(), 5, 20)]
    );

    // 1. 改 B.cs（Run 下移两行）→ 通知 → 常驻进程重绑 → 跳转到新位置
    let b2 = B_CS.replace(
        "    public class Enemy\n",
        "    // moved\n    // down\n    public class Enemy\n",
    );
    fs::write(root.join("Assets/B.cs"), &b2).unwrap();
    let started = Instant::now();
    lifecycle.files_changed(&[SourceChange::Changed("Assets/B.cs".into())]);
    wait_for("B.cs overlay", || {
        query(&router, NavigationKind::Definition, "Assets/A.cs", run_call)
            == Some(vec![("Assets/B.cs".into(), 7, 20)])
    });
    eprintln!(
        "single-file update visible after {:?} (cold process start included)",
        started.elapsed()
    );
    assert!(!lifecycle.is_stale("Assets/B.cs"));
    let snapshot = lifecycle.snapshot().unwrap();
    assert_eq!(snapshot.generation, generation, "no full rebuild");
    assert!(snapshot.overlay().is_some());
    // 暴露给 #38 的文件版本 = 覆盖层里的内容哈希
    let view = snapshot.view();
    let b = view.file_id("Assets/B.cs").unwrap();
    assert_eq!(
        view.file_hash(b),
        Some(rove_lang_cs::lifecycle::hash::xxh64(b2.as_bytes(), 0))
    );
    drop(snapshot);

    // 2. 改签名：Twice(int) → Twice(long)。A.cs 没变，它对 Twice 的引用在后台重绑后指向新的声明
    fs::write(root.join("Assets/Lib/Helper.cs"), LIB_CS_LONG).unwrap();
    lifecycle.files_changed(&[SourceChange::Changed("Assets/Lib/Helper.cs".into())]);
    let twice_decl = find(LIB_CS_LONG, "Twice");
    let twice_call = find(A_CS, "Twice");
    wait_for("background rebind of A.cs", || {
        query(
            &router,
            NavigationKind::References,
            "Assets/Lib/Helper.cs",
            twice_decl,
        )
        .is_some_and(|r| {
            r.contains(&(
                "Assets/A.cs".into(),
                u64::from(twice_call.0),
                u64::from(twice_call.1),
            ))
        })
    });
    assert_eq!(
        targets(
            &router,
            NavigationKind::Definition,
            "Assets/A.cs",
            twice_call
        ),
        vec![(
            "Assets/Lib/Helper.cs".into(),
            u64::from(twice_decl.0),
            u64::from(twice_decl.1)
        )]
    );

    // 3. 新增文件
    let c = "namespace Game\n{\n    class Boss : Enemy\n    {\n        void Go() { Run(); }\n    }\n}\n";
    fs::write(root.join("Assets/C.cs"), c).unwrap();
    lifecycle.files_changed(&[SourceChange::Created("Assets/C.cs".into())]);
    let c_run = find(c, "Run");
    wait_for("created file", || {
        router
            .navigate_detailed(
                NavigationKind::Definition,
                &point("Assets/C.cs", c_run.0, c_run.1),
                &Cancellation::default(),
            )
            .is_ok_and(|r| r.value.len() == 1 && r.precision == Precision::Exact)
    });
    assert_eq!(
        targets(&router, NavigationKind::Definition, "Assets/C.cs", c_run),
        vec![("Assets/B.cs".into(), 7, 20)]
    );
    let enemy = find(&b2, "Enemy");
    assert!(
        targets(&router, NavigationKind::DerivedTypes, "Assets/B.cs", enemy).contains(&(
            "Assets/C.cs".into(),
            2,
            10
        ))
    );

    // 4. 重命名：C.cs → D.cs
    fs::rename(root.join("Assets/C.cs"), root.join("Assets/D.cs")).unwrap();
    lifecycle.files_changed(&[SourceChange::Renamed {
        from: "Assets/C.cs".into(),
        to: "Assets/D.cs".into(),
    }]);
    wait_for("renamed file", || {
        query(
            &router,
            NavigationKind::References,
            "Assets/B.cs",
            find(&b2, "Run"),
        )
        .is_some_and(|refs| {
            refs.iter().any(|r| r.0 == "Assets/D.cs") && !refs.iter().any(|r| r.0 == "Assets/C.cs")
        })
    });

    // 5. 删除文件：不留悬空的出现位置
    fs::remove_file(root.join("Assets/D.cs")).unwrap();
    lifecycle.files_changed(&[SourceChange::Removed("Assets/D.cs".into())]);
    wait_for("deleted file", || {
        query(
            &router,
            NavigationKind::References,
            "Assets/B.cs",
            find(&b2, "Run"),
        ) == Some(vec![
            (
                "Assets/A.cs".into(),
                u64::from(run_call.0),
                u64::from(run_call.1),
            ),
            ("Assets/B.cs".into(), 7, 20),
        ])
    });
    assert!(lifecycle
        .snapshot()
        .unwrap()
        .view()
        .file_id("Assets/D.cs")
        .is_none());
    assert_eq!(lifecycle.snapshot().unwrap().generation, generation);

    // 6. .csproj 变化：不做增量，全量重建（新世代）
    let csproj = root.join("Assembly-CSharp.csproj");
    let text = fs::read_to_string(&csproj).unwrap();
    fs::write(
        &csproj,
        text.replace(
            "<ItemGroup>",
            "<ItemGroup><Compile Include=\"Assets/B.cs\" />",
        ),
    )
    .unwrap();
    lifecycle.files_changed(&[SourceChange::Changed("Assembly-CSharp.csproj".into())]);
    let status = lifecycle.wait_until(WAIT, |s| {
        s.state == IndexState::Ready && s.info.generation != generation
    });
    assert_ne!(status.info.generation, generation, "{status:?}");
    assert!(lifecycle.snapshot().unwrap().overlay().is_none());

    // 7. 关闭：常驻进程落盘退出
    let log = status.info.log.clone().unwrap();
    drop(router);
    drop(lifecycle);
    wait_for("resident exit logged", || {
        fs::read_to_string(&log)
            .is_ok_and(|l| l.contains("resident: exited") || l.contains("resident: terminated"))
    });
}

/// 直接说协议的客户端。
struct Resident {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
    next: u64,
}

impl Resident {
    fn start(dll: &Path, args: &[&str]) -> Self {
        let mut child = Command::new("dotnet")
            .arg(dll)
            .arg("--serve")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let _ =
                    tx.send(serde_json::from_str::<Value>(&line).expect("protocol line is JSON"));
            }
        });
        let mut r = Self {
            child,
            stdin,
            rx,
            next: 1,
        };
        let hello = r.recv(|m| m["method"] == "hello");
        assert_eq!(
            hello["params"]["protocol"], 1,
            "version field in the first message"
        );
        assert!(hello["params"]["indexer_version"]
            .as_str()
            .unwrap()
            .starts_with("1.2."));
        r
    }

    fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        )
        .unwrap();
        self.stdin.flush().unwrap();
        id
    }

    fn recv(&mut self, f: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = self
                .rx
                .recv_timeout(left)
                .expect("message from the indexer");
            if f(&msg) {
                return msg;
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        let msg = self.recv(|m| m["id"] == id);
        assert!(msg.get("error").is_none(), "{msg}");
        msg["result"].clone()
    }
}

/// 全量索引 + 初始化好的常驻进程。
fn serve(dir: &Path, dll: &Path, args: &[&str]) -> (Resident, PathBuf) {
    let root = dir.join("Game");
    write_project(&root);
    let base = dir.join("base.rvcsidx");
    let mut r = Resident::start(dll, args);
    // 全量索引消息：带进度
    let id = r.send("index", json!({ "project": root, "output": base }));
    let progress = r.recv(|m| m["method"] == "progress");
    assert_eq!(progress["params"]["total"], 3);
    let done = r.recv(|m| m["id"] == id);
    assert_eq!(done["result"]["files"], 3, "{done}");
    assert!(
        base.with_extension("rvcsidx.decls").is_file(),
        "declaration table"
    );
    let init = r.call(
        "initialize",
        json!({ "protocol": 1, "project": root, "base": base, "journal": dir.join("base.rvcsovl") }),
    );
    assert_eq!(init["segments"], 0);
    (r, root)
}

#[cfg(windows)]
#[test]
fn protocol_locked_file_is_retried_without_stopping_the_process() {
    use std::os::windows::fs::OpenOptionsExt;
    let Some(dll) = indexer_dll() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (mut r, root) = serve(dir.path(), &dll, &[]);
    let path = root.join("Assets/B.cs");
    fs::write(&path, B_CS.replace("Hp--", "Hp -= 2")).unwrap();
    // 独占打开（不共享读写删），模拟 Unity 编辑器或杀毒软件
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let result = r.call(
        "changed",
        json!({ "changes": [{ "path": "Assets/B.cs", "kind": "changed" }] }),
    );
    assert_eq!(result["locked"], json!(["Assets/B.cs"]), "{result}");
    assert!(result["segment"].is_null());
    let status = r.call("status", json!({}));
    assert_eq!(status["locked"], 1);
    drop(lock);
    // 后台重试成功：overlay 通知带上这个文件
    let overlay = r.recv(|m| m["method"] == "overlay");
    assert_eq!(overlay["params"]["files"], json!(["Assets/B.cs"]));
    // 进程照常工作
    let result = r.call("changed", json!({ "changes": [{ "path": "Assets/A.cs" }] }));
    assert_eq!(result["files"], json!(["Assets/A.cs"]));
    assert_eq!(r.call("status", json!({}))["locked"], 0);
    r.call("shutdown", json!({}));
    assert!(r.child.wait().unwrap().success());
}

#[test]
fn protocol_structure_change_requires_full_rebuild_and_impact_is_reported() {
    let Some(dll) = indexer_dll() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (mut r, root) = serve(dir.path(), &dll, &[]);
    for config in [
        "Assembly-CSharp.csproj",
        "Assets/Lib/Lib.asmdef",
        "Assets/Lib/Lib.asmref",
    ] {
        let result = r.call("changed", json!({ "changes": [{ "path": config }] }));
        assert_eq!(result["rebuild_required"], true, "{config}");
    }
    // 只改方法体：没有影响
    fs::write(root.join("Assets/B.cs"), B_CS.replace("Hp--", "Hp -= 3")).unwrap();
    let result = r.call("changed", json!({ "changes": [{ "path": "Assets/B.cs" }] }));
    assert_eq!(result["impact"]["changed"], json!([]), "{result}");
    assert_eq!(result["impact"]["queued"], 0);
    // 可见性变化：Run 变成 internal → 引用它的 A.cs 进后台队列
    fs::write(
        root.join("Assets/B.cs"),
        B_CS.replace("public void Run", "internal void Run"),
    )
    .unwrap();
    let result = r.call("changed", json!({ "changes": [{ "path": "Assets/B.cs" }] }));
    assert_eq!(
        result["impact"]["changed"],
        json!(["Assembly-CSharp|M:Game.Enemy.Run"]),
        "{result}"
    );
    let overlay = r.recv(|m| m["method"] == "overlay");
    assert_eq!(overlay["params"]["files"], json!(["Assets/A.cs"]));
    r.call("wait_idle", json!({}));
    // 新名字：加一个方法 Jump，文本里出现 Jump 的文件进队列
    fs::write(
        root.join("Assets/A.cs"),
        A_CS.replace("e.Run();", "e.Run(); e.Jump();"),
    )
    .unwrap();
    r.call("changed", json!({ "changes": [{ "path": "Assets/A.cs" }] }));
    fs::write(
        root.join("Assets/B.cs"),
        B_CS.replace(
            "public void Run",
            "public void Jump() { }\n        public void Run",
        ),
    )
    .unwrap();
    let result = r.call("changed", json!({ "changes": [{ "path": "Assets/B.cs" }] }));
    assert_eq!(result["impact"]["added_names"], json!(["Jump"]), "{result}");
    let overlay = r.recv(|m| m["method"] == "overlay");
    assert_eq!(overlay["params"]["files"], json!(["Assets/A.cs"]));
    r.call("shutdown", json!({}));
    assert!(r.child.wait().unwrap().success());
}

#[test]
fn protocol_idle_trim_and_exit() {
    let Some(dll) = indexer_dll() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (mut r, root) = serve(
        dir.path(),
        &dll,
        &["--trim-seconds", "1", "--idle-exit-seconds", "4"],
    );
    fs::write(root.join("Assets/B.cs"), B_CS.replace("Hp--", "Hp -= 2")).unwrap();
    r.call("changed", json!({ "changes": [{ "path": "Assets/B.cs" }] }));
    let busy = r.call("status", json!({}));
    assert!(
        !busy["warm_projects"].as_array().unwrap().is_empty(),
        "{busy}"
    );
    std::thread::sleep(Duration::from_millis(2500));
    let idle = r.call("status", json!({}));
    assert_eq!(idle["warm_projects"], json!([]), "trimmed: {idle}");
    eprintln!(
        "tiny project: working set {} MB while warm, {} MB after trim",
        busy["working_set_mb"], idle["working_set_mb"]
    );
    let exiting = r.recv(|m| m["method"] == "exiting");
    assert_eq!(exiting["params"]["reason"], "idle");
    assert!(r.child.wait().unwrap().success());
    // 退出前落盘：日志里有这次重绑
    let journal = fs::read(dir.path().join("base.rvcsovl")).unwrap();
    assert!(journal.starts_with(b"RVCSOVL\0"));
}

#[test]
fn self_test() {
    let Some(dll) = indexer_dll() else { return };
    let out = Command::new("dotnet")
        .arg(dll)
        .arg("--self-test")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

mod fake {
    //! 不依赖索引进程的假常驻进程：把“重绑”写成只含一个出现位置的覆盖层段。
    use rove_base::Cancellation;
    use rove_lang_cs::lifecycle::{
        log::IndexLog, process::BuildRequest, testing::FixtureIndex, BuildFailure,
        IncrementalIndexer, IndexBuilder, Progress, ResidentEvent, ResidentSession, SessionRequest,
    };
    use rove_lang_cs::overlay::{OvKey, SegFile, SegOccurrence, SegSymbol, Segment};
    use rove_lang_cs::CsIndex;
    use std::{
        fs,
        io::Write,
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    pub const FILES: [&str; 4] = [
        "Assets/S0.cs",
        "Assets/S1.cs",
        "Assets/S2.cs",
        "Assets/S3.cs",
    ];

    #[derive(Default)]
    pub struct Builder {
        pub builds: AtomicUsize,
        pub block: AtomicBool,
    }

    impl IndexBuilder for Builder {
        fn build(
            &self,
            request: &BuildRequest,
            _: &mut dyn FnMut(Progress),
            _: &IndexLog,
            cancel: &Cancellation,
        ) -> Result<(), BuildFailure> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            while self.block.load(Ordering::SeqCst) {
                if cancel.check().is_err() {
                    return Err(BuildFailure::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            FixtureIndex::for_project(&request.project_root, &FILES, [0; 32])
                .write(&request.output)
                .map_err(|e| BuildFailure::InvalidOutput(e.to_string()))
        }

        fn incremental(&self) -> Option<Arc<dyn IncrementalIndexer>> {
            Some(Arc::new(Resident))
        }
    }

    struct Resident;
    struct Session {
        base: PathBuf,
        journal: PathBuf,
        sequence: u64,
        events: Box<dyn Fn(ResidentEvent) + Send + Sync>,
    }

    impl IncrementalIndexer for Resident {
        fn start(
            &self,
            request: &SessionRequest,
            events: Box<dyn Fn(ResidentEvent) + Send + Sync>,
            _: Arc<IndexLog>,
        ) -> Result<Box<dyn ResidentSession>, BuildFailure> {
            Ok(Box::new(Session {
                base: request.base.clone(),
                journal: request.journal.clone(),
                sequence: 0,
                events,
            }))
        }
    }

    impl ResidentSession for Session {
        fn changed(&mut self, id: u64, paths: &[String]) -> bool {
            self.sequence += 1;
            let identity = CsIndex::open(&self.base).unwrap().identity();
            let segment = Segment {
                sequence: self.sequence,
                symbols: vec![SegSymbol {
                    key: OvKey::Global("Fake|T:Changed".into()),
                    name: "Changed".into(),
                    display: "Changed".into(),
                    kind: 11,
                    flags: 1,
                    container: None,
                }],
                files: paths
                    .iter()
                    .map(|p| SegFile {
                        path: p.clone(),
                        hash: 0xfeed,
                        occurrences: vec![SegOccurrence {
                            line: 0,
                            col: 0,
                            len: 7,
                            symbol: 0,
                            role: 2,
                        }],
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.journal)
                .unwrap();
            file.write_all(&segment.encode(&identity)).unwrap();
            drop(file);
            (self.events)(ResidentEvent::Applied {
                id,
                files: paths.to_vec(),
                locked: vec![],
                rebuild_required: false,
                queued: 0,
                elapsed_ms: 1.0,
            });
            true
        }
        fn shutdown(&mut self) {}
    }
}

#[test]
fn overlay_limit_triggers_background_rebuild() {
    use rove_lang_cs::lifecycle::OverlayLimits;
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Game");
    fs::create_dir_all(root.join("Assets")).unwrap();
    fs::write(root.join("Game.csproj"), "<Project/>").unwrap();
    for f in fake::FILES {
        fs::write(root.join(f), format!("class {} {{}}", &f[7..9])).unwrap();
    }
    let builder = Arc::new(fake::Builder::default());
    let mut config = LifecycleConfig::new(true, Ok(dir.path().join("cache")));
    config.overlay = OverlayLimits {
        max_files: 3,
        max_journal_bytes: 1 << 30,
    };
    let lifecycle = IndexLifecycle::start(root.clone(), config, builder.clone());
    let status = lifecycle.wait_until(WAIT, |s| s.state == IndexState::Ready);
    let generation = status.info.generation;
    assert_eq!(builder.builds.load(Ordering::SeqCst), 1);
    builder.block.store(true, Ordering::SeqCst);

    lifecycle.files_changed(&[SourceChange::Changed("Assets/S0.cs".into())]);
    wait_for("first overlay", || {
        lifecycle
            .snapshot()
            .is_some_and(|s| s.overlay().is_some_and(|o| o.file_count() == 1))
            && !lifecycle.is_stale("Assets/S0.cs")
    });
    let snap = lifecycle.snapshot().unwrap();
    let view = snap.view();
    let s0 = view.file_id("Assets/S0.cs").unwrap();
    assert_eq!(view.file_hash(s0), Some(0xfeed));
    assert!(view.occurrence_at(s0, 0, Utf8Column(1)).is_some());
    drop(snap);
    assert_eq!(lifecycle.status().state, IndexState::Ready);

    // 第 3 个文件到达阈值：后台全量重建（被挡住），期间旧索引 + 覆盖层照常可查询
    lifecycle.files_changed(&[
        SourceChange::Changed("Assets/S1.cs".into()),
        SourceChange::Changed("Assets/S2.cs".into()),
    ]);
    let status = lifecycle.wait_until(WAIT, |s| matches!(s.state, IndexState::Building { .. }));
    assert!(
        matches!(
            status.state,
            IndexState::Building {
                rebuilding: true,
                ..
            }
        ),
        "{status:?}"
    );
    let snap = lifecycle.snapshot().expect("old index stays queryable");
    assert_eq!(snap.generation, generation);
    let view = snap.view();
    assert_eq!(snap.overlay().unwrap().file_count(), 3);
    let s2 = view.file_id("Assets/S2.cs").unwrap();
    assert!(view.occurrence_at(s2, 0, Utf8Column(0)).is_some());
    drop(snap);

    builder.block.store(false, Ordering::SeqCst);
    let status = lifecycle.wait_until(WAIT, |s| {
        s.state == IndexState::Ready && s.info.generation != generation
    });
    assert_ne!(status.info.generation, generation, "{status:?}");
    assert_eq!(builder.builds.load(Ordering::SeqCst), 2);
    let snap = lifecycle.snapshot().unwrap();
    assert!(
        snap.overlay().is_none(),
        "the new generation starts without overlays"
    );
    // 旧世代的日志随旧世代删除
    drop(snap);
    let cache = rove_lang_cs::lifecycle::ProjectCache::new(&dir.path().join("cache"), &root);
    wait_for("old journal removed", || {
        !cache.overlay_path(generation).exists()
    });
}
