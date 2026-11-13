//! #118：tree-sitter 折叠查询产出的语法折叠区间。
//!
//! 约定：区间首行折叠后仍可见，末行（含）被隐藏，所以断言里写的是 `(首行, 末行 + 1)`，
//! 与 `Fold::lines` 以及 #12 的标记 / 缩进折叠同一口径。
mod support;

use futures_lite::future::block_on;
use rove_base::{ByteOffset, Cancellation, Line, Span, TextEdit, Version};
use rove_buffer::TextSource;
use rove_syntax::{
    highlight::{DocumentSyntax, TreeSitterSyntax},
    FoldKind, SyntaxService, SyntaxSnapshot,
};
use std::{sync::Arc, time::Duration};
use support::Text;

/// 调试构建里解析比 release 慢得多，刷新预算放宽，保证每次分析都推进基线而不是返回旧快照。
fn syntax() -> TreeSitterSyntax {
    TreeSitterSyntax::default().with_refresh_budget(Duration::from_secs(30))
}

/// 全量分析一段文本的折叠区间。
fn folds(language: &str, source: &str) -> Vec<(u64, u64)> {
    folds_with(&syntax(), language, source)
}

fn folds_with(service: &dyn SyntaxService, language: &str, source: &str) -> Vec<(u64, u64)> {
    let text: Arc<dyn TextSource> = Arc::new(Text::new(source, 1));
    let update = block_on(service.analyze(text, language.into(), Cancellation::default())).unwrap();
    pairs(update.snapshot.as_ref())
}

fn pairs(snapshot: &dyn SyntaxSnapshot) -> Vec<(u64, u64)> {
    snapshot
        .folds()
        .iter()
        .map(|fold| {
            assert!(matches!(fold.kind, FoldKind::Syntax));
            (fold.lines.start().0, fold.lines.end().0)
        })
        .collect()
}

/// 自检：区间按首行升序、同首行外层在前，至少隐藏一行，不超出行数，且两两要么嵌套要么不相交。
fn check_shape(folds: &[(u64, u64)], lines: u64) {
    for (index, &(start, end)) in folds.iter().enumerate() {
        assert!(end >= start + 2, "{start}..{end} 至少要隐藏一行");
        assert!(end <= lines, "{start}..{end} 超出 {lines} 行");
        for &(next_start, next_end) in &folds[index + 1..] {
            assert!(
                (start, next_end) <= (next_start, end) || next_start >= end,
                "{start}..{end} 与 {next_start}..{next_end} 既不嵌套也不相离"
            );
        }
    }
}

const CSHARP: &str = "\
using System;

namespace Demo
{
    /* 多行
       注释 */
    public class Widget
    {
        private int[] values = new[]
        {
            1, 2, 3,
        };

        public int Count
        {
            get { return values.Length; }
        }

        public void Resize(int size)
        {
            if (size > 0)
            {
                Array.Resize(ref values, size);
            }
        }

        public enum Mode
        {
            One,
            Two,
        }
    }
}
";

#[test]
fn csharp_types_members_blocks_and_comments_fold() {
    let got = folds("csharp", CSHARP);
    assert_eq!(
        got,
        [
            (3, 33),  // namespace 的体
            (4, 6),   // 多行注释
            (7, 32),  // class 的体
            (9, 12),  // 数组初始化器
            (14, 17), // 属性的 get / set
            (19, 25), // 方法体
            (21, 24), // if 的语句块
            (27, 31), // enum 的成员列表
        ]
    );
    check_shape(&got, 33);
}

#[test]
fn csharp_folds_the_same_lines_with_crlf_and_k_and_r_braces() {
    let crlf = CSHARP.replace('\n', "\r\n");
    assert_eq!(folds("csharp", &crlf), folds("csharp", CSHARP));

    // K&R：折叠箭头落在声明行本身。
    let knr = "class C {\n    void M() {\n        return;\n    }\n}\n";
    assert_eq!(folds("csharp", knr), [(0, 5), (1, 4)]);
}

#[test]
fn json_objects_and_arrays_fold() {
    let source = "\
{
  \"a\": [
    1,
    2
  ],
  \"b\": { \"c\": 1 },
  \"d\": {
    \"e\": true
  }
}
";
    let got = folds("json", source);
    assert_eq!(got, [(0, 10), (1, 5), (6, 9)], "单行的对象不折叠");
    check_shape(&got, 11);
    // jsonc 的跨行注释。
    assert_eq!(
        folds("jsonc", "{\n  /* a\n     b */\n  \"k\": 1\n}\n"),
        [(0, 5), (1, 3)]
    );
}

#[test]
fn markdown_sections_and_fenced_code_fold() {
    let source = "\
# Title

intro

## Section

```cs
class C {}
```

## Last
text
";
    let got = folds("markdown", source);
    assert_eq!(
        got,
        [(0, 12), (4, 9), (6, 9), (10, 12)],
        "分节末尾的空行不算进折叠"
    );
    check_shape(&got, 13);
}

#[test]
fn xml_elements_and_comments_fold() {
    let source = "\
<root>
  <item a=\"1\">
    <!-- note
         more -->
    <child />
  </item>
  <short>x</short>
</root>
";
    let got = folds("xml", source);
    assert_eq!(got, [(0, 8), (1, 6), (2, 4)]);
    check_shape(&got, 9);
}

#[test]
fn languages_without_a_fold_query_and_plain_text_have_none() {
    // YAML 是 off-side 语言，折叠交给 #12 的缩进折叠，语法层不产出区间。
    assert!(folds("yaml", "a:\n  b: 1\n  c: 2\n").is_empty());
    assert!(folds("plaintext", "a\n  b\n  c\n").is_empty());
    assert!(folds("csharp", "class C {}\n").is_empty());
    assert!(folds("csharp", "").is_empty());
}

/// 对同一文本做一次编辑，并与从零全量分析的结果逐条比较。
fn apply_edit(
    document: &DocumentSyntax,
    current: &mut String,
    version: &mut u64,
    at: usize,
    old_len: usize,
    inserted: &str,
) -> Vec<(u64, u64)> {
    let edit = TextEdit {
        version: Version(*version),
        range: Span::new(ByteOffset(at as u64), ByteOffset((at + old_len) as u64)).unwrap(),
        inserted: inserted.into(),
    };
    current.replace_range(at..at + old_len, inserted);
    *version += 1;
    let text: Arc<dyn TextSource> = Arc::new(Text::new(current.clone(), *version));
    let update = block_on(document.with_edit(edit).analyze(
        text,
        "csharp".into(),
        Cancellation::default(),
    ))
    .unwrap();
    assert_eq!(update.snapshot.version(), Version(*version), "基线已推进");
    let incremental = pairs(update.snapshot.as_ref());
    assert_eq!(
        incremental,
        folds("csharp", current),
        "增量更新后的折叠区间与全量分析不一致（版本 {version}）"
    );
    incremental
}

#[test]
fn incremental_edits_keep_the_same_ranges_as_a_full_parse() {
    let document = syntax().document();
    let mut current = CSHARP.to_string();
    let mut version = 1;
    let text: Arc<dyn TextSource> = Arc::new(Text::new(current.clone(), version));
    let first = block_on(document.analyze(text, "csharp".into(), Cancellation::default())).unwrap();
    assert_eq!(pairs(first.snapshot.as_ref()), folds("csharp", &current));

    // 1. 方法体里改一个字符：区间不变，外层区间不会因为复用而丢。
    let at = current.find("size > 0").unwrap();
    let before = apply_edit(&document, &mut current, &mut version, at + 7, 1, "1");
    assert_eq!(before.first(), Some(&(3, 33)));

    // 2. 在方法体里插入一整行：编辑点之后的区间整体下移。
    let line = "                Console.WriteLine(size);\n";
    let at = current.find("                Array.Resize").unwrap();
    let after = apply_edit(&document, &mut current, &mut version, at, 0, line);
    assert_eq!(after.last(), Some(&(28, 32)), "enum 的成员列表下移一行");

    // 3. 删除刚插入的那一行。
    apply_edit(&document, &mut current, &mut version, at, line.len(), "");

    // 4. 删掉一个右花括号，让结构出错，再补回来。
    let at = current
        .find("            }\n        }\n\n        public enum")
        .unwrap();
    apply_edit(&document, &mut current, &mut version, at + 12, 1, "");
    apply_edit(&document, &mut current, &mut version, at + 12, 0, "}");
    assert_eq!(folds("csharp", &current), folds("csharp", CSHARP));

    // 5. 在文件开头插入一行：全部区间下移。
    apply_edit(&document, &mut current, &mut version, 0, 0, "// head\n");

    // 6. 整段换掉：从 using 到 namespace 的体之前。
    let at = current.find("namespace Demo").unwrap();
    apply_edit(
        &document,
        &mut current,
        &mut version,
        at,
        14,
        "namespace Other",
    );
}

#[test]
fn a_visible_first_snapshot_still_carries_the_whole_document() {
    let document = syntax().document();
    let text: Arc<dyn TextSource> = Arc::new(Text::new(CSHARP, 1));
    let update = block_on(
        document
            .with_visible(rove_base::LineRange::new(Line(0), Line(4)).unwrap())
            .analyze(text.clone(), "csharp".into(), Cancellation::default()),
    )
    .unwrap();
    assert!(document.is_partial(), "只给可见行生成了 token");
    assert_eq!(pairs(update.snapshot.as_ref()), folds("csharp", CSHARP));

    // 补全同一文本时沿用已经算好的区间。
    let update =
        block_on(document.analyze(text, "csharp".into(), Cancellation::default())).unwrap();
    assert_eq!(pairs(update.snapshot.as_ref()), folds("csharp", CSHARP));
}

/// 超过分区阈值的 C#（#94 的分区并行解析）：每个分区的树都带着头尾，把全部成员括起来的类体在
/// 每个分区里都在，去重后结果必须与整文件解析一致，增量只重解析一个分区时也一样。
#[test]
fn partitioned_csharp_folds_match_a_whole_file_parse() {
    let mut source = String::from("namespace Demo\n{\n");
    for index in 0..1_400 {
        source.push_str(&format!(
            "    public sealed class Mover{index} : MonoBehaviour\n    {{\n        \
             [SerializeField] private float speed = 1f;\n\n        private void Update()\n        \
             {{\n            var step = speed * Time.deltaTime; // move {index}\n            \
             transform.Translate(0f, step, 0f);\n        }}\n    }}\n\n"
        ));
    }
    source.push_str("}\n");
    assert!(source.len() > 256 * 1024);

    let partitioned = syntax().with_parallelism(4);
    let whole = syntax().with_parallelism(0);
    let expected = folds_with(&whole, "csharp", &source);
    assert_eq!(folds_with(&partitioned, "csharp", &source), expected);
    check_shape(&expected, source.split('\n').count() as u64);

    // 增量：改一个成员体，只重解析它所在的分区。
    let document = partitioned.document();
    let mut current = source;
    let mut version = 1;
    let text: Arc<dyn TextSource> = Arc::new(Text::new(current.clone(), version));
    block_on(document.analyze(text, "csharp".into(), Cancellation::default())).unwrap();
    let at = current
        .find("speed * Time.deltaTime; // move 1200")
        .unwrap();
    apply_edit(&document, &mut current, &mut version, at, 0, "2f * ");
    let at = current
        .find("            transform.Translate(0f, step, 0f);")
        .unwrap();
    apply_edit(
        &document,
        &mut current,
        &mut version,
        at,
        0,
        "            // note\n",
    );
}

/// 真实 Unity 项目里的 C# 文件：只检查形状与增量一致性，不写死路径（AGENTS §8）。
#[test]
fn real_unity_csharp_files_produce_nested_ranges() {
    let project =
        rove_test_support::require_unity_project!(rove_test_support::UnityProject::Medium);
    let mut checked = 0;
    for entry in walk(&project) {
        let Ok(source) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if source.len() < 2_000 || !source.contains("class ") {
            continue;
        }
        let lines = source.split('\n').count() as u64;
        let got = folds("csharp", &source);
        assert!(!got.is_empty(), "{} 没有折叠区间", entry.display());
        check_shape(&got, lines);
        checked += 1;
        if checked == 20 {
            break;
        }
    }
    assert!(checked > 0, "测试项目里没有找到 C# 文件");
}

/// 真实 C# 文件上的随机编辑：每次编辑后增量结果都要与全量分析逐条相同。
#[test]
fn real_unity_csharp_random_edits_match_a_full_parse() {
    let project =
        rove_test_support::require_unity_project!(rove_test_support::UnityProject::Medium);
    let Some(path) = walk(&project).into_iter().find(|path| {
        std::fs::metadata(path).is_ok_and(|meta| (8_000..40_000).contains(&meta.len()))
    }) else {
        eprintln!("已跳过：测试项目里没有合适大小的 C# 文件");
        return;
    };
    let mut current = std::fs::read_to_string(&path).unwrap();
    let document = syntax().document();
    let mut version = 1;
    let text: Arc<dyn TextSource> = Arc::new(Text::new(current.clone(), version));
    block_on(document.analyze(text, "csharp".into(), Cancellation::default())).unwrap();

    // 固定种子的线性同余，复现同一串编辑。
    let mut seed = 0x5eed_1234_u64;
    let mut random = move |bound: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % bound.max(1)
    };
    for step in 0..150 {
        let mut at = random(current.len());
        while at > 0 && !current.is_char_boundary(at) {
            at -= 1;
        }
        let (old_len, inserted) = match step % 5 {
            0 => (0, "{"),
            1 => (0, "\n    int Added() { return 0; }\n"),
            2 => (0, "// x"),
            3 => (current[at..].chars().next().map_or(0, char::len_utf8), ""),
            _ => (current[at..].find('\n').map_or(0, |end| end + 1), ""),
        };
        apply_edit(&document, &mut current, &mut version, at, old_len, inserted);
    }
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.join("Assets")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "cs") {
                found.push(path);
            }
        }
        if found.len() > 200 {
            break;
        }
    }
    found.sort();
    found
}
