# AskBridge 项目交接文档（历史阶段记录）

更新时间：2026-08-10
项目目录：`D:\AskBridge`
远端仓库：`https://github.com/wanghongyu666qiang/AskBridge.git`
当前分支：`main`
Phase 6 开发基线：Phase 5 已验收但仍位于 `main` 未提交工作区；最近已推送提交为 `4a97eb0 feat: complete Phase 4 browser targets and CDP`

> 当前 Phase 0–8 候选实现、2026-08-10 自动/真实验收证据和剩余阻塞项以 [`ACCEPTANCE_REPORT.md`](ACCEPTANCE_REPORT.md) 为准。本文后续内容保留阶段历史和旧环境证据；其中“下一步”“当前不包含”和旧版本号描述不再代表当前状态。

## 当前有效交接信息

### 1. 基线与范围

- 仓库内的 [`docs/DEVELOPMENT_SPEC.md`](DEVELOPMENT_SPEC.md) 是当前唯一开发基线。
- Phase 0–4 及后续托盘/截图稳定性修复已提交并推送；Phase 5 已完成实现和真实验收，Phase 6 实施边界见 [`PHASE_6_PLAN.md`](PHASE_6_PLAN.md)。
- 本轮额外修复真实 Windows Shell 托盘回调无法到达 Runtime，以及截图拖选时遮罩闪烁的问题。
- Phase 4 的桌面 PWA、专用 Chrome、CDP 和页面目标基础已经完成；Phase 5 的通用网页准备和人工兜底已验收；Phase 6 的四家真实网页文字准备和 ChatGPT 图片准备已通过，Gemini、Claude、豆包图片仍安全进入人工兜底，Windows UI 最终串联验收待完成。
- 用户确认 AskBridge 的全部运行数据留在 D 盘；当前开发工作区使用 `D:\AskBridge\data`。配置和 257 个专用 Chrome 配置条目已从旧 `%LOCALAPPDATA%` 位置非破坏性复制，旧 C 盘副本暂未删除。
- 本机 `Alt+A` 与现有截图工具冲突；用户确认将“直接文字提问”当前配置和项目默认值改为 `Alt+W`。
- 正式程序不使用 Electron、Tauri、Python、WebView、浏览器扩展或本地 HTTP 服务。

### 2. 当前 Git 状态

`main` 与 `origin/main` 当前均指向 `4a97eb0`。Phase 5 与当前 Phase 6 工作区均尚未提交、尚未推送。

Phase 5 修改：

- `README.md`
- `crates/askbridge-core/src/{browser,error,lib,preparation,workflow}.rs`
- `crates/askbridge-win/Cargo.toml`
- `crates/askbridge-win/{app.manifest,askbridge.rc,build.rs}`
- `crates/askbridge-win/src/{adapter,app,browser,capture,clipboard,fallback,main}.rs`
- `docs/DEVELOPMENT_SPEC.md`
- `docs/HANDOFF.md`
- `docs/PHASE_5_PLAN.md`

Phase 6 在上述未提交基线上继续修改：

- `crates/askbridge-core/src/{lib,preparation,provider}.rs`
- `crates/askbridge-win/src/adapter/{mod.rs,rules.rs,builtin_rules.json}`
- `crates/askbridge-win/src/{app,fallback}.rs`
- `crates/askbridge-win/src/browser/{cdp,mod,worker}.rs`
- `README.md`
- `docs/{DEVELOPMENT_SPEC,HANDOFF,PHASE_6_PLAN}.md`

### 3. Phase 4 当前实现

- `ChromeInstallation` 按用户配置、HKCU/HKLM App Paths 和常见安装位置发现 `chrome.exe`，不下载浏览器。
- 设置页新增可手动填写的 Chrome 完整路径；留空继续自动检测，保存前验证文件存在且名称为 `chrome.exe`。
- 设置页新增“ChatGPT 使用桌面网页端”开关；ChatGPT 默认启动用户桌面的 `ChatGPT.lnk`，其他供应商继续使用专用 Chrome。
- `BrowserTargetPreference` 和 `BrowserLaunch` 构成目标载体 seam；调用方只选择 `desktop_pwa` 或 `dedicated_chrome`，不依赖快捷方式、Windows Shell 或 CDP 实现细节。
- `DesktopPwaLauncher` 只接受存在的绝对 `.lnk`，可自动发现桌面 `ChatGPT.lnk`，并在后台通过 Windows Shell 启动；不读取 Cookie、密码、历史记录或网页正文。
- `ManagedProfile` 只允许 AskBridge 标记的新目录或已标记目录，拒绝 Chrome Stable/Beta/Canary 默认 `User Data`，也拒绝接管非空未标记目录。
- 运行数据路径由单一解析器确定：开发工作区默认使用仓库根目录 `data`，便携版本默认使用程序旁 `data`，绝对路径环境变量 `ASKBRIDGE_DATA_DIR` 可显式覆盖；当前路径为 `D:\AskBridge\data`。
- 所有启动参数集中在 `ChromeManager`：独立 `--user-data-dir`、动态 `--remote-debugging-port=0`、`--no-first-run` 和 `--no-default-browser-check`。
- `DevToolsActivePort` 只从专用目录读取，严格校验动态端口和浏览器路径；普通日志不记录端口。
- 本地调试 HTTP 和 WebSocket 只连接 IPv4、`localhost` 或 IPv6 回环，限制响应大小、请求方法、路径、URL 和目标 ID。
- `CdpClient` 完成 `/json/version` 与 `Browser.getVersion` 双重握手，支持目标列表、创建、激活和固定内置 `document.readyState` 就绪检查。
- `TargetResolver` 显式区分 `Confirmed(TargetId)` 与 `Unknown`；零匹配新建、唯一匹配复用、多匹配且无可靠焦点时新建。
- `BrowserService` 在后台线程执行启动、连接、一次重连、目标解析和页面等待，通过私有 Win32 消息将阶段事件返回 UI。
- `WorkflowController` 已覆盖专用 Chrome 的 `StartingBrowser`、`ConnectingBrowser`、`ResolvingTarget`、`WaitingForPage`，以及桌面 PWA 从 `StartingBrowser` 直接到 `PreparingPage` 的 Phase 5 交接边界。
- 默认生命周期按需启动并保持运行；`on_startup` 会后台预热；`on_demand_idle_close` 在十分钟空闲后只对本实例确认管理的进程请求正常关闭。
- `close_after_dispatch` 的最终关闭时机必须等 Phase 5 真正完成内容准备后执行，Phase 4 不会在用户尚未使用页面时提前关闭。
- 首次启动通知明确要求用户在专用 Chrome 中自行登录；不读取密码、验证码、Cookie、网页正文或存储。
- 请求仍只保存在当前进程内存；目标页就绪后显示 Phase 5 尚未接入，未执行网页输入、附件上传、剪贴板操作或自动发送。
- 隐藏主窗口现在会在收到直接 `WM_CLOSE` 时结束消息循环，并按顺序释放托盘、快捷键和浏览器工作线程；Windows `taskkill` 不会向这个隐藏窗口投递普通关闭，正式退出入口仍是托盘“退出”。
- Shell 托盘回调先在 `WndProc` 接收，再转发为独立队列消息交给 Runtime；不再错误地假设所有托盘回调都会由 `GetMessage` 返回。
- 托盘图标在 `NIM_ADD` 后协商 `NOTIFYICON_VERSION_4`，并按 `LOWORD(lParam)` 解码 v4 打包事件。
- 托盘回调、单实例激活、截图忙碌和 Runtime 转发使用互不重叠的私有消息编号。
- 截图拖动不再为每个 `WM_MOUSEMOVE` 强制同步整窗重绘；连续事件由 Windows 合并为延迟重绘，重复坐标不触发绘制。
- 遮罩、透明选区、边框和文字先在兼容内存位图中完成，再通过一次 `BitBlt` 显示，避免分层窗口暴露中间绘制帧。

### 4. Phase 5 当前实现

- `PreparationPolicy`、`PreparationOutcome` 和 `DispatchOutcome` 已进入 core，并通过受控构造和校验集中维护成功/兜底不变量。
- `WorkflowController` 已覆盖 `PreparingPage -> PreparedForUser` 与 `PreparingPage -> PreparingFallback -> FallbackReady`，支持重试、取消和清理回到空闲。
- `ProviderAdapter.prepare(...)` 是应用与页面差异之间的单一主要调用面；调用方不依赖定位、上传、输入或验证步骤。
- 专用 Chrome 通用 adapter 会重新确认目标和 URL，评分可见且可编辑的 textarea/contenteditable/role=textbox；低分或多个高分候选停止并进入兜底。
- 文字通过固定内置 CDP 输入程序触发正常 input 事件并回读验证；程序没有点击、按键发送或自动提交路径。
- 图片使用唯一且接受 PNG 的 `input[type=file]` 与 `DOM.setFileInputFiles`；缺失、禁用、多个候选或验证失败都会停止。
- 临时截图只写入 `D:\AskBridge\data\Temp`（或当前 AskBridge 数据目录的 `Temp`），随机文件名不包含问题，CDP 文件控件确认后立即删除；启动时只清理 24 小时前的 `askbridge-*.png`。
- 桌面 PWA 当前没有足够可靠的 UI Automation 证据，因此不猜窗口或控件，统一返回人工兜底。
- 人工兜底 Task Dialog 保留请求，提供“复制图片”“复制问题”“重试自动投递”“取消”；窗口存续期间修改剪贴板，关闭时尽力恢复 `CF_UNICODETEXT` 与 `CF_DIB`。
- Windows GNU 与 MSVC 构建都会嵌入 Common Controls v6 清单；这修复了 `TaskDialogIndirect` 静态导入在系统 comctl32 v5 下令程序进入 Rust `main` 前退出的问题。Debug 模式同时把启动错误写到 stderr，便于以后区分加载器失败和应用错误。
- 真实 Chrome ignored 集成路径已在 Chrome `150.0.7871.187` 上通过稳定本地页面的文字输入、PNG 文件控件准备、结果验证和清理。
- Phase 5 验收时尚未开始供应商专用规则；当前 Phase 6 已在其上继续实现，所有请求仍固定 `auto_submit=false`。

### 5. Phase 6 当前实现

- 2026-08-04 已从官方页面重新核验 ChatGPT、Gemini、Claude、豆包入口；编译时默认入口保持 `chatgpt.com`、`gemini.google.com/app`、`claude.ai/new` 和 `doubao.com/chat/`。ChatGPT、Gemini、Claude 的匿名登录跳转与规则一致；豆包仅确认同源 `/login`，动态 DOM 和后续 SSO 仍需真实浏览器验收。
- 新增带 `schema_version=2` 的只读内嵌规则文件，启动时校验四家覆盖完整、ID 唯一、登录模式为 HTTPS，选择器不含可执行脚本结构。
- `ProviderAdapter.prepare(...)` interface 不变；adapter implementation 内按 `adapter_override` 选择内置规则或通用模式。
- 供应商输入框和图片文件控件选择器优先；专用规则未命中时保留 Phase 5 通用候选评分和通用 PNG 文件控件回退。
- 明确登录 URL 返回 `LoginInBrowser` 人工兜底；专用规则和通用输入定位均失效时返回 `ProviderPageChanged`，对话框分别显示登录或网页改版提示。
- 页面准备日志不再输出包含 `target_url` 的完整结果对象，只记录布尔结果、失败阶段和恢复提示。
- SPA 页面会在配置的准备超时内以 100 ms 间隔等待文件控件、输入框或可见登录结构稳定；每次探测均先执行精确 URL 守卫，缺失状态不修改页面。轮询间隙至多每 25 ms 检查一次取消，单次本地 CDP 探测上限为 750 ms，探测返回时会优先保留取消语义。
- 文件控件出现前若 SPA 在同一受信供应商边界内完成一次路由切换，adapter 会重新取得当前 target 与精确 URL 后重试一次；登录 URL 或跨供应商导航仍立即停止，不会沿用旧 URL 写入。
- 所有路径继续固定 `auto_submit=false`；规则不含发送按钮动作，不从网络更新，不读取或记录网页正文。
- Chrome `151.0.7922.77` 与 `D:\AskBridge\data\BrowserProfile` 的真实 provider harness 已确认 ChatGPT、Gemini、Claude、豆包均能把非敏感测试文字写入编辑区，且没有自动发送；ChatGPT 另确认 1×1 PNG 与文字同时准备成功。
- Gemini 只暴露“上传和工具”菜单而未直接暴露唯一文件控件；Claude 的唯一文件控件在同站 SPA 路由切换后不稳定；豆包未发现可证明唯一的附件入口。三者均按既有安全边界返回 `CopyImageThenText`，没有猜控件或放宽 URL 守卫。

### 6. 自动检查与构建产物

当前 Phase 6 工作区新增门禁结果：

- `cargo fmt --all -- --check`：通过；
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过；
- `cargo test --workspace --offline`：124 个通过，0 个失败，2 个真实 Chrome 测试默认忽略；
- `cargo test -p askbridge-win dedicated_chrome_cdp_round_trip --offline -- --ignored --nocapture`：1 个通过，真实 Chrome 正常关闭，D 盘临时配置已清理；
- `cargo build --workspace --offline`：通过；
- `cargo build --workspace --release --offline`：通过；
- `git diff --check`：通过；
- GNU 链接器仍只有既有的非致命 `.drectve` warning。

| Phase 6 构建产物 | 大小 | SHA-256 |
| --- | ---: | --- |
| `target\debug\askbridge.exe` | 34,110,865 bytes | `CF318ECCFCC03AD05E6A3064E0A2E0BC4D23D5B51E642045F1013B10DED4FDB5` |
| `target\release\askbridge.exe` | 1,137,664 bytes | `3687075641D89243AF12EB7F51191DBC37DE555B340959F22C9038EA8D1196B5` |

以下 Phase 5 结果保留为上一阶段验收证据：

以下检查已在当前 Phase 5 工作区上通过：

- `cargo fmt --all -- --check`：通过；
- `cargo clippy --workspace --all-targets --offline -- -D warnings`：通过；
- `cargo test --workspace --offline`：95 个通过，0 个失败，1 个实机 Chrome 测试默认忽略；
- `cargo build --workspace --offline`：通过；GNU 链接器仍输出既有的非致命 `.drectve` 警告；
- `cargo build --workspace --release --offline`：通过。

| 产物 | 大小 | SHA-256 |
| --- | ---: | --- |
| `target\debug\askbridge.exe` | 33,483,475 bytes | `EAADE013D45403F9E73D6315F7588F75496D13457FAB4977283237A965306C57` |
| `target\release\askbridge.exe` | 1,102,848 bytes | `2B76C0ABD5223CE5F575E8CB1604B184660FD9AF1B2BBC0FD22FAA4CFA8AF910` |

真实 Windows 11 验证：

- 差分最小程序证明 Explorer 和物理输入路径正常；
- 自动探针先打开隐藏图标面板，再按 `Shell_NotifyIconGetRect` 定位实际 AskBridge 图标并执行真实右键；
- 修复前稳定无回调，修复后收到 v4 打包的右键按下/释放事件并成功创建菜单；
- 回归测试覆盖非队列 Shell 回调从 `WndProc` 转发到 Runtime 队列；
- 所有临时探针、调试日志和可执行文件已删除。
- 用户已在真实桌面按 `Alt+Q` 连续移动鼠标框选，确认修复后不再闪烁。
- 实机 CDP 测试使用 Chrome `150.0.7871.187`、临时专用配置和 `127.0.0.1` 随机端口，已通过动态端点、浏览器握手、目标创建、激活、交互就绪、目标列表复核与 `Browser.close` 正常关闭。
- 每次成功或失败的实机测试均清理临时 Chrome 和临时配置目录；没有连接日常 Chrome 配置。
- Computer Use 已显示检查新版设置窗口，确认 Chrome 路径输入、说明文字、按钮和状态栏布局正常。
- 用户已完成新的真实桌面流程：`Alt+W` 输入非敏感问题并确认后，AskBridge 成功打开现有 ChatGPT PWA，并复用其中的登录状态；没有再启动无法登录的专用 ChatGPT Chrome。
- Phase 5 真实 Chrome ignored 测试通过：隔离临时配置、回环测试页、textarea 写入、1×1 PNG 文件上传、结果验证、浏览器关闭和临时目录清理均成功；失败的沙箱内 GPU 启动尝试也完成进程与配置清理。
- 新清单嵌入后，Debug 实机运行保持存活且 `Alt+W` 能打开问题窗口；Release 也在启动 2 秒后保持存活。Debug/Release 二进制均检测到 Common Controls v6 清单标记。
- 桌面 PWA 文字人工兜底实机通过：自动复制内容哈希匹配，“复制问题”保持对话框打开且内容匹配，“重试自动投递”重新创建对话框并保留请求，两次取消后均恢复到验收前无 Unicode 文本的剪贴板状态；全程未自动发送。
- 验收发现全局热键创建遮罩后会收到正常 `WM_KILLFOCUS`，旧逻辑将其误判为取消，导致真实框选后直接消失。取消原因日志精确确认 `reason=KillFocus` 后，修复为焦点切换不取消；`Esc`、右键、关闭和 `WM_CANCELMODE` 仍保留显式取消，并增加回归测试。
- 修复后用户以真实鼠标完成 `Alt+Q` 框选，日志确认截图只在内存中完成；图片人工兜底自动复制位图、“复制问题”切到文字、“复制图片”恢复为同一位图，且对话框保持打开。取消后恢复为原先的有位图、无 Unicode 文字格式，恢复位图哈希不同于本次截图；全程未自动发送。
- 2026-08-09 真实四家网页文字验收全部通过，均回报 `text_inserted=true`；ChatGPT 的 1×1 PNG 验收同时回报 `attachment_prepared=true`。失败/兜底轮次均正常关闭专用 Chrome，`D:\AskBridge\data\BrowserProfile\Temp` 未留下 `askbridge-*.png`。
- 真实 profile 仍会输出既有 Service Worker、GCM、IndexedDB/WebStorage `LOCK`/IO 告警，但未阻止四家文字准备；这些告警不能作为图片准备成功证据，也未通过禁用 sandbox/GPU 等生产参数规避。
- 验收用 Debug AskBridge 进程已停止，以解除最终构建的文件锁。

### 7. 下一步

1. 使用真实 AskBridge 热键/UI 串联复核四家文字路径与 Gemini、Claude、豆包的 `CopyImageThenText` 对话框提示；CDP harness 已验证网页写入，但不能替代 Windows UI 串联验收。
2. 若以后要自动处理 Gemini/豆包图片，必须先取得唯一附件触发器和触发后唯一文件控件的真实结构证据；Claude 还需解决同站路由后的控件稳定性。没有证据时保留人工兜底。
3. 最终 Windows UI 串联验收完成前不提交、不推送；任何情况下均不进入 Phase 7。

---

# Phase 2 交接快照（历史，仅供追溯）

更新时间：2026-07-29
项目目录：`D:\AskBridge`
远端仓库：`https://github.com/wanghongyu666qiang/AskBridge.git`
当前分支：`main`
最后已推送提交：`4ada778 feat: initialize AskBridge Phase 0 and Phase 1`

## 当前有效交接信息

### 1. 基线与范围

- 仓库内的 [`docs/DEVELOPMENT_SPEC.md`](DEVELOPMENT_SPEC.md) 是当前唯一开发基线。
- 外部 `D:\Obsidian Vault\DEVELOPMENT_SPEC.md` 是本轮输入来源，不再作为后续实现直接读取的第二基线。
- 本轮只校正并验收现有 Phase 0–2，没有开始 Phase 3。
- 正式程序不使用 Electron、Tauri、Python、WebView、浏览器扩展或本地 HTTP 服务。
- 后续自动化测试可以按开发规格第 2.4 节仅临时监听 `127.0.0.1` 随机端口。

### 2. 当前 Git 状态

`main` 与 `origin/main` 仍指向 `4ada778`。所有 v2.1 对齐和 Phase 2 修改均尚未提交、尚未推送。

当前已修改的跟踪文件：

- `Cargo.lock`
- `Cargo.toml`
- `README.md`
- `crates/askbridge-core/src/config.rs`
- `crates/askbridge-core/src/config_store.rs`
- `crates/askbridge-core/src/error.rs`
- `crates/askbridge-core/src/lib.rs`
- `crates/askbridge-core/src/provider.rs`
- `crates/askbridge-core/src/state.rs`
- `crates/askbridge-win/Cargo.toml`
- `crates/askbridge-win/src/app.rs`
- `crates/askbridge-win/src/hotkey_manager.rs`
- `crates/askbridge-win/src/main.rs`

当前新增、未跟踪文件：

- `crates/askbridge-core/src/capture.rs`
- `crates/askbridge-win/src/capture/encoder.rs`
- `crates/askbridge-win/src/capture/mod.rs`
- `crates/askbridge-win/src/capture/monitor.rs`
- `crates/askbridge-win/src/capture/overlay.rs`
- `crates/askbridge-win/src/capture/screen.rs`
- `docs/DEVELOPMENT_SPEC.md`
- `docs/HANDOFF.md`
- `docs/PHASE_2_PLAN.md`

当前没有残留的 `askbridge.exe` 进程。

### 3. v2.1 对齐内容

- 配置 schema 从 v1 升级到 v3。
- 旧配置在加载时迁移并原子保存；损坏配置仍先备份再恢复默认。
- 内置供应商来自编译时默认值，用户配置只保存 `provider_overrides` 与 `custom_providers`。
- `general.auto_submit` 在 1.0 中强制为 `false`；旧配置中的 `true` 会被归一化并写回。
- 浏览器配置改为专用 Chrome 路径、独立配置目录、生命周期和连接/页面超时。
- 快捷键管理器现在使用可测试的后端边界、候选临时 ID、`MOD_NOREPEAT`、注销失败回滚和 ID 回收。
- 注册失败、注销失败或配置保存失败时保留旧绑定和旧映射。
- `CapturedImage` 按开发规格保存严格校验长度的 RGBA 缓冲区。
- 保留独立的内存 PNG 编码能力，但截图成功路径不再写剪贴板。
- 已移除 Phase 2 的 `CF_DIB` 剪贴板模块及对应 Windows feature。

### 4. Phase 2 当前实现

应用层接口仍为：

```rust
CaptureService::capture() -> Result<CaptureOutcome>
```

当前链路：

```text
全局截图快捷键
  -> 显示器和虚拟桌面枚举
  -> 原生半透明选区遮罩
  -> 隐藏遮罩并同步桌面合成
  -> GDI 捕获物理屏幕像素
  -> BGRA 转 RGBA
  -> 返回内存 CapturedImage
```

成功和取消都不修改剪贴板，也不把截图写入磁盘。PNG 编码器作为独立能力由自动化测试覆盖，供后续网页文件上传流程使用。

### 5. 自动检查结果

工具链：

```text
rustc 1.97.1
cargo 1.97.1
workspace rust-version = 1.85
```

以下命令已在 v2.1 对齐后的最终源码上通过：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
cargo build --workspace --release
```

测试结果：

- `askbridge-core`：24 个通过；
- `askbridge-win`：11 个通过；
- 总计：35 个通过，0 个失败。

当前构建产物：

| 产物 | 大小 | SHA-256 |
| --- | ---: | --- |
| `target\debug\askbridge.exe` | 20,927,304 bytes | `81B617E0173DB51222BB17FEB1ED8CB3DC25C8B5861C99F093522FAD65ED7594` |
| `target\release\askbridge.exe` | 603,136 bytes | `493A8E1E894E0DBB10565CE9E1D307883D1B13C30FA893F3FC426637A953B1ED` |

`artifacts\AskBridge-0.1.0\askbridge.exe` 仍是旧的 Phase 0/1 包，不能作为当前交付物。

### 6. 已完成的桌面端验证

已实际启动 `target\debug\askbridge.exe` 并确认：

- 旧配置被迁移为 schema v3，`auto_submit` 为 `false`；
- 第二实例能够唤起现有实例的设置窗口；
- 设置窗口布局正常；
- 本机 `Alt+A` 外部占用仍被准确报告为 Windows 错误 1409；
- `Alt+Q` 能显示覆盖虚拟桌面的截图遮罩；
- `Alt+Shift+Q` 能进入同一截图遮罩链路；
- 遮罩显示操作说明，真实桌面内容在遮罩下可见；
- 验证结束后已停止调试进程，没有残留托盘实例或全局快捷键。

尚未冒充完成的项目：

- Computer Use 无法把无边框工具遮罩作为输入目标，拖选时返回：

```text
point (...) is over ChatGPT.exe "Chrome Legacy Window",
not target window askbridge.exe "AskBridge 设置"
```

- 因此尚未自动闭环正向/反向拖选后的成功捕获；
- 尚未人工确认成功和取消均不修改原剪贴板；
- 尚未完成人工双屏、负坐标副屏和 125%/150% 混合 DPI 矩阵；
- 尚未刷新 `artifacts\AskBridge-0.1.0` 便携包。

### 7. 接手后的下一步

1. 人工运行 `target\debug\askbridge.exe`。
2. 在剪贴板预先放置可识别内容。
3. 用 `Alt+Q` 分别完成正向和反向非零拖选。
4. 确认选区透明、边框可见、尺寸实时更新和“截图已捕获”提示正确。
5. 确认成功捕获后原剪贴板内容仍然不变。
6. 分别用 `Esc`、右键和失焦取消，确认原剪贴板仍不变。
7. 用 `Alt+Shift+Q` 重复一次完整拖选。
8. 在可用硬件上补测负坐标副屏、跨屏和 125%/150% 混合 DPI。
9. 全部通过后运行 `scripts\package.ps1`，重新记录便携包大小和 SHA-256。
10. 最后审查 `git diff --check`、范围和文档，再决定提交与推送。

Phase 2 人工验收和便携包刷新完成前不要进入 Phase 3。

---

# 附录：v2.1 对齐前的历史交接快照（仅供追溯）

以下内容描述的是本轮对齐之前的状态，若与上方“当前有效交接信息”冲突，以上方内容和 `docs/DEVELOPMENT_SPEC.md` 为准。

## 1. 项目目标与基线

AskBridge 是面向 Windows 10/11 的超轻量桌面快捷操作层。功能和架构以开发规格为基线：

- 原始规格：`D:\Obsidian Vault\DEVELOPMENT_SPEC.md`
- 仓库中目前另有一份未跟踪的 `DEVELOPMENT_SPEC.md`，提交前应决定是否保留并纳入版本控制。
- 项目必须保存在 D 盘；Rust/Cargo 工具链安装在 C 盘不影响这一约束。
- 禁止 Electron、Tauri、Python、WebView、本地 HTTP 服务和内嵌浏览器。
- 当前实现采用 Rust workspace、原生 Win32、GDI 和 Windows Clipboard API。

## 2. 当前 Git 状态

`main` 与 `origin/main` 均指向 `4ada778`。Phase 2 修改尚未提交、尚未推送。

当前已修改文件：

- `Cargo.lock`
- `Cargo.toml`
- `README.md`
- `crates/askbridge-core/src/lib.rs`
- `crates/askbridge-win/Cargo.toml`
- `crates/askbridge-win/src/app.rs`
- `crates/askbridge-win/src/main.rs`

当前新增、未跟踪文件：

- `DEVELOPMENT_SPEC.md`
- `crates/askbridge-core/src/capture.rs`
- `crates/askbridge-win/src/capture/mod.rs`
- `crates/askbridge-win/src/capture/monitor.rs`
- `crates/askbridge-win/src/capture/overlay.rs`
- `crates/askbridge-win/src/capture/screen.rs`
- `crates/askbridge-win/src/capture/encoder.rs`
- `crates/askbridge-win/src/clipboard/mod.rs`
- `docs/PHASE_2_PLAN.md`
- `docs/HANDOFF.md`

交接时没有残留的 `askbridge.exe` 进程。

## 3. 已完成并已验证：Phase 0 / Phase 1

下列能力已完成、通过构建测试，并包含在最后已推送提交中：

- Rust workspace：`askbridge-core` 与 `askbridge-win`
- 核心配置模型、统一错误类型和配置仓储
- `%LOCALAPPDATA%\AskBridge\config.json` 保存与加载
- 损坏配置备份及恢复默认值
- 当前用户会话内的命名单实例互斥体
- 原生 Windows 系统托盘
- 三个默认全局快捷键：
  - `Alt+Q`：截图并提问
  - `Alt+Shift+Q`：截图快速投递
  - `Alt+A`：直接文字提问
- 快捷键可修改、禁用、恢复默认，并在应用后立即生效
- 内部重复、缺少修饰键、危险系统组合和外部占用检测
- 注册新快捷键或保存配置失败时保留原配置和旧注册
- 最小原生设置窗口及必要构建、测试、打包脚本
- 设置窗口的 DPI、字体、边框和按钮视觉优化

Phase 0/1 的最终检查曾全部通过：

- `cargo fmt`
- 严格 `cargo clippy`
- 15 个单元测试
- Debug build
- Release build

环境中 `Alt+A` 被其他进程占用时，系统会返回 Win32 错误 1409。应用已正确识别冲突并保留原配置；这属于本机快捷键占用，不是 AskBridge 内部重复检测失效。

## 4. 已实现但待最终验收：Phase 2

Phase 2 的代码级实现已落地：

- 多显示器枚举与虚拟桌面边界
- 负坐标显示器支持
- Per-Monitor V2 DPI 下的物理坐标处理
- 覆盖虚拟桌面的原生半透明截图遮罩
- 鼠标正向和反向拖选
- 选区透明显示、边框和实时尺寸标签
- `Esc`、右键和失焦取消
- 隐藏遮罩并同步桌面合成后再捕获，避免遮罩进入截图
- GDI 屏幕像素捕获
- BGRA 到 RGBA 转换和内存 PNG 编码
- `CF_DIB` 剪贴板图像写入
- Win32/GDI/全局内存资源的 RAII 清理
- `Alt+Q` 和 `Alt+Shift+Q` 接入真实截图事件路由
- 结构化日志仅记录命令、状态、尺寸和 PNG 字节数，不记录像素内容

Phase 2 当前状态必须理解为：

> 实现已完成，最终验证尚未闭环。

在最后一轮细节调整之前，严格 clippy、测试、Debug 和 Release build 曾通过，当时共 27 个测试（core 20、win 7）。之后又增加了 PNG 文件头测试并调整了显示器错误信息、日志字段和遮罩窗口样式；这些最终改动只重新执行了 `cargo fmt` 与 `cargo check --workspace --all-targets`，两者通过。完整五项命令需要重新执行，不能沿用此前结果。

## 5. Phase 2 架构

应用层只调用一个较深的接口：

```rust
CaptureService::capture() -> Result<CaptureOutcome>
```

事件流：

```text
全局快捷键
  -> CaptureService
  -> 显示器/虚拟桌面枚举
  -> 原生选区遮罩
  -> GDI 像素捕获
  -> 内存 PNG 编码
  -> CF_DIB 剪贴板写入
  -> 轻量提示与结构化日志
```

模块职责：

| 文件 | 职责 |
| --- | --- |
| `askbridge-core/src/capture.rs` | `ScreenRect`、`CapturedImage` 和纯逻辑校验 |
| `capture/monitor.rs` | 显示器枚举、虚拟桌面几何 |
| `capture/overlay.rs` | 遮罩窗口、拖选状态、取消流程 |
| `capture/screen.rs` | GDI 捕获与资源所有权 |
| `capture/encoder.rs` | BGRA → RGBA → PNG 内存编码 |
| `clipboard/mod.rs` | DIB 构造与剪贴板所有权转移 |
| `capture/mod.rs` | 对应用层暴露的截图服务编排 |

Win32 handle 和 GDI 对象不会进入 `askbridge-core`。核心 crate 只暴露跨平台的数据结构和规则。

## 6. 已有手工验证证据

已实际启动 Debug 应用并通过全局 `Alt+Q` 触发截图：

- 半透明遮罩成功覆盖当前虚拟桌面。
- 真实桌面内容在遮罩下可见。
- 顶部操作说明正确显示。
- 第二实例能够唤起已有实例的设置窗口。

尚未完成的手工闭环：

- 自动化工具无法把无边框 `WS_EX_TOOLWINDOW` 遮罩枚举为可操作目标，因此没有完成自动拖选。
- 尚未人工确认“拖选 → 遮罩隐藏 → GDI 捕获 → PNG 编码 → `CF_DIB` 剪贴板 → 在画图中粘贴”的完整链路。
- 尚未在双显示器、左侧负坐标副屏及 125%/150% 不同缩放组合下完成矩阵验收。

曾用于诊断的临时标题栏样式已经撤销。当前源码恢复为正式的无边框：

```text
WS_POPUP + WS_EX_LAYERED + WS_EX_TOPMOST + WS_EX_TOOLWINDOW
```

## 7. 接手后应立即执行

### 7.1 准备构建环境

```powershell
Set-Location D:\AskBridge
$env:PATH = 'C:\Users\why17\.cargo\bin;D:\gw\MinGW-w64-x86_64-8.1.0-release-win32-seh-rt_v6-rev0\mingw64\bin;' + $env:PATH
```

当前可用工具链为 Rust stable GNU，实测 `rustc 1.97.1`。workspace 的最低 Rust 版本声明为 1.85。

### 7.2 重新执行最终五项检查

```powershell
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
cargo build --workspace --release
```

预期最终测试总数应至少为 28；以实际命令输出为准。任何失败都应先修复，再继续打包和提交。

MinGW 8.1 可能输出：

```text
corrupt .drectve at end of def file
```

此前该信息为非致命链接器警告，构建仍成功。仍需以本轮命令退出码和产物为准。

### 7.3 完成 Phase 2 手工验收

1. 运行 `.\target\debug\askbridge.exe`。
2. 按 `Alt+Q`，从两个方向分别拖选非零区域。
3. 确认选区透明、边框可见、尺寸实时更新。
4. 完成选区后，确认立即出现“截图已复制”提示。
5. 打开画图或其他支持图片粘贴的应用，按 `Ctrl+V`。
6. 确认粘贴尺寸与选区一致，且截图不包含遮罩。
7. 在剪贴板预先放置可识别内容，然后分别用 `Esc` 和右键取消；确认剪贴板不变。
8. 验证 `Alt+Shift+Q` 进入同一截图链路。
9. 确认 `Alt+A` 仍只显示 Phase 3 尚未实现的轻量提示。
10. 如有对应硬件，补测 125%/150% 缩放、左侧负坐标副屏和跨屏选区。

### 7.4 刷新 Release 包

当前 `artifacts\AskBridge-0.1.0\askbridge.exe` 是 Phase 0/1 时生成的旧包，不能作为 Phase 2 交付物。

五项检查和手工验收通过后执行：

```powershell
.\scripts\package.ps1
Get-FileHash -LiteralPath .\artifacts\AskBridge-0.1.0\askbridge.exe -Algorithm SHA256
```

记录新文件大小和 SHA-256，不要沿用旧哈希。

### 7.5 审查、提交并推送

```powershell
git status --short
git diff --check
git diff --stat
```

确认只包含预期文件后再暂存。不要在未决定前盲目加入仓库根目录的 `DEVELOPMENT_SPEC.md`。

建议提交信息：

```text
feat: implement Phase 2 screen capture
```

然后执行：

```powershell
git push origin main
```

## 8. 当前限制

- 尚未实现 Phase 3 的问题输入窗口、Provider 选择和命令编排。
- 尚未实现 Phase 4 的浏览器启动、窗口激活、自动粘贴和剪贴板恢复。
- 尚未实现浏览器扩展。
- 截图只存在于进程内存和系统剪贴板，不写入项目目录或临时文件。
- 成功截图会替换用户当前剪贴板内容；恢复原剪贴板属于后续阶段。
- 当前只写入 `CF_DIB` 图像；PNG 字节保存在内存中供后续流程使用。
- 不使用网络，也没有本地 HTTP 服务。
- 多显示器和混合 DPI 代码路径已实现，但硬件矩阵仍需人工验证。

## 9. 安全与隐私约束

后续修改必须继续保持：

- 不把截图像素、PNG 内容或用户问题写入日志。
- 不把截图落盘，除非未来规格明确要求并获得用户授权。
- 取消截图时不调用 `EmptyClipboard`，保持用户剪贴板不变。
- `SetClipboardData` 成功后由系统拥有全局内存，应用不得再次释放。
- GDI DC、bitmap、选中对象和窗口资源必须在所有错误路径上释放。
- 不引入 Electron、Tauri、Python、WebView 或本地 HTTP 服务。

## 10. 下一阶段建议

只有在 Phase 2 的五项命令、端到端剪贴板验收和 Release 包刷新全部完成后，再进入 Phase 3。Phase 3 建议按以下顺序推进：

1. 在 `askbridge-core` 定义提问请求、来源命令和 Provider 路由模型。
2. 实现独立于 Win32 的状态机与单元测试。
3. 增加轻量原生问题输入窗口。
4. 将三种快捷键分别路由为“截图并提问”“截图快速投递”“直接文字提问”。
5. 保持浏览器启动、网页自动粘贴和扩展逻辑继续在 Phase 4 之外。
