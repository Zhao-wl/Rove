# C 链人工验收清单（#5 / #6）

更新：2026-11-10。以下全部是待手测项；未测试请写“未测”，发现异常请保留步骤和证据，不勾选通过。

预算已确认：60 MB 只约束未开启预览的基础阅读器；WebView2 预览单列预算，具体上限尚未设定。无需因预览进程树超过 60 MB 判失败。

## 0. 准备与启动

先记录 Windows 版本、输入法名称/版本、每台显示器分辨率与缩放比例。建议先测 100%，再测 125% 或 150%；多显示器没有条件可写未测。测试完成后恢复原显示设置。普通字母快捷键用英文输入状态、直接按小写键，不必按 Shift。

### #5：输入法原型（PR #80）

在 PowerShell 运行，默认读取本工作树的 UX_SPEC.md 作为背景文本，不会编辑该文件：

```powershell
$imeRoot = 'C:\Users\zhaowenlong\.codex\worktrees\rove-c-platform\Rove'
& "$imeRoot\spikes\ime-gpui\target\release\ime-gpui.exe" "$imeRoot\docs\UX_SPEC.md"
```

输入位置是窗口底部的输入条，不是上方只读文档。关闭窗口退出。若输入法切换键与下面不同，按自己系统配置操作，不需要改注册表。

### #6：WebView2 交互原型（PR #81）

此命令会启动可见窗口，将日志保存到独立时间戳目录；不添加 `--auto`，因此有充足时间操作：

```powershell
$webRoot = 'C:\Users\zhaowenlong\.codex\worktrees\rove-c-webview\Rove'
$runDir = Join-Path $webRoot ('spikes\webview-gpui\runs\manual-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $runDir | Out-Null
$webProcess = Start-Process -FilePath "$webRoot\spikes\webview-gpui\target\release\webview-gpui.exe" -WindowStyle Normal -PassThru -RedirectStandardOutput "$runDir\events.jsonl" -RedirectStandardError "$runDir\stderr.log"
$runDir
```

启动后点击/激活原型窗口，在宿主侧按 `o` 打开右侧预览。若键盘焦点在网页中，先按 `Esc` 返回宿主；每次测试网页快捷键前，重新点击右侧网页正文。若 Esc 无法返回，记录为失败并重启原型，不要把按键打到其他应用。

可在原 PowerShell 窗口读取日志；若换到第二个窗口，先把 `$runDir` 设为上面输出的实际路径：

```powershell
Get-Content "$runDir\events.jsonl" | Select-String 'key_route|ready|controller|closed|error'
```

若启动失败，先保留 stderr.log 和 events.jsonl。两个 exe 已在本机存在；如果目录移动或 exe 缺失，在已配置 MSVC 的开发者 PowerShell 中进入对应工作树，执行 `cargo build --manifest-path spikes/ime-gpui/Cargo.toml --release` 或 `cargo build --manifest-path spikes/webview-gpui/Cargo.toml --release`，再启动。

## 1. #5 微软拼音验收

| 勾选 | 操作 | 通过标准与证据 |
|---|---|---|
| [ ] IME-1 长句 | 点击底部输入条，切换微软拼音中文模式。逐字输入“我们正在验证中文输入法候选窗口能够始终跟随当前插入位置”，先保持长组合串，再选词提交；输入期间移动组合串光标 | 候选框随插入符移动，无持续滞留旧位置；提交内容正确。录屏包含组合过程及提交，不能只截图最后结果 |
| [ ] IME-2 屏幕边缘 | 在窗口仍可操作的前提下移到屏幕右边缘，再到下边缘，再到右下角；每处重复长串输入 | 候选框留在可见屏幕内，必要时翻转/调整位置，无裁切；各留一张候选框展开的截图 |
| [ ] IME-3 点击换位 | 未提交组合时，分别点击输入条另一位置、上方只读区域、另一个普通窗口；再返回继续输入 | 无崩溃、无孤立组合串；提交或取消符合输入法行为，无重复/丢失文本；记录实际行为 |
| [ ] IME-4 切输入法 | 组合未提交时切到英文/另一输入法，再切回微软拼音继续输入 | 不崩溃、不残留旧组合串；候选框恢复且新输入正常 |
| [ ] IME-5 非100% DPI | 在 Windows 显示设置改为125%或150%，重新启动原型，重复 IME-1、IME-2 | 候选框紧跟插入符，无随缩放扩大的位置偏移；记录缩放值和截图。至少一种非100%比例 |
| [ ] IME-6 混合字符 | 先粘贴 `A😀中文` 到输入条末尾，再在那里开始中文组合 | 候选框对应真实插入点，无 emoji 后的偏移；这是 UTF-16 边界补测 |
| [ ] IME-7 第三方（有条件） | 已安装搜狗/微信等输入法时重复 IME-1～5 | 分别记录输入法名称、版本及结果；没有安装就写未测，不必为本次测试安装 |

## 2. #6 遮挡、恢复和加载（优先，影响 #16）

1. 启动交互原型，先录屏，再按 `o`。观察右侧加载占位直到网页出现，记录是否长时间空白。当前原型只有文字占位，并非产品最终灰条骨架，不要据此勾选产品骨架样式通过。
2. 预览出现后，保持焦点在宿主（必要时按 Esc），按 `u` 显示**不规避**的重叠矩形。截图须同时包含左侧原生区、右侧网页和跨区矩形；记录哪一层遮住哪一层。
3. 按 `Esc` 关闭矩形，再按 `h` 显示**隐藏网页规避**的矩形。预期整个预览临时隐藏，原生矩形完整可见；截图。不要直接从 `u` 按 `h`，两者都是开关，可能只把矩形关闭。
4. 按 `Esc`：预览应恢复。点击网页，滚动、选中文字并复制；重复 `h`→Esc 三次，检查残影、闪烁、不可点击、焦点丢失或崩溃。
5. 调整窗口大小后重复步骤2～4。保存原始与规避后两种重叠画面的截图/录屏。

- [ ] WV-1 加载占位与恢复正常。
- [ ] WV-2 原始遮挡已记录（该步骤用于复现问题，不要求原生层胜出）。
- [ ] WV-3 隐藏规避后矩形完整，关闭后预览与交互恢复。
- [ ] WV-4 调整窗口尺寸后布局与鼠标命中正常。

**范围限制：只有一个共用矩形，不是真正的命令面板、汉堡菜单、悬停卡片、通知四种控件。四类真实控件逐项验收留到工作台集成，不应让你在此原型里寻找不存在的入口。** 使用 Windows 截图/录屏工具捕获真实可见画面；黑屏或缺失网页的 PrintWindow 图不算通过证据。

## 3. #6 真实按键与焦点

每行开始前先关闭矩形、点击右侧网页正文，确保网页获得焦点，再按表中按键。原型没有产品命令注册，不以产品界面完整出现作为通过标准。

| 勾选 | 按键/操作 | 当前原型应观察到的结果 |
|---|---|---|
| [ ] KEY-1 | Ctrl+P | 日志 `key_route` 的 route 为 `quickOpen`；网页隐藏、原生矩形出现；不弹浏览器打印对话框 |
| [ ] KEY-2 | Ctrl+Shift+P | route=`commandPalette`；与上一行相同；Esc 后预览恢复 |
| [ ] KEY-3 | Ctrl+Shift+V | route=`togglePreview` 且焦点返回宿主。这里只验路由，尚无源码/预览切换功能 |
| [ ] KEY-4 | Esc | 网页有焦点时 route=`escape` 并返回宿主；矩形打开时 Esc 能关闭并恢复预览 |
| [ ] KEY-5 | Tab、Shift+Tab 分别测试 | route=`tab`，返回宿主；不陷入网页。产品完整 Tab 遍历/边界顺序未实现 |
| [ ] KEY-6 | Ctrl+R | route=`blockedRefresh`；网页没有重新加载（不新增 navigation_completed/ready） |
| [ ] KEY-7 | Ctrl+F | route=`findDeferred`；没有实现查找框，此项只验归属，不能记录“预览查找通过” |
| [ ] KEY-8 | 右键、F12、Ctrl+Shift+I | 不弹默认浏览器右键菜单或 DevTools |
| [ ] KEY-9 | 隐藏网页期间输入，再Esc恢复 | 隐藏网页不能继续接收输入；恢复后点击网页可选择、滚动和复制；分别记录键盘/鼠标是否可恢复 |

## 4. #6 DPI、主题及生命周期补测

- [ ] WV-5：125%或150%下重新启动，打开预览、拖动窗口尺寸、测试 WV-2～4 和 KEY-1～5；网页占右半区、无缝隙/越界，鼠标点击与内容对齐。
- [ ] WV-6（有条件）：两个不同缩放显示器之间往返拖动窗口，重复上项；记录两屏比例。只有一屏就写未测。
- [ ] WV-7：网页显示后返回宿主，按 `t`；应变成浅色，页面内容仍在，日志不新增 controller。`t` 仅切浅色，不是主题切换开关。
- [ ] WV-8：返回宿主按 `d` 立即销毁（在下一帧执行，不在按键回调里），再按 `o` 重建，重复三次；网页可重新加载、没有遗留空白子窗口或崩溃。`x` 现在只隐藏并保留控制器，见文末“关闭保留与延迟回收”。

## 5. 不需要你现在勉强补测的项目

以下依赖产品实现或专门环境，我保留为后续工作，不以本次手测代替：

- #2 E2E、workspace 真正使用 vendor、产品 rove.exe 的受控构建/体积比较。
- 四类真实弹出控件、源码/预览切换、完整 Tab 顺序、产品灰条骨架、300秒定时回收。
- 基础阅读器静止空闲60 MB、独立预览预算具体值、外部首帧/OS冷缓存、长期内存增长及OS级网络拦截验证。
- 真正无 WebView2 运行时机器：有现成干净测试机时再安排；不要卸载日常系统运行时。已有“缺失目录”故障注入不等于该验收。

这些遗留项已在报告 [§6.1](../../docs/M0_WEBVIEW_SPIKE_REPORT.md) 逐条登记（冷/热启动与内存口径、静止空闲预算；真机无运行时环境与 300 秒全释放），明确不在 PR #81 完成，转入 #29；#29 还需补的原型缺口见报告 §5.1。

## 6. 反馈模板

可直接复制填写；视频/截图用测试编号命名（如 IME-1.mp4、WV-2.png、WV-3.png），附 #6 的 events.jsonl / stderr.log。日志不用逐行手工分析，交给我核对即可。

```text
Windows版本：
输入法及版本：
显示器分辨率/缩放：

#5：IME-1～7（通过/失败/未测）：
#6：WV-1～8（通过/失败/未测）：
#6：KEY-1～9（通过/失败/未测）：
异常编号 + 操作步骤 + 实际表现：
证据路径/附件：
```

## 最新反馈修复：三模式对照（优先执行）

在本工作树PowerShell中逐个启动，关闭前一个窗口再换模式：

```powershell
./spikes/webview-gpui/launch-review.ps1 -Mode Baseline
./spikes/webview-gpui/launch-review.ps1 -Mode Hwnd
./spikes/webview-gpui/launch-review.ps1 -Mode Transparent
```

每个命令返回独立日志目录，保存exe哈希且恢复调用者环境变量。按O打开：应看到网页正文而非宿主占位；截图记录实际结果，不能用复制成功代替可见性。按U看原始叠放，再按H切换隐藏规避，分别截图。网页按Tab/Shift+Tab应回宿主，宿主再次按Tab/Shift+Tab应进入网页；检查key_route的source为trusted_bridge_keydown/host。继续WV-1～8、KEY-5/9，100%和125%分别记录。Transparent是待验候选，不预设一定透出网页；Hwnd也须实测实际叠放。

## 最新三模式手测之后的复验重点

Hwnd已有100%/125%、双屏及Tab真实通过记录；Baseline是已确认不可见的负对照，不作为推荐模式。新版请重点检查：
1. Transparent：启动未打开、加载中、H隐藏、X关闭四种阶段均应不再透出后方窗口；ready可见时仍显示正文，U矩形完整。
2. Hwnd在125%调整尺寸、切浅色主题后，X后约200ms按O，连续至少三轮；每轮应有open_requested、environment_reused、controller_requested/completed、ready。若失败，保留全部日志和操作时间，不用延迟半秒替代该条件。
3. 继续检查边缘命中、双屏与已通过按键以防回归；模式改变后的内存与300秒全释放不得沿用旧指标。

最新复验已确认Transparent四态不透底、两模式混合DPI/主题/KEY-1～9通过。下一轮重点仅是Transparent125%原始短按的200ms X→O：先确认host_focus_restored.ok=true；host_key附native_focus，若没有o需保留原始输入发送记录和按键持续时间。不要只用更长按键替代首轮失败条件。

## 关闭保留与延迟回收（f7e3cf6 之后）

`x` 只隐藏预览并保留控制器，到 `ROVE_WEBVIEW_IDLE_MS`（默认 300000，对应 `rove.markdown.webviewIdleTimeout`）后才在渲染帧里真正 `Close()`；超时前按 `o` 直接重新显示，日志为 `dismissed` → `reshown`，不再有 `open_requested`/`ready`。`d` 请求立即销毁（同样在下一帧执行）。

复验（第 1 项 125% 物理短按已暂缓，不阻塞本 PR；需要时再执行）：
1. Transparent 125%，调整尺寸、按 `t` 后，原始短按 `x`→约200ms→`o` 至少三轮：每轮应有 `dismissed`、`host_focus_restored.ok=true`、`host_key=o`、`reshown`，网页正文立刻可见；关闭期间右侧显示“Preview closed — controller retained until idle timeout”，不透底。
2. `$env:ROVE_WEBVIEW_IDLE_MS='1000'` 后启动：`x` 后等 1.5 秒应出现 `dispose`/`closed`，再按 `o` 重建并 `ready`。
3. 自动复现脚本 `shortpress.ps1`（每次注入前校验前台窗口是原型，否则中止）：

```powershell
./spikes/webview-gpui/shortpress.ps1 -Exe ./spikes/webview-gpui/target/release/webview-gpui.exe -Mode Transparent -Trials 30 -Prep -Out ./spikes/webview-gpui/runs/shortpress
```
