//! 测试与基准辅助：写出带符号、出现位置、关系与文档的 RVCSIDX v2 索引（格式见
//! `docs/CS_INDEX_FORMAT.md`），用真实读取端打开。不用于生产路径。
//!
//! 文件按给定顺序编号，写出时要求路径已按 UTF-8 字节序排好（与索引进程一致）。
use crate::lifecycle::validity::EXPECTED_INDEXER_VERSION;
use std::{fs, io, path::Path};

/// 一个符号。`local = Some((file, byte_offset))` 时写成局部符号键。
#[derive(Clone, Debug)]
pub struct TestSymbol {
    pub key: String,
    pub name: String,
    pub display: String,
    pub container: Option<u32>,
    /// Roslyn `SymbolKind`（9 方法、11 类型、6 字段、8 局部变量、13 参数、15 属性、17 类型参数）。
    pub kind: u8,
    pub local: Option<(u32, u32)>,
}

/// 一个出现位置（0 起算的行、UTF-8 字节列）。
#[derive(Clone, Copy, Debug)]
pub struct TestOccurrence {
    pub file: u32,
    pub line: u32,
    pub col: u32,
    pub len: u32,
    pub symbol: u32,
    pub definition: bool,
}

/// 索引内容。
#[derive(Clone, Debug, Default)]
pub struct TestIndex {
    pub config_hash: [u8; 32],
    /// 项目相对路径与内容哈希。
    pub files: Vec<(String, u64)>,
    pub symbols: Vec<TestSymbol>,
    pub occurrences: Vec<TestOccurrence>,
    /// (from, to, kind)：kind 1 基类型/接口，2 实现接口成员，3 重写。
    pub relations: Vec<(u32, u32, u32)>,
    pub signatures: Vec<(u32, String)>,
    pub docs: Vec<(u32, String)>,
    /// 每个文件的不生效行区间。
    pub inactive: Vec<(u32, u32, u32)>,
    /// 元数据视图（#34 段 13）。
    pub views: Vec<TestView>,
}

/// 一份元数据视图：外部类型、正文与成员名字位置（symbol, line, col, len）。
#[derive(Clone, Debug)]
pub struct TestView {
    pub type_symbol: u32,
    pub text: String,
    pub members: Vec<(u32, u32, u32, u32)>,
}

pub const KIND_FIELD: u8 = 6;
pub const KIND_LOCAL: u8 = 8;
pub const KIND_METHOD: u8 = 9;
pub const KIND_TYPE: u8 = 11;
pub const KIND_PARAMETER: u8 = 13;
pub const KIND_PROPERTY: u8 = 15;
pub const KIND_TYPE_PARAMETER: u8 = 17;

impl TestIndex {
    pub fn file(&mut self, path: &str, hash: u64) -> u32 {
        self.files.push((path.to_owned(), hash));
        self.files.len() as u32 - 1
    }

    pub fn symbol(&mut self, key: &str, name: &str, kind: u8, container: Option<u32>) -> u32 {
        self.symbols.push(TestSymbol {
            key: key.to_owned(),
            name: name.to_owned(),
            display: name.to_owned(),
            container,
            kind,
            local: None,
        });
        self.symbols.len() as u32 - 1
    }

    pub fn local(&mut self, name: &str, kind: u8, file: u32, offset: u32) -> u32 {
        let id = self.symbol("", name, kind, None);
        self.symbols[id as usize].local = Some((file, offset));
        id
    }

    pub fn def(&mut self, symbol: u32, file: u32, line: u32, col: u32) {
        self.occurrence(symbol, file, line, col, true);
    }

    pub fn reference(&mut self, symbol: u32, file: u32, line: u32, col: u32) {
        self.occurrence(symbol, file, line, col, false);
    }

    fn occurrence(&mut self, symbol: u32, file: u32, line: u32, col: u32, definition: bool) {
        let len = self.symbols[symbol as usize].name.len() as u32;
        self.occurrences.push(TestOccurrence {
            file,
            line,
            col,
            len,
            symbol,
            definition,
        });
    }

    pub fn bytes(&self) -> Vec<u8> {
        assert!(
            self.files
                .windows(2)
                .all(|w| w[0].0.as_bytes() < w[1].0.as_bytes()),
            "paths must be sorted"
        );
        let mut strings = Strings::default();
        // 出现位置按 (文件, 行, 列) 排序，再按符号重新收集。
        let mut occ = self.occurrences.clone();
        occ.sort_by_key(|o| (o.file, o.line, o.col));
        let mut wide = Vec::<u8>::new();
        let mut packed = Vec::<u8>::new();
        for o in &occ {
            let role = if o.definition { 2u64 } else { 1 };
            let fits = o.line < 4096 && o.col < 256 && o.len < 32 && o.symbol < (1 << 21);
            let word = if fits {
                role | u64::from(o.len) << 3
                    | u64::from(o.col) << 8
                    | u64::from(o.line) << 16
                    | u64::from(o.symbol) << 28
                    | u64::from(o.file) << 49
            } else {
                let i = (wide.len() / 24) as u64;
                for v in [o.file, o.line, o.col, o.symbol, o.len, role as u32] {
                    wide.extend_from_slice(&v.to_le_bytes());
                }
                i << 3
            };
            packed.extend_from_slice(&word.to_le_bytes());
        }
        // 文件表。
        let mut paths = Vec::new();
        let mut files = Vec::new();
        let mut inactive = Vec::new();
        let mut occ_start = 0u32;
        let mut per_file = vec![0u32; self.files.len()];
        let mut by_symbol = vec![Vec::new(); self.symbols.len()];
        for (i, o) in occ.iter().enumerate() {
            per_file[o.file as usize] += 1;
            by_symbol[o.symbol as usize].push((i, o.definition));
        }
        for (i, (path, hash)) in self.files.iter().enumerate() {
            let count = per_file[i];
            let inactive_start = (inactive.len() / 8) as u32;
            for &(f, a, b) in &self.inactive {
                if f == i as u32 {
                    inactive.extend_from_slice(&a.to_le_bytes());
                    inactive.extend_from_slice(&b.to_le_bytes());
                }
            }
            for v in [paths.len() as u32, occ_start, count, inactive_start] {
                files.extend_from_slice(&v.to_le_bytes());
            }
            files.extend_from_slice(&hash.to_le_bytes());
            paths.extend_from_slice(path.as_bytes());
            occ_start += count;
        }
        // 符号与按符号的出现位置索引（定义在前，各组内按出现位置序号升序；每项 3 字节）。
        let mut symbols = Vec::new();
        let mut refs = Vec::new();
        let mut local_keys = Vec::new();
        for (sid, s) in self.symbols.iter().enumerate() {
            let mine = &by_symbol[sid];
            let defs: Vec<usize> = mine.iter().filter(|m| m.1).map(|m| m.0).collect();
            let uses: Vec<usize> = mine.iter().filter(|m| !m.1).map(|m| m.0).collect();
            let ref_start = (refs.len() / 3) as u32;
            for i in defs.iter().chain(&uses) {
                refs.extend_from_slice(&(*i as u32).to_le_bytes()[..3]);
            }
            let key = match s.local {
                Some((file, offset)) => {
                    let i = (local_keys.len() / 8) as u32;
                    local_keys.extend_from_slice(&file.to_le_bytes());
                    local_keys.extend_from_slice(&offset.to_le_bytes());
                    i | 1 << 31
                }
                None => strings.add(&s.key),
            };
            let name = strings.add(&s.name);
            let display = strings.add(&s.display);
            for v in [
                key,
                name,
                display,
                s.container.unwrap_or(u32::MAX),
                ref_start,
            ] {
                symbols.extend_from_slice(&v.to_le_bytes());
            }
            symbols.push(s.kind);
            symbols.push(1);
            symbols.extend_from_slice(&(defs.len() as u16).to_le_bytes());
        }
        let relation_bytes = |mut rels: Vec<(u32, u32, u32)>| {
            rels.sort();
            rels.iter()
                .flat_map(|&(a, b, k)| [a, b, k])
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<u8>>()
        };
        let from = relation_bytes(self.relations.clone());
        let to = relation_bytes(self.relations.iter().map(|&(f, t, k)| (t, f, k)).collect());
        // 段 9 的记录仍是 (from, to, kind)，只是按 (to, from) 排序。
        let to: Vec<u8> = to
            .chunks_exact(12)
            .flat_map(|c| {
                let mut r = [0u8; 12];
                r[..4].copy_from_slice(&c[4..8]);
                r[4..8].copy_from_slice(&c[..4]);
                r[8..].copy_from_slice(&c[8..]);
                r
            })
            .collect();
        let mut texts = |list: &[(u32, String)]| {
            let mut list = list.to_vec();
            list.sort_by_key(|e| e.0);
            list.iter()
                .flat_map(|(s, t)| [*s, strings.add(t)])
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<u8>>()
        };
        let signatures = texts(&self.signatures);
        let docs = texts(&self.docs);
        let metadata = {
            let mut views = self.views.clone();
            views.sort_by_key(|v| v.type_symbol);
            let mut records = Vec::new();
            let mut members = Vec::new();
            for v in &views {
                let start = (members.len() / 16) as u32;
                let mut m = v.members.clone();
                m.sort();
                for (symbol, line, col, len) in &m {
                    for x in [*symbol, *line, *col, *len] {
                        members.extend_from_slice(&x.to_le_bytes());
                    }
                }
                for x in [v.type_symbol, strings.add(&v.text), start, m.len() as u32] {
                    records.extend_from_slice(&x.to_le_bytes());
                }
            }
            if views.is_empty() {
                Vec::new()
            } else {
                let mut out = Vec::new();
                out.extend_from_slice(&(views.len() as u32).to_le_bytes());
                out.extend_from_slice(&((members.len() / 16) as u32).to_le_bytes());
                out.extend_from_slice(&records);
                out.extend_from_slice(&members);
                out
            }
        };
        let (offsets, payload) = strings.finish();
        let sections: [(&[u8], usize); 15] = [
            (&offsets, 4),
            (&files, 24),
            (&paths, 1),
            (&symbols, 24),
            (&local_keys, 8),
            (&packed, 8),
            (&wide, 24),
            (&refs, 1),
            (&from, 12),
            (&to, 12),
            (&inactive, 8),
            (&signatures, 8),
            (&docs, 8),
            (&metadata, 1),
            (&payload, 1),
        ];
        encode(sections, self.config_hash)
    }

    pub fn write(&self, path: &Path) -> io::Result<()> {
        let temp = path.with_extension("testindex.tmp");
        fs::write(&temp, self.bytes())?;
        fs::rename(&temp, path)
    }
}

#[derive(Default)]
struct Strings {
    offsets: Vec<u32>,
    payload: Vec<u8>,
    ids: std::collections::HashMap<String, u32>,
}

impl Strings {
    fn add(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        if self.offsets.is_empty() {
            self.offsets.push(0);
        }
        self.payload.extend_from_slice(s.as_bytes());
        self.offsets.push(self.payload.len() as u32);
        let id = self.offsets.len() as u32 - 2;
        self.ids.insert(s.to_owned(), id);
        id
    }

    fn finish(mut self) -> (Vec<u8>, Vec<u8>) {
        if self.offsets.is_empty() {
            self.offsets.push(0);
        }
        let offsets = self.offsets.iter().flat_map(|o| o.to_le_bytes()).collect();
        (offsets, self.payload)
    }
}

fn encode(sections: [(&[u8], usize); 15], config_hash: [u8; 32]) -> Vec<u8> {
    const HEADER: usize = 96 + 15 * 16;
    let mut out = vec![0u8; HEADER];
    let mut table = Vec::new();
    for (data, size) in sections {
        while !out.len().is_multiple_of(8) {
            out.push(0);
        }
        table.push((out.len() as u64, (data.len() / size) as u64));
        out.extend_from_slice(data);
    }
    while !out.len().is_multiple_of(8) {
        out.push(0);
    }
    let total = out.len() as u64;
    out[..8].copy_from_slice(b"RVCSIDX\0");
    out[8..12].copy_from_slice(&2u32.to_le_bytes());
    out[12..16].copy_from_slice(&15u32.to_le_bytes());
    out[16..24].copy_from_slice(&total.to_le_bytes());
    for (i, v) in EXPECTED_INDEXER_VERSION.iter().enumerate() {
        out[24 + i * 2..26 + i * 2].copy_from_slice(&v.to_le_bytes());
    }
    out[32..64].copy_from_slice(&config_hash);
    for (i, (offset, count)) in table.iter().enumerate() {
        let at = 96 + i * 16;
        out[at..at + 8].copy_from_slice(&offset.to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&count.to_le_bytes());
    }
    let checksum = out[..88]
        .iter()
        .chain(&out[96..HEADER])
        .fold(14695981039346656037u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(1099511628211)
        });
    out[88..96].copy_from_slice(&checksum.to_le_bytes());
    out
}

/// 示例项目：两个文件、继承、重写外部方法、局部变量与 lambda 参数。
pub const PLAYER_CS: &str = "namespace Game
{
    /// <summary>A player.</summary>
    public class Player
    {
        public int Health;

        public void Move(int speed)
        {
            var step = speed * 2;
            Health -= step;
        }
    }
}
";

pub const HERO_CS: &str = "namespace Game
{
    public class Hero : Player
    {
        public override string ToString() => \"hero\";

        void Act()
        {
            Move(1);
            Health = 3;
            Run(x => x + 1);
        }
    }
}
";

pub const PLAYER_PATH: &str = "Assets/Player.cs";
pub const HERO_PATH: &str = "Assets/Hero.cs";

/// 第 `nth` 个完整单词 `word` 的位置（0 起算的行、UTF-8 字节列）。
pub fn find_word(text: &str, word: &str, nth: usize) -> (u32, u32) {
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut seen = 0;
    for (i, _) in text.match_indices(word) {
        let before = i.checked_sub(1).map(|j| bytes[j]);
        let after = bytes.get(i + word.len()).copied();
        if before.is_some_and(is_ident) || after.is_some_and(is_ident) {
            continue;
        }
        if seen == nth {
            let line = text[..i].matches('\n').count() as u32;
            let col = text[..i].rfind('\n').map_or(i, |n| i - n - 1) as u32;
            return (line, col);
        }
        seen += 1;
    }
    panic!("word {word:?} #{nth} not found");
}

/// 示例项目的符号 ID。
#[derive(Clone, Copy, Debug)]
pub struct SampleSymbols {
    pub player: u32,
    pub health: u32,
    pub move_: u32,
    pub hero: u32,
    pub hero_to_string: u32,
    pub object_to_string: u32,
    pub speed: u32,
    pub step: u32,
    pub x: u32,
}

/// 按两份文本的当前内容生成示例项目的索引（位置靠搜索单词得到，文本改了也能“重建”）。
pub fn sample_index(player: &str, hero: &str) -> (TestIndex, SampleSymbols) {
    use super::buffers::hash_bytes;
    let mut t = TestIndex::default();
    let h = t.file(HERO_PATH, hash_bytes(hero.as_bytes()));
    let p = t.file(PLAYER_PATH, hash_bytes(player.as_bytes()));
    let at = |text: &str, word: &str, nth: usize| find_word(text, word, nth);
    let s = SampleSymbols {
        player: t.symbol("Game|T:Game.Player", "Player", KIND_TYPE, None),
        health: t.symbol("Game|F:Game.Player.Health", "Health", KIND_FIELD, None),
        move_: t.symbol(
            "Game|M:Game.Player.Move(System.Int32)",
            "Move",
            KIND_METHOD,
            None,
        ),
        hero: t.symbol("Game|T:Game.Hero", "Hero", KIND_TYPE, None),
        hero_to_string: t.symbol("Game|M:Game.Hero.ToString", "ToString", KIND_METHOD, None),
        object_to_string: t.symbol(
            "mscorlib|M:System.Object.ToString",
            "ToString",
            KIND_METHOD,
            None,
        ),
        speed: t.local("speed", KIND_PARAMETER, p, 0),
        step: t.local("step", KIND_LOCAL, p, 1),
        x: t.local("x", KIND_PARAMETER, h, 2),
    };
    for (sym, file, text, word, defs, refs) in [
        (s.player, p, player, "Player", 1, 0),
        (s.health, p, player, "Health", 1, 1),
        (s.move_, p, player, "Move", 1, 0),
        (s.speed, p, player, "speed", 1, 1),
        (s.step, p, player, "step", 1, 1),
        (s.hero, h, hero, "Hero", 1, 0),
        (s.player, h, hero, "Player", 0, 1),
        (s.hero_to_string, h, hero, "ToString", 1, 0),
        (s.move_, h, hero, "Move", 0, 1),
        (s.health, h, hero, "Health", 0, 1),
        (s.x, h, hero, "x", 1, 1),
    ] {
        for nth in 0..defs + refs {
            let (line, col) = at(text, word, nth);
            if nth < defs {
                t.def(sym, file, line, col);
            } else {
                t.reference(sym, file, line, col);
            }
        }
    }
    t.symbols[s.hero_to_string as usize].container = Some(s.hero);
    t.relations.push((s.hero, s.player, 1));
    t.relations.push((s.hero_to_string, s.object_to_string, 3));
    t.docs.push((s.player, "A player.".into()));
    t.signatures.push((
        s.object_to_string,
        "public virtual string ToString()".into(),
    ));
    (t, s)
}

/// #42 的示例：一个源码文件使用外部 API，三个外部类型各有一份元数据视图。
pub const GAME_PATH: &str = "Assets/Game.cs";
pub const GAME_CS: &str = "namespace Game
{
    public class Player
    {
        public void Update()
        {
            var p = transform.position;
            transform.position = p;
            var q = transform.position;
            var parent = transform.parent;
            list.Add(1);
            transform.Translate(p);
        }
    }
}
";

pub const TRANSFORM_KEY: &str = "UnityEngine.CoreModule|T:UnityEngine.Transform";
pub const VECTOR3_KEY: &str = "UnityEngine.CoreModule|T:UnityEngine.Vector3";
pub const POSITION_KEY: &str = "UnityEngine.CoreModule|P:UnityEngine.Transform.position";
pub const PARENT_KEY: &str = "UnityEngine.CoreModule|P:UnityEngine.Transform.parent";
pub const TRANSLATE_KEY: &str =
    "UnityEngine.CoreModule|M:UnityEngine.Transform.Translate(UnityEngine.Vector3)";
pub const LIST_KEY: &str = "mscorlib|T:System.Collections.Generic.List`1";
pub const ENUMERATOR_KEY: &str = "mscorlib|T:System.Collections.Generic.List`1.Enumerator";
pub const ADD_KEY: &str = "mscorlib|M:System.Collections.Generic.List`1.Add(`0)";

pub const TRANSFORM_VIEW: &str =
    "#region 程序集 UnityEngine.CoreModule, Version=0.0.0.0, Culture=neutral, PublicKeyToken=null
// C:/Unity/Managed/UnityEngine.CoreModule.dll
#endregion

namespace UnityEngine;

/// <summary>Position, rotation and scale of an object.</summary>
public class Transform : Component
{
    /// <summary>The world space position of the Transform.</summary>
    public Vector3 position { get; set; }
    /// <summary>The parent of the transform.</summary>
    public Transform parent { get; set; }
    public Player Owner { get; }
    public void Translate(Vector3 translation);
}
";

pub const VECTOR3_VIEW: &str =
    "#region 程序集 UnityEngine.CoreModule, Version=0.0.0.0, Culture=neutral, PublicKeyToken=null
// C:/Unity/Managed/UnityEngine.CoreModule.dll
#endregion

namespace UnityEngine;

public struct Vector3
{
    public float x;
}
";

pub const LIST_VIEW: &str =
    "#region 程序集 mscorlib, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089
// C:/Windows/Microsoft.NET/Framework64/v4.0.30319/mscorlib.dll
#endregion

namespace System.Collections.Generic;

public class List<T>
{
    public void Add(T item);
    public struct Enumerator
    {
    }
}
";

const NL: char = '\n';

/// 元数据示例的符号。
#[derive(Clone, Copy, Debug)]
pub struct MetadataSample {
    pub player: u32,
    pub translate: u32,
    pub parent: u32,
    pub position: u32,
    pub transform: u32,
    pub vector3: u32,
    pub add: u32,
    pub list: u32,
    pub enumerator: u32,
}

/// 元数据示例索引。符号按键的字节序排列（`find_symbol` 二分查找的前提），所以不能叠在 `sample_index` 上。
/// `game` 是 [`GAME_CS`] 的当前内容。
pub fn metadata_sample_index(game: &str) -> (TestIndex, MetadataSample) {
    use super::buffers::hash_bytes;
    let mut t = TestIndex::default();
    let file = t.file(GAME_PATH, hash_bytes(game.as_bytes()));
    let s = MetadataSample {
        player: t.symbol("Game|T:Game.Player", "Player", KIND_TYPE, None),
        translate: t.symbol(TRANSLATE_KEY, "Translate", KIND_METHOD, None),
        parent: t.symbol(PARENT_KEY, "parent", KIND_PROPERTY, None),
        position: t.symbol(POSITION_KEY, "position", KIND_PROPERTY, None),
        transform: t.symbol(TRANSFORM_KEY, "Transform", KIND_TYPE, None),
        vector3: t.symbol(VECTOR3_KEY, "Vector3", KIND_TYPE, None),
        add: t.symbol(ADD_KEY, "Add", KIND_METHOD, None),
        list: t.symbol(LIST_KEY, "List", KIND_TYPE, None),
        enumerator: t.symbol(ENUMERATOR_KEY, "Enumerator", KIND_TYPE, None),
    };
    t.symbols[s.parent as usize].container = Some(s.transform);
    t.symbols[s.translate as usize].container = Some(s.transform);
    t.symbols[s.position as usize].container = Some(s.transform);
    t.symbols[s.add as usize].container = Some(s.list);
    t.symbols[s.enumerator as usize].container = Some(s.list);
    let (line, col) = find_word(game, "Player", 0);
    t.def(s.player, file, line, col);
    for nth in 0..3 {
        let (line, col) = find_word(game, "position", nth);
        t.reference(s.position, file, line, col);
    }
    let (line, col) = find_word(game, "parent", 1);
    t.reference(s.parent, file, line, col);
    let (line, col) = find_word(game, "Add", 0);
    t.reference(s.add, file, line, col);
    let (line, col) = find_word(game, "Translate", 0);
    t.reference(s.translate, file, line, col);
    // 成员名字的位置：先找声明里的片段（避开文档注释里的同名单词），再在片段里找名字。
    let members = |text: &str, names: &[(u32, &str, &str)]| -> Vec<(u32, u32, u32, u32)> {
        names
            .iter()
            .map(|(sym, anchor, name)| {
                let at = text.find(anchor).expect("anchor") + anchor.find(name).unwrap();
                let line = text[..at].matches(NL).count() as u32;
                let col = text[..at].rfind(NL).map_or(at, |n| at - n - 1) as u32;
                (*sym, line, col, name.len() as u32)
            })
            .collect()
    };
    t.views.push(TestView {
        type_symbol: s.transform,
        text: TRANSFORM_VIEW.into(),
        members: members(
            TRANSFORM_VIEW,
            &[
                (s.transform, "class Transform", "Transform"),
                (s.position, "Vector3 position", "position"),
                (s.parent, "Transform parent", "parent"),
                (s.translate, "void Translate", "Translate"),
            ],
        ),
    });
    t.views.push(TestView {
        type_symbol: s.vector3,
        text: VECTOR3_VIEW.into(),
        members: members(VECTOR3_VIEW, &[(s.vector3, "struct Vector3", "Vector3")]),
    });
    t.views.push(TestView {
        type_symbol: s.list,
        text: LIST_VIEW.into(),
        members: members(
            LIST_VIEW,
            &[
                (s.list, "class List", "List"),
                (s.add, "void Add", "Add"),
                (s.enumerator, "struct Enumerator", "Enumerator"),
            ],
        ),
    });
    (t, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CsIndex;
    use rove_base::Utf8Column;

    #[test]
    fn written_index_answers_like_the_indexer_output() {
        let mut t = TestIndex::default();
        let a = t.file("Assets/A.cs", 1);
        let b = t.file("Assets/B.cs", 2);
        let ty = t.symbol("Game|T:Game.Player", "Player", KIND_TYPE, None);
        let m = t.symbol("Game|M:Game.Player.Move", "Move", KIND_METHOD, Some(ty));
        let derived = t.symbol("Game|T:Game.Hero", "Hero", KIND_TYPE, None);
        t.def(ty, a, 1, 6);
        t.def(m, a, 3, 9);
        t.reference(m, b, 5000, 300); // 宽记录
        t.reference(ty, b, 2, 4);
        t.def(derived, b, 0, 6);
        t.relations.push((derived, ty, 1));
        t.docs.push((m, "Moves.".into()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.idx");
        t.write(&path).unwrap();
        let index = CsIndex::open(&path).unwrap();
        let defs = index.goto_definition("Assets/B.cs", 5000, Utf8Column(302));
        assert_eq!(defs.len(), 1);
        assert_eq!(
            (defs[0].path, defs[0].line, defs[0].col.0),
            ("Assets/A.cs", 3, 9)
        );
        let refs = index.find_references("Assets/A.cs", 1, Utf8Column(6));
        assert_eq!(refs.len(), 2);
        assert_eq!(
            index
                .implementations(ty)
                .map(|r| r.from)
                .collect::<Vec<_>>(),
            [derived]
        );
        assert_eq!(index.xml_docs()[0].symbol, m);
        assert_eq!(index.str(index.xml_docs()[0].text).unwrap(), "Moves.");
        assert_eq!(index.files()[1].hash, 2);
    }
}
