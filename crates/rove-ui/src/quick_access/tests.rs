//! 快速打开核心逻辑的单元测试（#24）：前缀与后缀解析、转到行、命令面板排序、文件索引的增量更新、
//! 按键间剪枝与完整扫描等价、历史加权。fuzzyScorer 本身的一致性测试在 `rove-fuzzy`。
#![allow(clippy::single_range_in_vec_init)]
use super::{
    commands::{palette_picks, CommandEntry, CommandHistory, PaletteLabels},
    files::{FileIndex, FileLabels, HistorySort, Search, SearchOptions, MAX_RESULTS},
    lines::{state, LineState},
    matcher::FuzzyMatcher,
    parse::{parse, split_file_target, FileTarget, Mode},
};
use crate::list_widget::matcher::Matcher;
use rove_base::WorkspacePath;
use rove_workspace::FileEvent;
use std::sync::Arc;

fn wp(path: &str) -> WorkspacePath {
    WorkspacePath::new(path).unwrap()
}

fn index(paths: &[&str]) -> FileIndex {
    let mut index = FileIndex::default();
    index.apply(
        &paths
            .iter()
            .map(|p| FileEvent::Created(wp(p)))
            .collect::<Vec<_>>(),
    );
    index
}

fn labels() -> FileLabels {
    FileLabels {
        recent: "最近打开".into(),
        files: "文件结果".into(),
    }
}

fn names(index: &FileIndex, hits: &[super::files::Hit]) -> Vec<String> {
    hits.iter()
        .map(|h| index.entry(h.entry).unwrap().path.as_str().to_owned())
        .collect()
}

#[test]
fn prefixes_select_the_mode_only_at_the_start() {
    assert_eq!(parse(">toggle"), Mode::Commands("toggle"));
    assert_eq!(parse(":12:3"), Mode::Line("12:3"));
    assert_eq!(parse("@move"), Mode::Symbols("move"));
    assert_eq!(parse("#Player"), Mode::WorkspaceSymbols("Player"));
    assert_eq!(
        parse("player"),
        Mode::Files {
            query: "player",
            target: None
        }
    );
    // 中间出现的前缀字符不改变模式
    assert!(matches!(parse("a>b"), Mode::Files { .. }));
    assert!(matches!(parse(""), Mode::Files { query: "", .. }));
}

#[test]
fn file_queries_carry_line_and_column_suffixes() {
    assert_eq!(
        split_file_target("Player.cs:12"),
        (
            "Player.cs",
            Some(FileTarget {
                line: 12,
                column: 0
            })
        )
    );
    assert_eq!(
        split_file_target("Player.cs:12:5"),
        (
            "Player.cs",
            Some(FileTarget {
                line: 12,
                column: 5
            })
        )
    );
    assert_eq!(
        split_file_target("a/b.cs(7,2)"),
        ("a/b.cs", Some(FileTarget { line: 7, column: 2 }))
    );
    assert_eq!(split_file_target("Player.cs"), ("Player.cs", None));
    // 盘符之类的冒号后面不是数字：不拆
    assert_eq!(split_file_target("C:\\x"), ("C:\\x", None));
    assert_eq!(split_file_target("a:"), ("a:", None));
}

#[test]
fn goto_line_follows_vscode_rules() {
    assert_eq!(
        state("12", 100),
        LineState::Go {
            line: 12,
            column: None
        }
    );
    assert_eq!(
        state("12:5", 100),
        LineState::Go {
            line: 12,
            column: Some(5)
        }
    );
    assert_eq!(
        state("12,5", 100),
        LineState::Go {
            line: 12,
            column: Some(5)
        }
    );
    // 负数从末尾数起，-1 是最后一行
    assert_eq!(
        state("-1", 100),
        LineState::Go {
            line: 100,
            column: None
        }
    );
    assert_eq!(
        state("-100", 100),
        LineState::Go {
            line: 1,
            column: None
        }
    );
    // 越界、0 与非数字都只提示
    assert_eq!(state("101", 100), LineState::Prompt);
    assert_eq!(state("0", 100), LineState::Prompt);
    assert_eq!(state("-101", 100), LineState::Prompt);
    assert_eq!(state("", 100), LineState::Prompt);
    assert_eq!(state("abc", 100), LineState::Prompt);
}

fn entry(id: &str, label: &str, key: Option<&str>) -> CommandEntry {
    CommandEntry {
        id: id.into(),
        label: label.into(),
        keybinding: key.map(Into::into),
    }
}

fn palette_labels() -> PaletteLabels {
    PaletteLabels {
        recent: "最近使用".into(),
        other: "其他命令".into(),
    }
}

#[test]
fn palette_groups_recent_first_then_sorts_by_name() {
    let entries = vec![
        entry("c", "Toggle Panel", Some("Ctrl+J")),
        entry("a", "Close Editor", None),
        entry("b", "Toggle Sidebar", Some("Ctrl+B")),
    ];
    let mut history = CommandHistory::default();
    let none = palette_picks(&entries, &history, "", &palette_labels());
    let labels: Vec<_> = none.rows.iter().map(|r| &*r.label).collect();
    assert_eq!(labels, ["Close Editor", "Toggle Panel", "Toggle Sidebar"]);
    assert!(none.separators.is_empty(), "没有最近使用时不分组");
    assert_eq!(none.decor[1].trailing.as_deref(), Some("Ctrl+J"));

    history.record("b");
    history.record("c"); // c 更近
    let picks = palette_picks(&entries, &history, "", &palette_labels());
    let ids: Vec<_> = picks.rows.iter().map(|r| &*r.id.0).collect();
    assert_eq!(ids, ["c", "b", "a"]);
    let groups: Vec<_> = picks
        .separators
        .iter()
        .map(|(i, l)| (*i, l.as_deref().unwrap()))
        .collect();
    assert_eq!(groups, [(0, "最近使用"), (2, "其他命令")]);
}

#[test]
fn palette_filter_uses_the_word_filter_and_highlights() {
    let entries = vec![
        entry("a", "Toggle Side Bar Visibility", None),
        entry("b", "Toggle Panel", None),
        entry("c", "切换侧边栏", None),
    ];
    let history = CommandHistory::default();
    let picks = palette_picks(&entries, &history, "togg side", &palette_labels());
    assert_eq!(picks.rows.len(), 1);
    assert_eq!(picks.decor[0].label_highlights, [0..4, 6..11]);
    let cjk = palette_picks(&entries, &history, "侧边", &palette_labels());
    assert_eq!(&*cjk.rows[0].id.0, "c");
    assert_eq!(cjk.decor[0].label_highlights, [6..12]);
    // 命令 ID 完全相同也算匹配
    let by_id = palette_picks(&entries, &history, "b", &palette_labels());
    assert!(by_id.rows.iter().any(|r| &*r.id.0 == "b"));
    // 标签重复时显示命令 ID
    let dup = vec![entry("x", "Same", None), entry("y", "Same", None)];
    let picks = palette_picks(&dup, &history, "", &palette_labels());
    assert_eq!(picks.rows[0].detail.as_deref(), Some("x"));
}

#[test]
fn file_index_follows_workspace_events() {
    let mut idx = index(&["Assets/A.cs", "Assets/Sub/B.cs", "README.md"]);
    assert_eq!(idx.len(), 3);
    let gen = idx.generation();
    idx.apply(&[FileEvent::Changed(wp("README.md"))]);
    assert_eq!(idx.generation(), gen, "内容变化不影响候选");
    idx.apply(&[FileEvent::Created(wp("Assets/C.cs"))]);
    assert!(idx.find("Assets/C.cs").is_some());
    // 重命名文件与目录
    idx.apply(&[FileEvent::Renamed {
        from: wp("Assets/A.cs"),
        to: wp("Assets/AA.cs"),
    }]);
    assert!(idx.find("Assets/A.cs").is_none() && idx.find("Assets/AA.cs").is_some());
    idx.apply(&[FileEvent::Renamed {
        from: wp("Assets/Sub"),
        to: wp("Assets/Other"),
    }]);
    assert!(idx.find("Assets/Sub/B.cs").is_none() && idx.find("Assets/Other/B.cs").is_some());
    // 删除目录带走其下文件；swap_remove 后查找表仍然正确
    idx.apply(&[FileEvent::Removed(wp("Assets/Other"))]);
    assert!(idx.find("Assets/Other/B.cs").is_none());
    for p in ["Assets/AA.cs", "Assets/C.cs", "README.md"] {
        let at = idx.find(p).unwrap();
        assert_eq!(idx.entry(at).unwrap().path.as_str(), p);
    }
    assert!(idx.apply(&[FileEvent::RescanRequired]));
}

#[test]
fn file_results_show_name_then_directory_with_highlights() {
    let idx = index(&[
        "Assets/Scripts/Player.cs",
        "Assets/Scripts/PlayerInput.cs",
        "Docs/play.md",
    ]);
    let mut search = Search::default();
    let results = search.run(&idx, "player", &[], SearchOptions::default());
    let picks = results.picks(&idx, &labels());
    assert_eq!(
        picks.rows[0].label.as_ref(),
        "Player.cs",
        "前缀匹配且更短的在前"
    );
    assert_eq!(picks.rows[1].label.as_ref(), "PlayerInput.cs");
    assert_eq!(picks.decor[0].label_highlights, [0..6]);
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(
        picks.rows[0].detail.as_deref(),
        Some(format!("Assets{sep}Scripts").as_str())
    );
    // 路径部分的匹配高亮在 detail 上
    let results = search.run(&idx, "scripts player", &[], SearchOptions::default());
    let picks = results.picks(&idx, &labels());
    assert!(!picks.rows.is_empty());
    assert!(!picks.decor[0].detail_highlights.is_empty());
    assert_eq!(picks.separators, [(0, Some("文件结果".into()))]);
}

#[test]
fn history_comes_first_and_follows_the_sort_order_setting() {
    let idx = index(&["a/Alpha.cs", "b/Alpha2.cs", "c/Beta.cs", "d/Alphabet.cs"]);
    let history = vec![wp("b/Alpha2.cs"), wp("c/Beta.cs"), wp("d/Alphabet.cs")];
    let mut search = Search::default();
    // 空查询：只列历史，保持最近打开的顺序
    let r = search.run(&idx, "", &history, SearchOptions::default());
    assert_eq!(
        names(&idx, &r.history),
        ["b/Alpha2.cs", "c/Beta.cs", "d/Alphabet.cs"]
    );
    assert!(r.files.is_empty());
    // 有查询：历史里匹配的先列出；默认按相关度（更短的前缀匹配在前），`recency` 保持最近优先
    let r = search.run(&idx, "alpha", &history, SearchOptions::default());
    assert_eq!(names(&idx, &r.history), ["b/Alpha2.cs", "d/Alphabet.cs"]);
    assert_eq!(
        names(&idx, &r.files),
        ["a/Alpha.cs"],
        "历史中的文件不在文件结果里重复"
    );
    let r = search.run(
        &idx,
        "alpha",
        &[wp("d/Alphabet.cs"), wp("b/Alpha2.cs")],
        SearchOptions {
            include_history: true,
            history_sort: HistorySort::Recency,
        },
    );
    assert_eq!(names(&idx, &r.history), ["d/Alphabet.cs", "b/Alpha2.cs"]);
    // includeHistory = false：没有历史分组
    let r = search.run(
        &idx,
        "alpha",
        &history,
        SearchOptions {
            include_history: false,
            history_sort: HistorySort::Default,
        },
    );
    assert!(r.history.is_empty());
    assert_eq!(r.files.len(), 3);
    let picks = r.picks(&idx, &labels());
    assert_eq!(picks.separators.len(), 1);
}

/// 确定性的伪随机数，生成大量形如 Unity 项目的路径。
fn synthetic_paths(count: usize) -> Vec<String> {
    let mut x: u64 = 0x2545F4914F6CDD1D;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let words = [
        "Player",
        "Enemy",
        "Weapon",
        "Inventory",
        "UI",
        "Menu",
        "Level",
        "Camera",
        "Audio",
        "Network",
        "Spawner",
        "Health",
        "Score",
        "Input",
        "Manager",
        "Controller",
        "View",
    ];
    let dirs = [
        "Assets", "Scripts", "Gameplay", "Editor", "Tests", "Plugins", "Prefabs",
    ];
    (0..count)
        .map(|i| {
            let depth = 1 + next() as usize % 4;
            let mut path: Vec<String> = (0..depth)
                .map(|_| dirs[next() as usize % dirs.len()].to_string())
                .collect();
            let name = format!(
                "{}{}{}",
                words[next() as usize % words.len()],
                words[next() as usize % words.len()],
                i
            );
            path.push(format!("{name}.cs"));
            path.join("/")
        })
        .collect()
}

#[test]
fn incremental_pruning_gives_the_same_results_as_a_full_scan() {
    let paths = synthetic_paths(3000);
    let idx = index(&paths.iter().map(String::as_str).collect::<Vec<_>>());
    for target in [
        "PlayerHealth",
        "assets enemy",
        "Scripts/UI",
        "\"Menu\"",
        "cam 12",
    ] {
        let mut incremental = Search::default();
        let mut typed = String::new();
        for ch in target.chars() {
            typed.push(ch);
            let a = incremental.run(&idx, &typed, &[], SearchOptions::default());
            let b = Search::default().run(&idx, &typed, &[], SearchOptions::default());
            assert_eq!(
                names(&idx, &a.files),
                names(&idx, &b.files),
                "查询 {typed:?}"
            );
        }
    }
    // 退格（缩短查询）也要得到完整结果
    let mut s = Search::default();
    s.run(&idx, "playerhealth", &[], SearchOptions::default());
    let a = s.run(&idx, "player", &[], SearchOptions::default());
    let b = Search::default().run(&idx, "player", &[], SearchOptions::default());
    assert_eq!(names(&idx, &a.files), names(&idx, &b.files));
    assert_eq!(a.files.len(), MAX_RESULTS.min(a.files.len()));
}

#[test]
fn parallel_scoring_gives_the_same_results_as_a_single_thread() {
    let paths = synthetic_paths(6000);
    let idx = index(&paths.iter().map(String::as_str).collect::<Vec<_>>());
    for query in ["c", "pl", "enemy spawner", "Scripts/UI", "\"Menu\"", "zzz"] {
        let mut one = Search::default();
        one.set_threads(Some(1));
        let mut many = Search::default();
        many.set_threads(Some(5));
        let a = one.run(&idx, query, &[], SearchOptions::default());
        let b = many.run(&idx, query, &[], SearchOptions::default());
        assert_eq!(
            names(&idx, &a.files),
            names(&idx, &b.files),
            "查询 {query:?}"
        );
        // 下一次按键的剪枝集合也一致
        let a = one.run(&idx, &format!("{query}e"), &[], SearchOptions::default());
        let b = many.run(&idx, &format!("{query}e"), &[], SearchOptions::default());
        assert_eq!(
            names(&idx, &a.files),
            names(&idx, &b.files),
            "查询 {query:?}e"
        );
    }
}

/// 整条优化过的流水线（紧凑数据、摘要打分、前缀剪枝、共享下限、并行、跳过用不上的 description 匹配）必须给出
/// 与朴素实现（逐个 `score_item` + VS Code 比较器，选出前 N 名）完全相同的结果。
#[test]
fn optimized_pipeline_matches_a_plain_vscode_implementation() {
    use rove_fuzzy::{compare_scored, prepare_query, score_item, ItemScore, Scored};
    let mut paths = synthetic_paths(5000);
    // 再加一些非 ASCII 与重名的候选，走另一条数据路径
    paths.extend(
        [
            "Assets/角色/玩家Player.cs",
            "Docs/攻略/Enemy攻略.md",
            "a/Enemy.cs",
            "b/Enemy.cs",
        ]
        .map(String::from),
    );
    let idx = index(&paths.iter().map(String::as_str).collect::<Vec<_>>());
    let top = 40;
    for target in [
        "Enemy",
        "enemy spawner",
        "PlayerHealth",
        "Scripts/UI",
        "ui menu",
        "\"Menu\"",
        "角色",
        "e",
    ] {
        let mut search = Search::default();
        search.set_threads(Some(4));
        let mut typed = String::new();
        for ch in target.chars() {
            typed.push(ch);
            let got = search.run(&idx, &typed, &[], SearchOptions::default());
            let query = prepare_query(&typed);
            // 朴素实现：所有候选打分，重复选出最好的
            let mut scored: Vec<(u32, ItemScore)> = (0..idx.len() as u32)
                .map(|i| (i, idx.full_score(i, &query)))
                .filter(|(_, s)| s.score != 0)
                .collect();
            let mut want = Vec::new();
            while want.len() < top && !scored.is_empty() {
                let mut best = 0;
                for i in 1..scored.len() {
                    let (a, b) = (&scored[i], &scored[best]);
                    let (ta, tb) = (idx.text(a.0).unwrap(), idx.text(b.0).unwrap());
                    let less = compare_scored(
                        &Scored {
                            score: &a.1,
                            label: ta.label,
                            description: ta.description,
                            path: Some(ta.path),
                        },
                        &Scored {
                            score: &b.1,
                            label: tb.label,
                            description: tb.description,
                            path: Some(tb.path),
                        },
                        &query,
                    ) == std::cmp::Ordering::Less;
                    if less {
                        best = i;
                    }
                }
                want.push(scored.swap_remove(best).0);
            }
            let got: Vec<u32> = got.files.iter().take(top).map(|h| h.entry).collect();
            let name = |v: &[u32]| -> Vec<String> {
                v.iter()
                    .map(|i| idx.entry(*i).unwrap().path.as_str().to_owned())
                    .collect()
            };
            assert_eq!(
                name(&got),
                name(&want[..got.len().min(want.len())]),
                "查询 {typed:?}"
            );
            assert_eq!(got.len(), want.len().min(top), "查询 {typed:?}");
        }
    }
}

#[test]
fn results_are_capped_like_vscode() {
    let paths = synthetic_paths(2000);
    let idx = index(&paths.iter().map(String::as_str).collect::<Vec<_>>());
    let r = Search::default().run(&idx, "c", &[], SearchOptions::default());
    assert_eq!(r.files.len(), MAX_RESULTS);
}

#[test]
fn symbol_matcher_reports_utf8_ranges() {
    let m = FuzzyMatcher::default();
    let found = m.score("mv", "MoveForward").unwrap();
    assert_eq!(found.ranges, [0..1, 2..3]);
    let cjk = m.score("角色", "游戏角色控制").unwrap();
    assert_eq!(cjk.ranges, [6..12]);
    assert!(m.score("zz", "MoveForward").is_none());
    assert!(m.score("", "x").unwrap().ranges.is_empty());
    let _: Arc<dyn Matcher> = Arc::new(FuzzyMatcher::default());
}
