//! 一个脏文档的热退出备份记录（#50）。
//!
//! 记录里**必须**有原始路径、打开时识别出的编码与 BOM、主换行符与“是否混合”、文档版本与缓冲
//! 内容；少了编码 / BOM / 混合换行这三项，恢复后保存就会产生整文件差异，等于绕开 M3 守住的
//! “除改动处外字节完全一致”（见 issue 的「风险与提示」）。
//!
//! 另外记录三样恢复时要用的状态：最后一次已知的磁盘标记（交给 #49 的保存冲突路径）、备份时
//! 磁盘上的文件是否已被删除（恢复成“已删除”状态的标签）、以及标签顺序与是否活动标签。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rove_base::{ByteOffset, Span, Version};

use super::codec::{DecodeError, DecodeResult, Reader, Writer};
use crate::contracts::{Encoding, Eol, LineEndings};
use crate::edit::selection::{Selection, Selections};
use crate::edit::transaction::{NewlineMode, Replacement, Transaction};
use crate::edit::{UndoSnapshot, UndoStep, UndoUnit};
use crate::save::DiskStamp;

pub const BACKUP_MAGIC: &[u8] = b"ROVEBAK\x01";
pub const BACKUP_VERSION: u32 = 1;

/// 一个脏文档的备份。
#[derive(Clone, Debug)]
pub struct Backup {
    /// 原始路径（绝对路径）。未命名文档（#63）暂不备份，所以这里永远有值。
    pub path: PathBuf,
    pub language: Arc<str>,
    /// 打开时识别出的编码与 BOM：恢复后按原样写回。
    pub encoding: Encoding,
    pub bom: bool,
    /// 主换行符与“是否混合”。
    pub line_endings: LineEndings,
    /// 缓冲内容（解码后的 UTF-8，保留原始行终止符）。
    pub text: String,
    /// 备份时的文档版本（只记录，恢复时文档从新版本重新开始）。
    pub version: Version,
    /// 备份时的保存点标记（撤销回它时文档重新变为“未修改”）。
    pub saved_marker: u64,
    /// 备份时确实有未保存的修改。撤销栈不备份（`files.restoreUndoStack = false`）时，恢复后的
    /// 撤销标记必然是 0，单靠 `saved_marker` 判断不出脏状态，所以显式记一位。
    pub dirty: bool,
    /// 最后一次已知的磁盘标记：恢复后保存时由 #49 比较，不一致就走保存冲突路径。
    pub disk: Option<DiskStamp>,
    /// 备份时磁盘上的文件已被删除。
    pub deleted: bool,
    /// 撤销栈（`files.restoreUndoStack = false` 时为空快照）。
    pub undo: UndoSnapshot,
    /// 在所在编辑器组里的标签序号与是否活动标签（恢复时保持顺序与活动标签）。
    pub tab_index: u32,
    pub active: bool,
}

impl Backup {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_magic(BACKUP_MAGIC, BACKUP_VERSION);
        w.path(&self.path);
        w.str(&self.language);
        write_encoding(&mut w, &self.encoding);
        w.bool(self.bom);
        write_line_endings(&mut w, &self.line_endings);
        w.u64(self.version.0);
        w.u64(self.saved_marker);
        w.bool(self.dirty);
        w.option(self.disk.as_ref(), write_stamp);
        w.bool(self.deleted);
        w.u32(self.tab_index);
        w.bool(self.active);
        write_undo(&mut w, &self.undo);
        // 内容放最后：读取时前面的元信息都已校验过，坏在内容上也能定位。
        w.str(&self.text);
        w.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> DecodeResult<Self> {
        let mut r = Reader::with_magic(bytes, BACKUP_MAGIC, BACKUP_VERSION)?;
        let backup = Self {
            path: r.path()?,
            language: Arc::from(r.str()?),
            encoding: read_encoding(&mut r)?,
            bom: r.bool()?,
            line_endings: read_line_endings(&mut r)?,
            version: Version(r.u64()?),
            saved_marker: r.u64()?,
            dirty: r.bool()?,
            disk: r.option(read_stamp)?,
            deleted: r.bool()?,
            tab_index: r.u32()?,
            active: r.bool()?,
            undo: read_undo(&mut r)?,
            text: r.str()?.to_owned(),
        };
        if !r.at_end() {
            return Err(DecodeError);
        }
        Ok(backup)
    }

    /// 备份文件名：路径的稳定哈希 + 文件名，便于排查时一眼看出是哪个文件。
    pub fn file_name(path: &Path) -> String {
        let name: String = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .map(|c| c.to_ascii_lowercase())
            .take(32)
            .collect();
        let hash = crate::save::content_hash(super::store::normalize_path(path).as_bytes());
        if name.is_empty() {
            format!("{hash:016x}{}", super::store::BACKUP_EXT)
        } else {
            format!("{name}-{hash:016x}{}", super::store::BACKUP_EXT)
        }
    }
}

// --------------------------------------------------------------------------- 元信息

fn write_encoding(w: &mut Writer, encoding: &Encoding) {
    match encoding {
        Encoding::Utf8 => w.u8(0),
        Encoding::Utf16Le => w.u8(1),
        Encoding::Utf16Be => w.u8(2),
        Encoding::Gbk => w.u8(3),
        Encoding::Other(name) => {
            w.u8(4);
            w.str(name);
        }
    }
}

fn read_encoding(r: &mut Reader<'_>) -> DecodeResult<Encoding> {
    Ok(match r.u8()? {
        0 => Encoding::Utf8,
        1 => Encoding::Utf16Le,
        2 => Encoding::Utf16Be,
        3 => Encoding::Gbk,
        4 => Encoding::Other(Arc::from(r.str()?)),
        _ => return Err(DecodeError),
    })
}

fn write_eol(w: &mut Writer, eol: Eol) {
    w.u8(match eol {
        Eol::Lf => 0,
        Eol::CrLf => 1,
        Eol::Cr => 2,
    });
}

fn read_eol(r: &mut Reader<'_>) -> DecodeResult<Eol> {
    Ok(match r.u8()? {
        0 => Eol::Lf,
        1 => Eol::CrLf,
        2 => Eol::Cr,
        _ => return Err(DecodeError),
    })
}

fn write_line_endings(w: &mut Writer, line_endings: &LineEndings) {
    match line_endings {
        LineEndings::Uniform(eol) => {
            w.bool(false);
            write_eol(w, *eol);
        }
        LineEndings::Mixed { preferred } => {
            w.bool(true);
            write_eol(w, *preferred);
        }
    }
}

fn read_line_endings(r: &mut Reader<'_>) -> DecodeResult<LineEndings> {
    let mixed = r.bool()?;
    let eol = read_eol(r)?;
    Ok(if mixed {
        LineEndings::Mixed { preferred: eol }
    } else {
        LineEndings::Uniform(eol)
    })
}

fn write_stamp(w: &mut Writer, stamp: &DiskStamp) {
    w.u64(stamp.len);
    w.u64(stamp.hash);
    // 修改时间按 Unix epoch 的纳秒写出；epoch 之前的时间（不可能出现在真实文件上）按“未知”。
    let nanos = stamp.modified.and_then(|t| {
        t.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos())
            .and_then(|n| u64::try_from(n).ok())
    });
    w.option(nanos, |w, n| w.u64(n));
}

fn read_stamp(r: &mut Reader<'_>) -> DecodeResult<DiskStamp> {
    Ok(DiskStamp {
        len: r.u64()?,
        hash: r.u64()?,
        modified: r
            .option(|r| r.u64())?
            .map(|n| std::time::UNIX_EPOCH + std::time::Duration::from_nanos(n)),
    })
}

// --------------------------------------------------------------------------- 撤销栈

fn write_undo(w: &mut Writer, snapshot: &UndoSnapshot) {
    w.bool(snapshot.truncated);
    let units = |w: &mut Writer, units: &[UndoUnit]| {
        w.seq(units.iter(), |w, unit| {
            w.u64(unit.id);
            write_selections(w, &unit.selections_before);
            write_selections(w, &unit.selections_after);
            w.seq(unit.steps.iter(), |w, step| {
                write_transaction(w, &step.forward);
                write_transaction(w, &step.inverse);
                write_line_endings(w, &step.line_endings_before);
                write_line_endings(w, &step.line_endings_after);
            });
        });
    };
    units(w, &snapshot.undo);
    units(w, &snapshot.redo);
}

fn read_undo(r: &mut Reader<'_>) -> DecodeResult<UndoSnapshot> {
    fn units(r: &mut Reader<'_>) -> DecodeResult<Vec<UndoUnit>> {
        r.seq(|r| {
            Ok(UndoUnit {
                id: r.u64()?,
                selections_before: read_selections(r)?,
                selections_after: read_selections(r)?,
                steps: r.seq(|r| {
                    Ok(UndoStep {
                        forward: read_transaction(r)?,
                        inverse: read_transaction(r)?,
                        line_endings_before: read_line_endings(r)?,
                        line_endings_after: read_line_endings(r)?,
                    })
                })?,
            })
        })
    }
    Ok(UndoSnapshot {
        truncated: r.bool()?,
        undo: units(r)?,
        redo: units(r)?,
    })
}

fn write_selections(w: &mut Writer, selections: &Selections) {
    w.seq(selections.all().iter(), |w, s| {
        w.u64(s.anchor.0);
        w.u64(s.active.0);
    });
}

fn read_selections(r: &mut Reader<'_>) -> DecodeResult<Selections> {
    let items = r.seq(|r| Ok(Selection::new(ByteOffset(r.u64()?), ByteOffset(r.u64()?))))?;
    // 期望列不持久化（上下移动时重新计算）；空集合是损坏的记录。
    Selections::new(items).map_err(|_| DecodeError)
}

fn write_transaction(w: &mut Writer, transaction: &Transaction) {
    w.bool(matches!(transaction.newlines(), NewlineMode::Verbatim));
    w.seq(transaction.edits().iter(), |w, edit| {
        w.u64(edit.range.start().0);
        w.u64(edit.range.end().0);
        w.str(&edit.text);
    });
}

fn read_transaction(r: &mut Reader<'_>) -> DecodeResult<Transaction> {
    let verbatim = r.bool()?;
    let edits = r.seq(|r| {
        let range =
            Span::new(ByteOffset(r.u64()?), ByteOffset(r.u64()?)).map_err(|_| DecodeError)?;
        Ok(Replacement::new(range, r.str()?))
    })?;
    let transaction = Transaction::new(edits).map_err(|_| DecodeError)?;
    Ok(if verbatim {
        transaction.verbatim()
    } else {
        transaction
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn sample(text: &str, line_endings: LineEndings, encoding: Encoding, bom: bool) -> Backup {
        Backup {
            path: PathBuf::from(r"D:\Game\Assets\Player.cs"),
            language: Arc::from("csharp"),
            encoding,
            bom,
            line_endings,
            text: text.to_owned(),
            version: Version(42),
            saved_marker: 3,
            dirty: true,
            disk: Some(DiskStamp {
                len: 17,
                modified: Some(UNIX_EPOCH + Duration::from_nanos(1_700_000_000_123_456_789)),
                hash: 0xdead_beef,
            }),
            deleted: false,
            undo: UndoSnapshot::default(),
            tab_index: 2,
            active: true,
        }
    }

    #[test]
    fn metadata_survives_the_round_trip() {
        for (line_endings, encoding, bom) in [
            (LineEndings::Uniform(Eol::CrLf), Encoding::Gbk, false),
            (LineEndings::Uniform(Eol::Lf), Encoding::Utf8, true),
            (
                LineEndings::Mixed { preferred: Eol::Lf },
                Encoding::Other(Arc::from("shift_jis")),
                false,
            ),
            (LineEndings::Uniform(Eol::Cr), Encoding::Utf16Be, true),
        ] {
            let backup = sample("第一行\r\n第二行\n", line_endings, encoding.clone(), bom);
            let decoded = Backup::decode(&backup.encode()).unwrap();
            assert_eq!(decoded.text, backup.text);
            assert_eq!(decoded.encoding, encoding);
            assert_eq!(decoded.bom, bom);
            assert_eq!(decoded.path, backup.path);
            assert_eq!(&*decoded.language, "csharp");
            assert_eq!(decoded.version, Version(42));
            assert_eq!(decoded.saved_marker, 3);
            assert!(decoded.dirty);
            assert_eq!(decoded.disk, backup.disk);
            assert_eq!(decoded.tab_index, 2);
            assert!(decoded.active);
            match (&decoded.line_endings, &backup.line_endings) {
                (LineEndings::Uniform(a), LineEndings::Uniform(b)) => assert_eq!(a, b),
                (LineEndings::Mixed { preferred: a }, LineEndings::Mixed { preferred: b }) => {
                    assert_eq!(a, b)
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn truncated_or_padded_records_are_rejected() {
        let bytes = sample("x", LineEndings::Uniform(Eol::Lf), Encoding::Utf8, false).encode();
        assert!(Backup::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut padded = bytes.clone();
        padded.push(0);
        assert!(Backup::decode(&padded).is_err());
        assert!(Backup::decode(b"").is_err());
        assert!(Backup::decode(b"not a backup at all").is_err());
    }

    #[test]
    fn file_name_is_stable_and_distinguishes_paths() {
        let a = Backup::file_name(Path::new(r"D:\Game\Player.cs"));
        assert_eq!(a, Backup::file_name(Path::new(r"d:/game/player.cs/")));
        assert!(a.starts_with("player.cs-"), "{a}");
        assert!(a.ends_with(".rvbak"), "{a}");
        assert_ne!(a, Backup::file_name(Path::new(r"D:\Game\Other.cs")));
    }
}
