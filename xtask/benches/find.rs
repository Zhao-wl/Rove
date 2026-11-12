//! #21：当前文件查找的公开接口基准（不含窗口；窗口里的帧预算见 `scroll_unique` 的 find_visible 场景）。
//!
//! 文本是内存生成的 10 万行（约 4 MB）Unity 风格 C#，用 `RopeText` 作为 `TextSource`。
//!
//! - `find/input_char_100k_lines`：在查找框里多键入一个字符（普通文本、忽略大小写）到匹配计数更新：
//!   编译查询 + 后台整篇扫描 + 把结果交回模型（验收：< 16 ms）；
//! - `find/input_char_100k_lines_case_sensitive`：区分大小写的普通文本（memchr 快路径）；
//! - `find/input_char_100k_lines_regex`：JavaScript 正则（逐行匹配）；
//! - `find/next_match_far_100k_lines`：从文件中部找下一个匹配（引擎导航，匹配列表被截断时的回退路径）。
use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{Cancellation, Line, Position, Utf8Column};
use rove_buffer::RopeText;
use rove_ui::find::engine::{find_next, FindOptions, Query, Searcher};
use rove_ui::find::state::FindModel;
use std::hint::black_box;

const SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

/// 10 万行：每 97 行出现一次 `TargetComponent`，其余是普通语句。
fn source() -> String {
    let mut s = String::with_capacity(4_500_000);
    for i in 0..100_000u32 {
        if i % 97 == 0 {
            s.push_str(&format!(
                "        var target{i} = GetComponent<TargetComponent>(); // {i}\n"
            ));
        } else {
            s.push_str(&format!(
                "        int value{i} = Mathf.Max({i}, speed * deltaTime + offset);\n"
            ));
        }
    }
    s
}

fn typed(model: &mut FindModel, text: &RopeText, search: &str) -> usize {
    model.set_search(search);
    let job = model.start_search().expect("查询有效");
    let out = job.run(text, &mut |_| {});
    model.finish(job.generation, out);
    model.matches().len()
}

fn benchmark(c: &mut Criterion) {
    let text = RopeText::from_text(&source());
    let mut group = c.benchmark_group("find");
    for (id, options, search) in [
        (
            "input_char_100k_lines",
            FindOptions::default(),
            "targetcompone",
        ),
        (
            "input_char_100k_lines_case_sensitive",
            FindOptions {
                case_sensitive: true,
                ..Default::default()
            },
            "TargetCompone",
        ),
        (
            "input_char_100k_lines_regex",
            FindOptions {
                regex: true,
                case_sensitive: true,
                ..Default::default()
            },
            "Target\\w+nent",
        ),
    ] {
        let mut model = FindModel::new(SEPARATORS.into());
        model.set_options(options);
        assert!(typed(&mut model, &text, search) >= 1000);
        group.bench_function(id, |b| {
            b.iter(|| black_box(typed(&mut model, &text, search)))
        });
    }
    let searcher = Searcher::compile(&Query {
        text: "TargetComponent".into(),
        options: FindOptions {
            case_sensitive: true,
            ..Default::default()
        },
        word_separators: SEPARATORS.into(),
    })
    .unwrap()
    .unwrap();
    let cancel = Cancellation::default();
    group.bench_function("next_match_far_100k_lines", |b| {
        b.iter(|| {
            let from = Position {
                line: Line(50_001),
                column: Utf8Column(0),
            };
            black_box(find_next(&text, &searcher, None, from, true, true, &cancel).unwrap())
        })
    });
    group.finish();
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
