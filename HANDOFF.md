# AskBridge 开发交接文档（HANDOFF）

> 写给下一个接手的 agent / 开发者。本文档描述当前工作状态、未完成的改动、调试结论和验证方法。
> 生成时间：2026-08-23。**本文件不要提交到 git。**

---

## 1. 项目与仓库

- 仓库：`D:\askbridge`（GitHub: wanghongyu666qiang/AskBridge），Windows 原生「截图 → AI 网页」快捷工具，Rust + Win32。
- 工作区：`crates/askbridge-core`（纯领域逻辑）、`crates/askbridge-win`（平台层，bin 名 `askbridge`）、`xtask`（发布校验）。
- 远端：origin/main。gh 已登录账号 `wanghongyu666qiang`，有 push 权限。

## 2. 本地已提交、**尚未推送**的 commit

| commit | 内容 |
|---|---|
| `7d23729` | overlay.rs（2414 行）拆分为 `capture/overlay/` 目录（mod/session/guards/layout/draw/gdiplus），纯重构 |
| `60ca952` | overlay 绘制层 PaintCache：双缓冲位图/快照 DC/字体/画刷画笔/GDI+ 会话跨帧复用 |
| `496cdc6` | 新功能：通用粘贴模式（clipboard_paste 第三种打开方式） |
| `1bcf518` | 粘贴激活失败时的 Alt 轻敲重试 |

推送前需要用户验收（用户尚未跑 `scripts/test-hotkey-ui.ps1` 和目视检查）。**推送前先问用户。**

## 3. 当前未提交的工作区改动（2026-08-23 已收口）

这批改动已完成编译、测试、Release 构建和不落盘的真实粘贴流程验收，但仍未提交或推送。

### 3.1 已完成
- 补齐 `Path`、进程查询、COM 和 Windows 应用包查询导入；`CloseHandle` 从实际所属的 `Win32::Foundation` 导入。
- 合法目标包括主流浏览器以及 ChatGPT、Claude、豆包桌面客户端；标题关键词仍必须匹配。
- Codex 桌面端的进程名和窗口标题会分别表现为 `chatgpt.exe` / `ChatGPT`，因此增加 Windows 应用包标识检查，明确排除 `OpenAI.Codex`，且不记录完整路径或包标识。
- 枚举所有合法候选并逐个尝试，避免一个不可激活窗口一直挡住新开的浏览器页面。
- 窗口激活逐级校验前台归属；Ctrl+V 注入失败不会换窗口重复粘贴，并会尽力释放 Ctrl/V。
- 修正 COM 成功/失败检查、`S_FALSE` 配对释放和 HRESULT 诊断。
- 移除通用粘贴路径多余的 `BrowserStage::Started` 事件，修复真实粘贴成功后被状态机误判为失败的问题，并增加专门的工作流测试。
- README、README_EN、PRIVACY、ARCHITECTURE、设置 UI 和错误提示已经同步。

### 3.2 当前只剩
1. 用户目视确认 Edge/目标客户端输入区确实出现截图缩略图。
2. 用户目视检查 overlay 外观和拖拽流畅度。
3. 得到用户明确授权后再提交；推送仍需另行授权。

## 4. 用户已拍板的产品决策（不要推翻）

1. 2026-08-24 用户已覆盖旧决定：设置新增“专用 Chrome 优先，安全失败后通用粘贴”单选项。仅截图请求且 CDP 能证明尚未插入文字或附件时，才自动降级到一次 Ctrl+V；部分写入或状态不确定时禁止降级。
2. **只贴图不打字**：quick_prompt 不自动填入，完成通知里已明确说明。
3. 冷启动（找不到窗口）→ 自动用默认浏览器开 `start_url`，在 `locate_timeout`（取 page_timeout_ms 与 5s 的较大值）预算内轮询盲贴。
4. **最新补充：ChatGPT/Claude/豆包等桌面客户端也是合法粘贴目标**（见 3.2 第 2 条）。

## 5. 调试结论（实测得出，勿重复试错）

自动化调试脚本：`C:\Users\why17\AppData\Local\Temp\askbridge-paste-debug.ps1`
用法：`powershell -NoProfile -ExecutionPolicy Bypass -File <脚本路径>`
它做什么：杀旧 askbridge → 启动 `target\release\askbridge.exe`（真实数据目录 D:\askbridge\data）→ FindWindow 找隐藏主窗类 `AskBridge.Desktop.HiddenWindow.v1` → PostMessage `WM_HOTKEY(0x0312)` wParam=`0x101`（快速截图，等价 Alt+Shift+Q，鼠标抬起即自动分发）→ 等 `AskBridge.CaptureOverlay.Window.v1` 出现 → SetCursorPos+mouse_event 模拟拖拽 → 轮询 `D:\askbridge\data\logs\askbridge.log` 等结果 → 全屏截图存 `%TEMP%\askbridge-paste-result.png`（可用 Read 工具直接看图验证图片是否贴上）。
注意：脚本会动真实鼠标约 5 秒，运行前告知用户。

实测结论：
- 日志时序注意：`Started` 等 Stage 事件由 UI 线程从事件队列取出后记录，worker 线程自己的日志可能早于它，别按文件行序推断先后。
- 失败时前台是 `Windows.UI.Core.CoreWindow`（UWP 窗口），普通 SFW/AttachThreadInput/SwitchToThisWindow 在**纯注入输入**的自动化场景下全部被拒——真实用户场景（有真实热键输入）大概率更好，但必须靠 `paste_foreground_denied` 的 `foreground_class` 字段判断。
- 找到的目标其实是 **ChatGPT 桌面客户端**（Chrome_WidgetWin_1 + 标题含 ChatGPT），不是浏览器——这正是用户要求支持桌面端的原因，也解释了此前所有"激活被拒/没贴上"。
- 项目有一个源码扫描测试（`logging.rs` 内）**禁止日志出现 `error = %error`** 等字段片段，属作者隐私设计，新增日志别违反；数值型诊断字段（如 shell_result）是允许的。
- 既有无关失败：xtask 的 `package_reports_a_command_failure_for_a_non_empty_root` 因系统 GBK 控制台编码 UTF-8 解码失败，与任何改动无关（已用 git stash 验证过 main 上同样失败）。

### windows-sys 0.61 的坑
- `BOOL` 不在 Foundation 里导出，就是 `i32`；EnumWindows 回调返回 `i32`。
- `AttachThreadInput` 在 **`Win32::System::Threading`**（不是 UI 模块！），参数是 `(u32, u32, i32)`。
- `COINIT_APARTMENTTHREADED` 是 i32，传给 CoInitializeEx 要 `as u32`。
- 字母虚拟键不导出，`VK_V: u16 = 0x56` 自己定义。
- INPUT 结构：`INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { .. } } }`。

## 6. 验证流程（每次改动后）

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked        # xtask 那个 GBK 失败可忽略
cargo build --workspace --release --locked
# release 构建前先 taskkill //IM askbridge.exe //F（exe 被运行中进程锁定）
```

全绿后跑 §5 的自动调试流程，看日志出现 `page preparation completed ... stage="page_preparation"` 且截图里 ChatGPT 输入框出现截图缩略图即为成功。成功路径日志链：`dispatch request prepared → paste_window_found(process=msedge.exe/chrome.exe/合法桌面客户端) → page preparation completed`。通用粘贴不应出现 `BrowserStage::Started`。

旧临时脚本把任意包含 `paste` 的日志都当成结果，可能被 `paste_window_skipped` 触发假阳性；运行时必须把最终判定收紧为 `page preparation completed|browser surface workflow failed`。保存验收截图前必须先询问用户位置，不要沿用脚本默认的 C 盘 `%TEMP%` 路径。

## 7. 手工验收清单（需转告用户）

1. 设置 → 浏览器 → 选「通用粘贴」→ **应用更改**（只点单选不生效）；确认 config.json `"chatgpt": "clipboard_paste"`。
2. 未登录的 ChatGPT 网页连手动上传都不允许，测试请用已登录的平台（豆包/Claude 或登录后的 ChatGPT）。
3. Alt+Q 框选 → Enter；观察是否自动聚焦目标窗口并出现截图。
4. overlay 重构的两个 commit 也顺带目视检查：框选拖拽流畅度、渲染外观与旧版一致。

## 8. 其他背景

- 用户 gh keyring token 曾提示失效但 API/push 实际可用；遇 403 让用户跑 `gh auth refresh`。
- 本会话模型服务偶发空响应导致任务多次中断，用户已被迫多次点"继续"——接手后尽量减少来回，把每轮做完再停。
- LICENSE Apache-2.0（署名 starry）、README_EN、CONTRIBUTING、Issue 模板、cargo-deny 均已在远端 main 上（commit 5c3fb3c 及之前）。
