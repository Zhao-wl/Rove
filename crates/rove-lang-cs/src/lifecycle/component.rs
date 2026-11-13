//! 索引组件的安装位置、发现规则与版本兼容判定（#43）。
//!
//! 组件（`tools/rove-indexer-cs` 的 self-contained 发布产物）是可选的、与主程序分开安装的独立包。
//! Rove 运行期**不下载**它：用户自己安装（解压到下面的目录）。规则与 #76 的安装包约定一致，
//! 完整说明见 `docs/CS_INDEX_FORMAT.md`「组件、协议与索引格式的版本」。
//!
//! # 布局与查找顺序
//!
//! ```text
//! %LOCALAPPDATA%\Rove\components\indexer-cs\<版本>\     用户级安装（默认；ROVE_COMPONENTS_DIR 可替换 components 目录）
//!     rove-indexer-cs.exe  component.json  …运行时与依赖…
//! <rove.exe 所在目录>\components\indexer-cs\<版本>\      便携版 / 随包放置，布局相同
//! ```
//!
//! 1. 环境变量 `ROVE_INDEXER_CS`（开发覆盖：指向 `rove-indexer-cs.exe` 或 `.dll`）：设置了就只用它，不看清单，
//!    握手仍在运行时进行；
//! 2. 上面两个目录里每个**版本子目录**都是一个候选，要求同时有 `rove-indexer-cs.exe` 与 `component.json`；
//! 3. 候选按清单判定兼容性（[`check`]）；**选版本号最高的兼容候选**，版本相同时用户目录优先；
//! 4. 有候选但没有一个兼容 → [`Discovery::Incompatible`]（提示更新组件或更新 Rove，降级到 L1/L0）；
//!    没有候选 → [`Discovery::NotInstalled`]。
//!
//! 多个版本可以并存：安装新版本只是多放一个版本目录，旧版本可以之后再删。
use super::process::{IndexerComponent, INDEXER_ENV};
use super::resident::{PROTOCOL, PROTOCOL_MIN};
use super::validity::EXPECTED_FORMAT;
use serde_json::Value;
use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
};

/// 组件在 `components` 目录下的子目录名。
pub const COMPONENT_DIR: &str = "indexer-cs";
/// 版本目录里的清单文件（`rove-indexer-cs --version` 的输出）。
pub const MANIFEST_FILE: &str = "component.json";
/// 版本目录里的可执行文件。
pub const EXE_FILE: &str = "rove-indexer-cs.exe";
/// 环境变量：替换 `%LOCALAPPDATA%\Rove\components`（测试与自带组件的开发环境用）。
pub const COMPONENTS_DIR_ENV: &str = "ROVE_COMPONENTS_DIR";

/// 清单（`component.json`）的内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: [u16; 4],
    /// 组件能说的最高 / 最低协议版本。
    pub protocol: u32,
    pub protocol_min: u32,
    /// 组件写出的索引格式版本。
    pub index_format: u32,
}

/// 把 `1.2.0` 或 `1.2.0.0` 解析成四段版本。
pub fn parse_version(text: &str) -> Option<[u16; 4]> {
    let mut out = [0u16; 4];
    let mut count = 0;
    for part in text.trim().split('.') {
        *out.get_mut(count)? = part.parse().ok()?;
        count += 1;
    }
    (count >= 2).then_some(out)
}

pub fn version_text(v: [u16; 4]) -> String {
    format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3])
}

impl Manifest {
    pub fn parse(text: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(text).ok()?;
        if v["component"].as_str() != Some(COMPONENT_DIR) {
            return None;
        }
        let protocol = u32::try_from(v["protocol"].as_u64()?).ok()?;
        Some(Self {
            version: parse_version(v["version"].as_str()?)?,
            protocol,
            protocol_min: v["protocol_min"]
                .as_u64()
                .and_then(|p| u32::try_from(p).ok())
                .unwrap_or(protocol),
            index_format: u32::try_from(v["index_format"].as_u64()?).ok()?,
        })
    }
}

/// 不兼容的方向：决定提示用户更新哪一边。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// 组件比 Rove 旧：更新组件。
    ComponentTooOld,
    /// 组件比 Rove 新：更新 Rove。
    ComponentTooNew,
}

/// 哪一项对不上。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aspect {
    Protocol,
    IndexFormat,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incompatible {
    pub direction: Direction,
    pub aspect: Aspect,
    /// 组件版本（清单或 hello 里有时才有）。
    pub component_version: Option<String>,
    /// 日志用的诊断文字，例如 `protocol 3..4, Rove speaks 1..1`。
    pub detail: String,
}

impl fmt::Display for Incompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let who = match self.direction {
            Direction::ComponentTooOld => "component too old",
            Direction::ComponentTooNew => "component too new",
        };
        write!(f, "{who}: {}", self.detail)?;
        if let Some(v) = &self.component_version {
            write!(f, " (component {v})")?;
        }
        Ok(())
    }
}

/// 协商协议版本：Rove 支持 `PROTOCOL_MIN..=PROTOCOL`，组件支持 `protocol_min..=protocol`，
/// 取两个区间交集里最高的一个；交集为空时按方向报告。这条规则对清单与运行时的 `hello` 通用。
pub fn negotiate_protocol(protocol: u32, protocol_min: u32) -> Result<u32, Incompatible> {
    let chosen = protocol.min(PROTOCOL);
    if chosen >= protocol_min.max(PROTOCOL_MIN) {
        return Ok(chosen);
    }
    let direction = if protocol < PROTOCOL_MIN {
        Direction::ComponentTooOld
    } else {
        Direction::ComponentTooNew
    };
    Err(Incompatible {
        direction,
        aspect: Aspect::Protocol,
        component_version: None,
        detail: format!(
            "protocol {protocol_min}..{protocol}, Rove speaks {PROTOCOL_MIN}..{PROTOCOL}"
        ),
    })
}

/// 清单是否与本 Rove 兼容；兼容时返回协商出的协议版本。
pub fn check(manifest: &Manifest) -> Result<u32, Incompatible> {
    let with_version = |mut e: Incompatible| {
        e.component_version = Some(version_text(manifest.version));
        e
    };
    if manifest.index_format != EXPECTED_FORMAT {
        return Err(with_version(Incompatible {
            direction: if manifest.index_format < EXPECTED_FORMAT {
                Direction::ComponentTooOld
            } else {
                Direction::ComponentTooNew
            },
            aspect: Aspect::IndexFormat,
            component_version: None,
            detail: format!(
                "index format {}, Rove reads {EXPECTED_FORMAT}",
                manifest.index_format
            ),
        }));
    }
    negotiate_protocol(manifest.protocol, manifest.protocol_min).map_err(with_version)
}

/// 查找结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Discovery {
    Found(IndexerComponent),
    NotInstalled,
    Incompatible(Incompatible),
}

/// 查找位置：环境变量覆盖与按优先级排列的组件根目录（`…\indexer-cs`）。
#[derive(Clone, Debug, Default)]
pub struct Roots {
    pub env_override: Option<PathBuf>,
    pub dirs: Vec<PathBuf>,
}

impl Roots {
    /// 按文档里的顺序从环境构造。`exe_dir` 是 `rove.exe` 所在目录。
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>) -> Self {
        let non_empty = |name: &str| env(name).filter(|v| !v.is_empty());
        let components = non_empty(COMPONENTS_DIR_ENV)
            .map(PathBuf::from)
            .or_else(|| {
                non_empty("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Rove").join("components"))
            });
        let mut dirs: Vec<PathBuf> = components
            .map(|d| d.join(COMPONENT_DIR))
            .into_iter()
            .collect();
        dirs.extend(exe_dir.map(|d| d.join("components").join(COMPONENT_DIR)));
        Self {
            env_override: non_empty(INDEXER_ENV).map(PathBuf::from),
            dirs,
        }
    }

    pub fn current() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_owned));
        Self::from_env(|k| std::env::var_os(k), exe_dir.as_deref())
    }
}

/// 版本子目录里的一个候选。
struct Candidate {
    manifest: Manifest,
    exe: PathBuf,
}

fn candidates(dir: &Path) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let exe = path.join(EXE_FILE);
            let text = std::fs::read_to_string(path.join(MANIFEST_FILE)).ok()?;
            exe.is_file().then_some(())?;
            Some(Candidate {
                manifest: Manifest::parse(&text)?,
                exe,
            })
        })
        .collect()
}

/// 按规则查找组件，不启动任何进程。
pub fn discover(roots: &Roots) -> Discovery {
    if let Some(path) = &roots.env_override {
        return IndexerComponent::from_path(path).map_or(Discovery::NotInstalled, Discovery::Found);
    }
    let mut compatible: Vec<(Candidate, usize)> = Vec::new();
    let mut rejected: Option<(Manifest, Incompatible)> = None;
    for (rank, dir) in roots.dirs.iter().enumerate() {
        for candidate in candidates(dir) {
            match check(&candidate.manifest) {
                Ok(_) => compatible.push((candidate, rank)),
                Err(why) => {
                    if rejected
                        .as_ref()
                        .is_none_or(|(m, _)| candidate.manifest.version > m.version)
                    {
                        rejected = Some((candidate.manifest, why));
                    }
                }
            }
        }
    }
    // 版本最高者优先；版本相同时靠前的根目录（用户目录）优先。
    compatible
        .sort_by(|(a, ra), (b, rb)| b.manifest.version.cmp(&a.manifest.version).then(ra.cmp(rb)));
    if let Some((best, _)) = compatible.into_iter().next() {
        if let Some(mut component) = IndexerComponent::from_path(&best.exe) {
            component.version = Some(best.manifest.version);
            component.protocol = check(&best.manifest).ok();
            return Discovery::Found(component);
        }
    }
    match rejected {
        Some((_, why)) => Discovery::Incompatible(why),
        None => Discovery::NotInstalled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, protocol: u32, min: u32, format: u32) -> String {
        format!(
            r#"{{"component":"indexer-cs","version":"{version}","protocol":{protocol},"protocol_min":{min},"index_format":{format}}}"#
        )
    }

    fn install(root: &Path, dir: &str, text: &str) {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(EXE_FILE), b"").unwrap();
        std::fs::write(d.join(MANIFEST_FILE), text).unwrap();
    }

    fn roots(dirs: &[&Path]) -> Roots {
        Roots {
            env_override: None,
            dirs: dirs.iter().map(|d| d.to_path_buf()).collect(),
        }
    }

    fn found_version(d: &Discovery) -> Option<[u16; 4]> {
        match d {
            Discovery::Found(c) => c.version,
            _ => None,
        }
    }

    #[test]
    fn versions_parse() {
        assert_eq!(parse_version("1.2.0"), Some([1, 2, 0, 0]));
        assert_eq!(parse_version("1.2.3.4"), Some([1, 2, 3, 4]));
        assert_eq!(parse_version("1"), None);
        assert_eq!(parse_version("1.2.3.4.5"), None);
        assert_eq!(parse_version("a.b"), None);
    }

    #[test]
    fn manifest_requires_component_name_and_fields() {
        let ok = Manifest::parse(&manifest("1.2.0.0", 1, 1, 2)).unwrap();
        assert_eq!(ok.version, [1, 2, 0, 0]);
        assert_eq!(Manifest::parse("{}"), None);
        assert_eq!(Manifest::parse("not json"), None);
        assert_eq!(
            Manifest::parse(
                r#"{"component":"other","version":"1.0","protocol":1,"index_format":2}"#
            ),
            None
        );
        // 没有 protocol_min 时等于 protocol
        let m = Manifest::parse(
            r#"{"component":"indexer-cs","version":"1.2.0","protocol":1,"index_format":2}"#,
        )
        .unwrap();
        assert_eq!(m.protocol_min, 1);
    }

    #[test]
    fn protocol_negotiation_covers_both_directions() {
        assert_eq!(negotiate_protocol(1, 1), Ok(1));
        // 组件更新（支持 1..3）但仍能说 Rove 的协议 → 用 Rove 的最高版本
        assert_eq!(negotiate_protocol(3, 1), Ok(PROTOCOL));
        // 组件只会说比 Rove 更高的协议 → 组件太新
        let new = negotiate_protocol(PROTOCOL + 2, PROTOCOL + 1).unwrap_err();
        assert_eq!(new.direction, Direction::ComponentTooNew);
        assert_eq!(new.aspect, Aspect::Protocol);
        // 组件最高协议比 Rove 最低的还低 → 组件太旧
        if PROTOCOL_MIN > 0 {
            let old = negotiate_protocol(PROTOCOL_MIN - 1, 0).unwrap_err();
            assert_eq!(old.direction, Direction::ComponentTooOld);
        }
    }

    #[test]
    fn index_format_mismatch_is_directional() {
        let old = check(&Manifest::parse(&manifest("0.9.0", 1, 1, EXPECTED_FORMAT - 1)).unwrap());
        assert_eq!(old.unwrap_err().direction, Direction::ComponentTooOld);
        let new = check(&Manifest::parse(&manifest("9.0.0", 1, 1, EXPECTED_FORMAT + 1)).unwrap());
        let err = new.unwrap_err();
        assert_eq!(err.direction, Direction::ComponentTooNew);
        assert_eq!(err.aspect, Aspect::IndexFormat);
        assert_eq!(err.component_version.as_deref(), Some("9.0.0.0"));
    }

    #[test]
    fn nothing_installed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(discover(&roots(&[dir.path()])), Discovery::NotInstalled);
        assert_eq!(
            discover(&roots(&[&dir.path().join("missing")])),
            Discovery::NotInstalled
        );
        // 没有清单或没有可执行文件的目录不算候选
        std::fs::create_dir_all(dir.path().join("1.2.0")).unwrap();
        std::fs::write(dir.path().join("1.2.0").join(EXE_FILE), b"").unwrap();
        assert_eq!(discover(&roots(&[dir.path()])), Discovery::NotInstalled);
    }

    #[test]
    fn highest_compatible_version_wins_across_roots() {
        let user = tempfile::tempdir().unwrap();
        let portable = tempfile::tempdir().unwrap();
        install(user.path(), "1.2.0", &manifest("1.2.0", 1, 1, 2));
        install(user.path(), "1.2.5", &manifest("1.2.5", 1, 1, 2));
        install(portable.path(), "1.3.0", &manifest("1.3.0", 1, 1, 2));
        // 版本更高但格式不兼容的候选不会被选中
        install(portable.path(), "2.0.0", &manifest("2.0.0", 1, 1, 3));
        let d = discover(&roots(&[user.path(), portable.path()]));
        assert_eq!(found_version(&d), Some([1, 3, 0, 0]));
        // 同版本：靠前的根目录优先
        install(portable.path(), "1.3.0", &manifest("1.3.0", 1, 1, 2));
        install(user.path(), "1.3.0", &manifest("1.3.0", 1, 1, 2));
        let Discovery::Found(c) = discover(&roots(&[user.path(), portable.path()])) else {
            panic!("expected a component");
        };
        assert!(c.program.starts_with(user.path()));
    }

    #[test]
    fn only_incompatible_candidates_report_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "0.5.0", &manifest("0.5.0", 1, 1, 1));
        install(dir.path(), "9.0.0", &manifest("9.0.0", 7, 7, 2));
        let Discovery::Incompatible(why) = discover(&roots(&[dir.path()])) else {
            panic!("expected incompatible");
        };
        assert_eq!(why.direction, Direction::ComponentTooNew);
        assert_eq!(why.component_version.as_deref(), Some("9.0.0.0"));
    }

    #[test]
    fn env_override_skips_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("custom.exe");
        std::fs::write(&exe, b"").unwrap();
        let r = Roots {
            env_override: Some(exe.clone()),
            dirs: vec![],
        };
        let Discovery::Found(c) = discover(&r) else {
            panic!("expected a component");
        };
        assert_eq!(c.program, exe);
        assert_eq!(c.version, None);
        // 指向不存在的文件：未安装（不回退到别的位置，免得悄悄用错组件）
        let r = Roots {
            env_override: Some(dir.path().join("none.exe")),
            dirs: vec![],
        };
        assert_eq!(discover(&r), Discovery::NotInstalled);
    }

    #[test]
    fn roots_follow_the_documented_order() {
        let env = |k: &str| match k {
            "LOCALAPPDATA" => Some(OsString::from(r"C:\Local")),
            _ => None,
        };
        let r = Roots::from_env(env, Some(Path::new(r"C:\Apps\Rove")));
        assert_eq!(
            r.dirs,
            vec![
                PathBuf::from(r"C:\Local\Rove\components\indexer-cs"),
                PathBuf::from(r"C:\Apps\Rove\components\indexer-cs"),
            ]
        );
        let env = |k: &str| match k {
            "LOCALAPPDATA" => Some(OsString::from(r"C:\Local")),
            COMPONENTS_DIR_ENV => Some(OsString::from(r"D:\C")),
            INDEXER_ENV => Some(OsString::from(r"D:\x.exe")),
            _ => None,
        };
        let r = Roots::from_env(env, None);
        assert_eq!(r.dirs, vec![PathBuf::from(r"D:\C\indexer-cs")]);
        assert_eq!(r.env_override, Some(PathBuf::from(r"D:\x.exe")));
    }
}
