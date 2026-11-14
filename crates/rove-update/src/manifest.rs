//! 更新清单 `latest.json` 的格式（issue #149 第 2 节定义，#76 的 `cargo xtask release` 生成）。
//!
//! 完整的字段说明与示例在 [`docs/UPDATE.md`](../../../docs/UPDATE.md)，这里只写解析规则：
//!
//! - **严格但向前兼容**：`schema` 必须是 [`SCHEMA`]、`product` 必须是 `rove`，
//!   不认识的字段一律忽略（以后 #76 加字段不会让旧版本的 Rove 以为清单坏了）。
//! - `installer` 可以是 `null`（本机没装 Inno Setup 时 #76 会这样产出）。没有安装包
//!   就没有可以装的东西，[`Manifest::installer`] 为 `None` 时更新端只提示、不下载。
//! - `components` 没有可选组件时是空数组（#43 的索引组件）。
//! - `min_version` 是**可选**字段：低于它的版本必须更新。#76 当前不产出这个字段，
//!   解析器先支持着，这样以后要用的时候老版本的 Rove 也能读懂。

use crate::{version::Version, Result, UpdateError};
use serde::Deserialize;

/// 本版本能读的清单 schema。
pub const SCHEMA: u32 = 1;
/// 清单里的产品名；对不上说明拿到的是别的产品的清单。
pub const PRODUCT: &str = "rove";
/// C# 索引组件在 `components` 里的 id（与 #43 的组件目录名一致）。
pub const COMPONENT_INDEXER_CS: &str = "indexer-cs";

/// 一个可下载的产物。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Artifact {
    /// 文件名（下载后就用这个名字落地，不从 URL 里猜）。
    pub file: String,
    pub url: String,
    pub size: u64,
    /// 十六进制小写的 SHA-256。
    pub sha256: String,
    /// 静默安装参数，由 #76 写进清单（更新端照着调用，不在代码里写死）。
    #[serde(default)]
    pub silent_args: Vec<String>,
}

/// 可选组件（当前只有 C# 索引组件）。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Component {
    pub id: String,
    pub version: String,
    pub file: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: u32,
    pub product: String,
    #[serde(default)]
    pub channel: String,
    pub version: String,
    #[serde(default)]
    pub released: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub notes_url: String,
    #[serde(default)]
    pub installer: Option<Artifact>,
    #[serde(default)]
    pub portable: Option<Artifact>,
    #[serde(default)]
    pub components: Vec<Component>,
    /// 低于它的版本必须更新（可选，见模块文档）。
    #[serde(default)]
    pub min_version: Option<String>,
}

impl Manifest {
    /// 解析清单字节。**调用方必须先验签**（[`crate::signature::verify`]），
    /// 正确的入口是 [`Manifest::parse_signed`]。
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let manifest: Manifest = serde_json::from_slice(bytes)
            .map_err(|e| UpdateError::Manifest(format!("JSON 无法解析：{e}")))?;
        if manifest.schema != SCHEMA {
            return Err(UpdateError::Manifest(format!(
                "schema 是 {}，本版本只读 {SCHEMA}",
                manifest.schema
            )));
        }
        if manifest.product != PRODUCT {
            return Err(UpdateError::Manifest(format!(
                "product 是 {:?}，不是 {PRODUCT:?}",
                manifest.product
            )));
        }
        // 版本号在这里就解析一次：畸形的版本号不该流到后面的流程里。
        manifest.version()?;
        Ok(manifest)
    }

    /// 先验签再解析——**唯一**应该被流程调用的入口。顺序很重要：没验过签的字节
    /// 不进解析器。
    pub fn parse_signed(bytes: &[u8], signature_file: &str) -> Result<Self> {
        crate::signature::verify(bytes, signature_file)?;
        Self::parse(bytes)
    }

    /// 清单声明的版本号。
    pub fn version(&self) -> Result<Version> {
        Version::parse(&self.version)
    }

    /// `min_version`：解析不了就当没写（不会因此拦住用户）。
    pub fn min_version(&self) -> Option<Version> {
        self.min_version
            .as_deref()
            .and_then(|text| Version::parse(text).ok())
    }

    /// `current` 是否低于 `min_version`（低于时提示必须更新）。
    pub fn requires_update(&self, current: &Version) -> bool {
        self.min_version().is_some_and(|min| current < &min)
    }

    /// 清单里的 C# 索引组件（#43）。
    pub fn indexer_component(&self) -> Option<&Component> {
        self.components
            .iter()
            .find(|c| c.id == COMPONENT_INDEXER_CS)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 与 `cargo xtask release` 实际产出的形状一致（docs/RELEASE.md §4）。
    pub(crate) fn sample(version: &str) -> String {
        format!(
            r#"{{
  "schema": 1,
  "product": "rove",
  "channel": "stable",
  "version": "{version}",
  "released": "2026-11-14",
  "commit": "0123456789abcdef0123456789abcdef01234567",
  "notes_url": "https://example.invalid/releases/tag/v{version}",
  "installer": {{
    "file": "Rove-{version}-win-x64-setup.exe",
    "url": "https://example.invalid/Rove-{version}-win-x64-setup.exe",
    "size": 12,
    "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
    "silent_args": ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCANCEL", "/SP-"]
  }},
  "portable": {{
    "file": "Rove-{version}-win-x64.zip",
    "url": "https://example.invalid/Rove-{version}-win-x64.zip",
    "size": 10,
    "sha256": "1111111111111111111111111111111111111111111111111111111111111111"
  }},
  "components": [
    {{
      "id": "indexer-cs",
      "version": "1.2.0.0",
      "file": "rove-indexer-cs-1.2.0.0-win-x64.zip",
      "url": "https://example.invalid/rove-indexer-cs-1.2.0.0-win-x64.zip",
      "size": 20,
      "sha256": "2222222222222222222222222222222222222222222222222222222222222222"
    }}
  ]
}}
"#
        )
    }

    #[test]
    fn the_release_pipeline_shape_parses() {
        let manifest = Manifest::parse(sample("0.1.1").as_bytes()).unwrap();
        assert_eq!(
            manifest.version().unwrap(),
            Version::parse("0.1.1").unwrap()
        );
        assert_eq!(manifest.channel, "stable");
        let installer = manifest.installer.as_ref().unwrap();
        assert_eq!(installer.file, "Rove-0.1.1-win-x64-setup.exe");
        // 静默参数来自清单，不在代码里写死（与 #76 对齐）。
        assert_eq!(installer.silent_args[0], "/VERYSILENT");
        assert_eq!(manifest.indexer_component().unwrap().version, "1.2.0.0");
        assert!(manifest.portable.is_some());
        assert_eq!(manifest.min_version(), None);
    }

    /// #76 在没装 Inno Setup 时产出 `"installer": null`、`"components": []`。
    #[test]
    fn a_manifest_without_an_installer_still_parses() {
        let text = r#"{"schema":1,"product":"rove","version":"0.1.1","installer":null,
            "portable":{"file":"a.zip","url":"u","size":1,"sha256":"x"},"components":[]}"#;
        let manifest = Manifest::parse(text.as_bytes()).unwrap();
        assert!(manifest.installer.is_none());
        assert!(manifest.components.is_empty());
        assert!(manifest.indexer_component().is_none());
    }

    /// 向前兼容：不认识的字段忽略掉，不报错。
    #[test]
    fn unknown_fields_are_ignored() {
        let text = r#"{"schema":1,"product":"rove","version":"0.2.0","surprise":{"a":1}}"#;
        assert_eq!(Manifest::parse(text.as_bytes()).unwrap().version, "0.2.0");
    }

    /// 验收：版本号非法 → 解析就失败（流程据此当作「没有更新」）。
    #[test]
    fn a_malformed_version_fails_to_parse() {
        let text = r#"{"schema":1,"product":"rove","version":"not-a-version"}"#;
        let error = Manifest::parse(text.as_bytes()).unwrap_err();
        assert!(matches!(error, UpdateError::Version(_)), "{error:?}");
    }

    #[test]
    fn a_foreign_or_future_manifest_is_rejected() {
        for text in [
            r#"{"schema":2,"product":"rove","version":"0.1.1"}"#,
            r#"{"schema":1,"product":"vscode","version":"0.1.1"}"#,
            r#"{"schema":1,"version":"0.1.1"}"#,
            "not json at all",
            "",
        ] {
            let error = Manifest::parse(text.as_bytes()).unwrap_err();
            assert!(matches!(error, UpdateError::Manifest(_)), "{text:?}");
        }
    }

    #[test]
    fn min_version_marks_an_update_as_required() {
        let text = r#"{"schema":1,"product":"rove","version":"0.3.0","min_version":"0.2.0"}"#;
        let manifest = Manifest::parse(text.as_bytes()).unwrap();
        assert!(manifest.requires_update(&Version::parse("0.1.9").unwrap()));
        assert!(!manifest.requires_update(&Version::parse("0.2.0").unwrap()));
        // 写坏了就当没写，不会把用户拦在门外。
        let broken = r#"{"schema":1,"product":"rove","version":"0.3.0","min_version":"???"}"#;
        let manifest = Manifest::parse(broken.as_bytes()).unwrap();
        assert!(!manifest.requires_update(&Version::parse("0.1.0").unwrap()));
    }
}
