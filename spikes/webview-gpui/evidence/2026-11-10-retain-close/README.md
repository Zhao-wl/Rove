# 关闭保留与延迟回收（报告 §15）

本机 Windows 11、双 1920×1080、100% 缩放；`shortpress.ps1` SendInput 注入，每次注入前校验前台窗口。

- `t2`：旧逻辑（x 内同步 Close），Transparent，30 次 x→200 ms→o。
- `t4`：新逻辑，Transparent，先调整尺寸＋浅色，30 次。
- `t5`：新逻辑，`ROVE_WEBVIEW_IDLE_MS=1000`，x→1.5 s→o，8 次（真实回收后重建）。
- `t6`：新逻辑，Hwnd，先调整尺寸＋浅色，15 次。
- `1-open.png` / `2-dismissed.png` / `3-reshown-300ms.png`：Transparent 打开、关闭保留、重新显示 300 ms 后。

`events.jsonl` 去掉了 roundtrip/local_resource 行；`trials.jsonl` 为注入端记录。
