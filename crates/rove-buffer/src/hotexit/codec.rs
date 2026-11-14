//! 备份文件的二进制编解码（#50）。
//!
//! 为什么不用 JSON：备份里最大的一块是缓冲内容，JSON 要对整份文本做转义与反转义，
//! 1 MB 的文件每次去抖备份都要多花一遍 O(n) 的转义与一份额外内存。这里用“长度前缀 + 原始
//! UTF-8 字节”，写入就是一次 `extend_from_slice`，读出就是一次切片校验。
//!
//! 格式是自描述的：开头有魔数与版本号，读取时任何越界、长度不合法、UTF-8 不合法都返回
//! [`DecodeError`]，由调用方按“没有备份”处理（`UX_SPEC` 的要求是恢复失败不能让 Rove 打不开）。

use std::path::PathBuf;

/// 读不出来的备份一律按“没有备份”处理，所以只需要一个不带细节的错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError;

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("backup is not readable")
    }
}

impl std::error::Error for DecodeError {}

pub type DecodeResult<T> = std::result::Result<T, DecodeError>;

/// 顺序写出。
#[derive(Debug, Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn with_magic(magic: &[u8], version: u32) -> Self {
        let mut writer = Self::default();
        writer.bytes.extend_from_slice(magic);
        writer.u32(version);
        writer
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn str(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.bytes.extend_from_slice(value.as_bytes());
    }

    /// 路径按 `to_string_lossy` 写出：Windows 上的路径都是合法 UTF-16，转成 UTF-8 不丢信息；
    /// 万一有孤立代理项，丢掉的那一份备份读回来也只是指向一个不存在的文件，按缺失处理。
    pub fn path(&mut self, value: &std::path::Path) {
        self.str(&value.to_string_lossy());
    }

    /// 写出一个可选值：先写 1 位存在标记。
    pub fn option<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
        match value {
            Some(value) => {
                self.bool(true);
                write(self, value);
            }
            None => self.bool(false),
        }
    }

    /// 写出一个序列：先写元素个数。
    pub fn seq<T>(
        &mut self,
        items: impl ExactSizeIterator<Item = T>,
        mut write: impl FnMut(&mut Self, T),
    ) {
        self.u64(items.len() as u64);
        for item in items {
            write(self, item);
        }
    }
}

/// 顺序读入。
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    /// 校验魔数与版本号；不匹配时返回 [`DecodeError`]（旧版本的备份按“没有备份”丢弃）。
    pub fn with_magic(bytes: &'a [u8], magic: &[u8], version: u32) -> DecodeResult<Self> {
        let mut reader = Self { bytes, at: 0 };
        if reader.take(magic.len())? != magic || reader.u32()? != version {
            return Err(DecodeError);
        }
        Ok(reader)
    }

    fn take(&mut self, count: usize) -> DecodeResult<&'a [u8]> {
        let end = self.at.checked_add(count).ok_or(DecodeError)?;
        let slice = self.bytes.get(self.at..end).ok_or(DecodeError)?;
        self.at = end;
        Ok(slice)
    }

    pub fn u8(&mut self) -> DecodeResult<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> DecodeResult<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError),
        }
    }

    pub fn u32(&mut self) -> DecodeResult<u32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| DecodeError)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn u64(&mut self) -> DecodeResult<u64> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| DecodeError)?;
        Ok(u64::from_le_bytes(bytes))
    }

    pub fn str(&mut self) -> DecodeResult<&'a str> {
        let len = usize::try_from(self.u64()?).map_err(|_| DecodeError)?;
        std::str::from_utf8(self.take(len)?).map_err(|_| DecodeError)
    }

    pub fn path(&mut self) -> DecodeResult<PathBuf> {
        Ok(PathBuf::from(self.str()?))
    }

    pub fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> DecodeResult<T>,
    ) -> DecodeResult<Option<T>> {
        if self.bool()? {
            read(self).map(Some)
        } else {
            Ok(None)
        }
    }

    /// 读入一个序列。元素个数先与剩余字节数比较，避免损坏的长度让我们预分配几个 GB。
    pub fn seq<T>(
        &mut self,
        mut read: impl FnMut(&mut Self) -> DecodeResult<T>,
    ) -> DecodeResult<Vec<T>> {
        let count = usize::try_from(self.u64()?).map_err(|_| DecodeError)?;
        if count > self.bytes.len() - self.at {
            return Err(DecodeError);
        }
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(read(self)?);
        }
        Ok(items)
    }

    /// 还有没有没读完的字节（尾部多余字节视为损坏）。
    pub fn at_end(&self) -> bool {
        self.at == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAGIC: &[u8] = b"ROVETEST";

    #[test]
    fn round_trips_every_primitive() {
        let mut w = Writer::with_magic(MAGIC, 3);
        w.bool(true);
        w.u64(u64::MAX);
        w.str("混合换行\r\n😀");
        w.path(std::path::Path::new(r"D:\Game\Player.cs"));
        w.option(Some(7u64), |w, v| w.u64(v));
        w.option(None::<u64>, |w, v| w.u64(v));
        w.seq([1u64, 2, 3].into_iter(), |w, v| w.u64(v));
        let bytes = w.into_bytes();

        let mut r = Reader::with_magic(&bytes, MAGIC, 3).unwrap();
        assert!(r.bool().unwrap());
        assert_eq!(r.u64().unwrap(), u64::MAX);
        assert_eq!(r.str().unwrap(), "混合换行\r\n😀");
        assert_eq!(r.path().unwrap(), PathBuf::from(r"D:\Game\Player.cs"));
        assert_eq!(r.option(|r| r.u64()).unwrap(), Some(7));
        assert_eq!(r.option(|r| r.u64()).unwrap(), None);
        assert_eq!(r.seq(|r| r.u64()).unwrap(), vec![1, 2, 3]);
        assert!(r.at_end());
    }

    #[test]
    fn corrupt_input_is_rejected_without_panicking() {
        let mut w = Writer::with_magic(MAGIC, 3);
        w.str("abc");
        let bytes = w.into_bytes();
        // 魔数不符、版本不符、截断、长度字段撒谎都只返回错误。
        assert!(Reader::with_magic(&bytes, b"OTHER!!!", 3).is_err());
        assert!(Reader::with_magic(&bytes, MAGIC, 4).is_err());
        assert!(Reader::with_magic(&bytes[..6], MAGIC, 3).is_err());
        let mut truncated = bytes.clone();
        truncated.pop();
        let mut r = Reader::with_magic(&truncated, MAGIC, 3).unwrap();
        assert!(r.str().is_err());
        let mut lying = bytes.clone();
        lying[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
        let mut r = Reader::with_magic(&lying, MAGIC, 3).unwrap();
        assert!(r.str().is_err());
        // 非 UTF-8 的内容。
        let mut invalid = bytes;
        *invalid.last_mut().unwrap() = 0xff;
        let mut r = Reader::with_magic(&invalid, MAGIC, 3).unwrap();
        assert!(r.str().is_err());
    }

    #[test]
    fn absurd_sequence_length_does_not_preallocate() {
        let mut w = Writer::with_magic(MAGIC, 3);
        w.u64(u64::MAX / 2);
        let bytes = w.into_bytes();
        let mut r = Reader::with_magic(&bytes, MAGIC, 3).unwrap();
        assert!(r.seq(|r| r.u64()).is_err());
    }
}
