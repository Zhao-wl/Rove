//! 通用列表、树与快速输入的数据侧基准（issue #18）。
//!
//! 只调用 `rove_ui::list_widget` 的公开接口，不开窗口：
//! - `lists/scroll_frame_100k`：10 万条列表每帧的数据工作（移动滚动位置 → 可见范围 → 取可见行）；
//! - `lists/tree_expand_10000`：展开一个有 1 万个子节点的目录（懒加载 + 展平 + 重新布局）；
//! - `lists/filter_100k_contiguous`：10 万条输入过滤（一次按键的同步刷新成本）。
//!
//! 含 GPUI 布局与绘制的每帧 CPU、帧间隔需要真实窗口与 vsync，见
//! `cargo run -p rove-ui --release --example list_bench`（口径同 M0_UI_SPIKE_REPORT §2.5）。
use std::{hint::black_box, sync::Arc};

use criterion::{criterion_group, criterion_main, Criterion};
use rove_ui::{
    list_widget::{
        matcher::{FindMode, MatchType},
        options::ListOptions,
        plain_row,
        tree::{ChildCount, Children, TreeItem, TreeModel, TreeSource},
        widget::{ListWidget, RowHeight},
        StaticList,
    },
    lists::ItemId,
};

fn hundred_thousand() -> Arc<StaticList> {
    let rows: Vec<_> = (0..100_000)
        .map(|i| {
            plain_row(
                &format!("item{i}"),
                &format!("Assets/Scripts/Module{}/File{i:06}.cs", i % 97),
            )
        })
        .collect();
    Arc::new(StaticList::new(rows, rove_base::Version(0)))
}

struct BigFolder;
impl TreeSource for BigFolder {
    fn children(&self, parent: Option<&ItemId>) -> Children {
        Children::Ready(match parent {
            None => Arc::from(vec![TreeItem {
                id: ItemId("big".into()),
                label: "big".into(),
                detail: None,
                children: ChildCount::Known(10_000),
            }]),
            Some(_) => (0..10_000)
                .map(|i| TreeItem {
                    id: ItemId(format!("big/File{i:05}.cs").into()),
                    label: format!("File{i:05}.cs").into(),
                    detail: None,
                    children: ChildCount::None,
                })
                .collect(),
        })
    }
}

fn bench_lists(c: &mut Criterion) {
    let model = hundred_thousand();
    let mut group = c.benchmark_group("lists");

    let mut list = ListWidget::new(
        model.clone(),
        RowHeight::Fixed(22.0),
        ListOptions::default(),
    );
    list.set_viewport(900.0, 420.0);
    let max = list.total_height() - 900.0;
    let mut y = 0.0f32;
    group.bench_function("scroll_frame_100k", |b| {
        b.iter(|| {
            y = if y + 66.0 > max { 0.0 } else { y + 66.0 };
            list.set_scroll_top(y);
            let range = list.visible_range();
            black_box(list.display_rows(range))
        })
    });

    group.bench_function("tree_expand_10000", |b| {
        b.iter_batched(
            || {
                let mut w = ListWidget::tree(
                    TreeModel::new(Arc::new(BigFolder)),
                    RowHeight::Fixed(22.0),
                    ListOptions::default(),
                );
                w.set_viewport(900.0, 420.0);
                w
            },
            |mut w| {
                w.expand_id(&ItemId("big".into()), true).unwrap();
                black_box(w.display_rows(w.visible_range()))
            },
            criterion::BatchSize::SmallInput,
        )
    });

    let contiguous = ListOptions {
        find_match_type: MatchType::Contiguous,
        ..Default::default()
    };
    let mut filtered = ListWidget::new(model, RowHeight::Fixed(22.0), contiguous);
    filtered.set_viewport(900.0, 420.0);
    let mut flip = false;
    group.bench_function("filter_100k_contiguous", |b| {
        b.iter(|| {
            flip = !flip;
            let q = if flip { "file0123" } else { "module42" };
            filtered.set_filter_query(Some((q.into(), FindMode::Filter)));
            black_box(filtered.len())
        })
    });
    group.finish();
}

criterion_group!(benches, bench_lists);
criterion_main!(benches);
