//! #56 缩略图离线生成与缓存上限。每个样本生成 30 万行的所有纹理。

use criterion::{criterion_group, criterion_main, Criterion};
use rove_theme::Color;
use rove_ui::code_view::minimap::{Tile, TileCache, TILE_ROWS};
use std::{hint::black_box, time::Duration};

const ROWS: u32 = 300_000;
const COLUMNS: u32 = 120;
const LINE: &str = "    private readonly Dictionary<string, Component> components = new();";
const FOREGROUND: Color = Color(204, 204, 204, 255);
const KEYWORD: Color = Color(86, 156, 214, 255);
const TYPE: Color = Color(78, 201, 176, 255);

fn all_tiles() -> (usize, usize) {
    let mut cache = TileCache::default();
    let mut total_bytes = 0;
    let mut active_bytes = 0;
    let mut max_active_bytes = 0;
    for start in (0..ROWS).step_by(TILE_ROWS as usize) {
        let rows = (ROWS - start).min(TILE_ROWS);
        let mut tile = Tile::new(COLUMNS, rows);
        for row in 0..rows {
            tile.paint_row(
                row,
                LINE,
                [(4..20, KEYWORD), (21..49, TYPE)],
                FOREGROUND,
                255,
                4,
            );
        }
        let image = tile.into_image();
        let bytes = image.as_bytes(0).unwrap().len();
        total_bytes += bytes;
        active_bytes += bytes;
        max_active_bytes = max_active_bytes.max(active_bytes);
        for evicted in cache.insert(u64::from(start / TILE_ROWS), image) {
            active_bytes -= evicted.as_bytes(0).unwrap().len();
        }
    }
    (total_bytes, max_active_bytes)
}

fn bench(c: &mut Criterion) {
    let (total, peak) = all_tiles();
    assert_eq!(total, ROWS as usize * COLUMNS as usize * 4);
    println!("minimap/300000：所有纹理 {total} B，含新纹理的缓存峰值 {peak} B");
    c.benchmark_group("minimap")
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .bench_function("rasterize_300000_rows", |b| {
            b.iter(|| black_box(all_tiles()))
        });
}

criterion_group!(benches, bench);
criterion_main!(benches);
