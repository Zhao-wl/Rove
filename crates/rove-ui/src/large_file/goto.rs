//! 转到行的等待（#22，`UX_SPEC` §3.2 与 §6.2）。
//!
//! `workbench.action.gotoLine` 输入的目标行若已索引就直接跳转；尚未索引时进入**等待**状态，
//! 索引到达该行后自动跳转。等待可以取消（§6.2：可能超过 1 秒的操作必须可取消）。
//!
//! 本文件提供状态与回调，不依赖 GPUI；快速输入框里的展示由 #24 负责（见
//! `skeleton/workbench/quick_access_host.rs`）。
//!
//! 等待用**轮询**实现而不是 `MappedText::wait_for_lines`：代码视图拿到的是
//! `Arc<dyn TextSource>`，没有向下转型的接口；轮询对任何文本源都成立，代价是最多一个
//! [`POLL_INTERVAL`] 的延迟。

use rove_base::{Cancellation, DocumentId, Line};
use rove_buffer::TextSource;
use std::time::{Duration, Instant};

/// 轮询间隔：一帧左右，用户感觉是“索引到了就跳”。
pub const POLL_INTERVAL: Duration = Duration::from_millis(16);

/// 等待的结局。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    /// 目标行已索引，可以跳转（行号 0 起始）。
    Ready(Line),
    /// 用户取消了等待。
    Cancelled,
    /// 索引已建完，但文件没有这么多行：跳到最后一行。
    OutOfRange {
        last: Line,
    },
    TimedOut,
}

/// 一次进行中的等待。宿主每打开一次“转到行”最多持有一个。
#[derive(Clone)]
pub struct PendingGoto {
    /// 等待的文档。
    pub document: DocumentId,
    /// 目标行（1 起始，与用户输入一致）。
    pub line: u64,
    /// 取消句柄：`cancel()` 后等待线程尽快退出，界面回到普通状态。
    pub cancel: Cancellation,
}

impl PendingGoto {
    pub fn new(document: DocumentId, line: u64) -> Self {
        Self {
            document,
            line,
            cancel: Cancellation::default(),
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.check().is_err()
    }
}

/// 输入框下方的提示文案 key：“正在建立行索引，完成后跳转”。
pub const WAIT_HINT_KEY: &str = "largeFile.gotoLine.waiting";
/// 取消按钮的文案 key。
pub const CANCEL_KEY: &str = "largeFile.gotoLine.cancel";
/// 等待提示下方的取消说明（`{key}` 是 `quickInput.hide` 的实际键位，默认 Esc）。
pub const CANCEL_HINT_KEY: &str = "largeFile.gotoLine.cancelHint";

/// 目标行现在能不能直接跳（已索引，或索引已建完）。
///
/// `line` 是 1 起始的用户输入。已索引返回 `Some(Ready)`；索引已建完但越界返回
/// `Some(OutOfRange)`；否则 `None`，表示需要等待。
pub fn resolve_now(text: &dyn TextSource, line: u64) -> Option<WaitOutcome> {
    let line = line.max(1);
    let count = text.lines();
    let known = count.known.max(1);
    if line <= known {
        return Some(WaitOutcome::Ready(Line(line - 1)));
    }
    if count.complete {
        return Some(WaitOutcome::OutOfRange {
            last: Line(known - 1),
        });
    }
    None
}

/// 在后台线程上等到目标行被索引、索引建完或被取消。
///
/// `deadline` 是兜底上限（索引线程异常退出时不至于永远挂着）；正常情况下由索引完成或取消结束。
pub fn wait_for_line(
    text: &dyn TextSource,
    line: u64,
    cancel: &Cancellation,
    timeout: Duration,
) -> WaitOutcome {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.check().is_err() {
            return WaitOutcome::Cancelled;
        }
        if let Some(outcome) = resolve_now(text, line) {
            return outcome;
        }
        if Instant::now() >= deadline {
            return WaitOutcome::TimedOut;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_buffer::RopeText;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    /// 行索引逐步推进的假文本源：`known` 由测试驱动。
    struct Growing {
        inner: RopeText,
        known: AtomicU64,
        complete_at: u64,
    }

    impl Growing {
        fn new(lines: u64, complete_at: u64) -> Self {
            Self {
                inner: RopeText::from_text(&"x\n".repeat(lines as usize)),
                known: AtomicU64::new(1),
                complete_at,
            }
        }
    }

    impl rove_buffer::TextSource for Growing {
        fn version(&self) -> rove_base::Version {
            self.inner.version()
        }
        fn byte_len(&self) -> u64 {
            self.inner.byte_len()
        }
        fn lines(&self) -> rove_buffer::LineCount {
            let known = self.known.load(Ordering::SeqCst);
            rove_buffer::LineCount {
                known,
                indexed_bytes: rove_base::ByteOffset(known * 2),
                total_bytes: self.inner.byte_len(),
                complete: known >= self.complete_at,
            }
        }
        fn line_range(&self, l: rove_base::Line) -> rove_base::Result<rove_base::ByteRange> {
            self.inner.line_range(l)
        }
        fn visit_chunks(
            &self,
            r: rove_base::ByteRange,
            v: &mut dyn FnMut(&str),
        ) -> rove_base::Result<()> {
            self.inner.visit_chunks(r, v)
        }
        fn offset(&self, p: rove_base::Position) -> rove_base::Result<rove_base::ByteOffset> {
            self.inner.offset(p)
        }
        fn position(&self, o: rove_base::ByteOffset) -> rove_base::Result<rove_base::Position> {
            self.inner.position(o)
        }
        fn utf16_to_utf8(
            &self,
            l: rove_base::Line,
            c: rove_base::Utf16Column,
        ) -> rove_base::Result<rove_base::Utf8Column> {
            self.inner.utf16_to_utf8(l, c)
        }
        fn utf8_to_utf16(
            &self,
            l: rove_base::Line,
            c: rove_base::Utf8Column,
        ) -> rove_base::Result<rove_base::Utf16Column> {
            self.inner.utf8_to_utf16(l, c)
        }
    }

    #[test]
    fn an_indexed_line_resolves_without_waiting() {
        let text = Growing::new(100, 101);
        text.known.store(50, Ordering::SeqCst);
        assert_eq!(resolve_now(&text, 1), Some(WaitOutcome::Ready(Line(0))));
        assert_eq!(resolve_now(&text, 50), Some(WaitOutcome::Ready(Line(49))));
        // 51 还没索引到：需要等待。
        assert_eq!(resolve_now(&text, 51), None);
    }

    #[test]
    fn waiting_resumes_when_the_index_reaches_the_line() {
        let text = Arc::new(Growing::new(1000, 1001));
        let cancel = Cancellation::default();
        let driver = text.clone();
        let handle = std::thread::spawn(move || {
            for known in [10u64, 100, 400, 900] {
                std::thread::sleep(Duration::from_millis(5));
                driver.known.store(known, Ordering::SeqCst);
            }
        });
        let outcome = wait_for_line(&*text, 800, &cancel, Duration::from_secs(5));
        handle.join().unwrap();
        assert_eq!(outcome, WaitOutcome::Ready(Line(799)));
    }

    #[test]
    fn waiting_can_be_cancelled() {
        let text = Arc::new(Growing::new(1000, 1001));
        let pending = PendingGoto::new(DocumentId(1), 900);
        let cancel = pending.cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            cancel.cancel();
        });
        let outcome = wait_for_line(&*text, 900, &pending.cancel, Duration::from_secs(5));
        assert_eq!(outcome, WaitOutcome::Cancelled);
        assert!(pending.is_cancelled());
    }

    #[test]
    fn a_completed_index_rejects_lines_past_the_end() {
        let text = Growing::new(10, 11);
        text.known.store(11, Ordering::SeqCst);
        assert_eq!(
            resolve_now(&text, 99),
            Some(WaitOutcome::OutOfRange { last: Line(10) })
        );
    }
}
