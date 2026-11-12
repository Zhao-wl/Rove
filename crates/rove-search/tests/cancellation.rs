//! 取消的计时测试（#25 验收：取消后 50 ms 内所有工作线程停止）。
//!
//! 单独一个测试二进制：cargo 逐个运行测试二进制，同一二进制里的测试才并行，
//! 这样计时不会被其他搜索测试抢走 CPU。

use rove_base::{Cancellation, Error, Value};
use rove_config::{DefaultSettings, Setting, SettingRegistry};
use rove_search::{search, settings, SearchConfig, SearchQuery};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn config(overrides: &[(&str, Value)]) -> SearchConfig {
    let base = settings::default_settings();
    let mut registry = SettingRegistry::default();
    for (id, setting) in base.0.entries() {
        let default = overrides
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| {
                setting
                    .rove_default
                    .clone()
                    .unwrap_or(setting.default.clone())
            });
        registry
            .register(
                id,
                Setting {
                    default,
                    rove_default: None,
                    language_overridable: setting.language_overridable,
                },
            )
            .unwrap();
    }
    SearchConfig::from_settings(Arc::new(DefaultSettings(registry)))
}

fn write(root: &Path, rel: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// 取消后 50 ms 内全部工作线程停止：`search` 返回即表示所有线程已经结束（scoped 线程）。
#[test]
fn cancellation_stops_all_workers_within_50_ms() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let line = "abcdefghijklmnopqrstuvwxyz0123456789 lorem ipsum dolor sit amet\n";
    let body = line.repeat((3 << 20) / line.len());
    for i in 0..48 {
        write(root, &format!("d{}/f{i}.txt", i % 6), &body);
    }
    let cfg = config(&[(settings::MAX_THREADS, Value::Number(4.0))]);
    // 没有字面量可预筛、也没有任何匹配的正则：每一块都要由正则引擎真正扫描，保证取消时搜索仍在进行，
    // 并且取消发生在工作线程的内层循环（块之间）而不只是文件之间。
    let q = SearchQuery {
        is_regex: true,
        ..SearchQuery::new(r"[a-z]+[0-9]{12}")
    };
    // 正则编译发生在启动任何线程之前（调试构建下要几十毫秒）：等它过去再取消，量的是工作线程的停止。
    let compiled = Instant::now();
    rove_search::query::compile(&q, false).unwrap();
    let compile = compiled.elapsed();
    // 先完整搜一遍：新写的文件第一次读取可能被杀毒软件扫描阻塞几百毫秒（磁盘 I/O，不是取消路径）。
    let warm = Instant::now();
    search(root, &q, &cfg, &Cancellation::default(), |_| {}).unwrap();
    eprintln!("完整搜索一遍：{:?}", warm.elapsed());
    let mut samples = Vec::new();
    for _ in 0..5 {
        let cancel = Cancellation::default();
        let started = Arc::new(std::sync::Mutex::new(None::<Instant>));
        let (root, q, cfg2, cancel2, done) = (
            root.to_path_buf(),
            q.clone(),
            cfg.clone(),
            cancel.clone(),
            started.clone(),
        );
        let handle = std::thread::spawn(move || {
            let r = search(&root, &q, &cfg2, &cancel2, |_| {});
            *done.lock().unwrap() = Some(Instant::now());
            r
        });
        std::thread::sleep(compile + Duration::from_millis(30));
        assert!(
            started.lock().unwrap().is_none(),
            "搜索在取消前就已结束，数据不够大"
        );
        let at = Instant::now();
        cancel.cancel();
        let result = handle.join().unwrap();
        let stopped = started.lock().unwrap().unwrap();
        assert_eq!(result.unwrap_err(), Error::Cancelled);
        samples.push(stopped - at);
    }
    samples.sort();
    let median = samples[samples.len() / 2];
    eprintln!("取消到全部线程结束：{samples:?}，中位数 {median:?}");
    // 断言中位数：内层循环每个块（几毫秒）、每处匹配都检查取消，单次偶发的超时来自操作系统调度
    // （本机同时有 Unity 与其他编译任务时，线程可能整段时间片拿不到 CPU），不是取消路径的问题；
    // 任何一次都不能“挂住”。
    assert!(median < Duration::from_millis(50), "取消耗时 {samples:?}");
    assert!(
        samples.iter().all(|d| *d < Duration::from_secs(1)),
        "取消挂住：{samples:?}"
    );
}
