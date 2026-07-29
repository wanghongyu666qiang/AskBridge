# AskBridge 项目交接文档（Phase 3）

更新时间：2026-07-29
项目目录：`D:\AskBridge`
远端仓库：`https://github.com/wanghongyu666qiang/AskBridge.git`
当前分支：`main`
最后已推送提交：`92ad103 feat: align v2.1 and implement Phase 2 capture`

## 当前有效交接信息

### 1. 基线与范围

- 仓库内的 [`docs/DEVELOPMENT_SPEC.md`](DEVELOPMENT_SPEC.md) 是当前唯一开发基线。
- Phase 0–2 和 v2.1 校正已提交并推送到 `main` 与 `origin/main`。
- 用户已明确授权进入 Phase 3；当前实现范围见 [`PHASE_3_PLAN.md`](PHASE_3_PLAN.md)。
- 本轮只实现原生问题窗口、三种请求工作流、`DispatchRequest` 和并发状态保护。
- Phase 4 的专用 Chrome、CDP、页面目标和网页投递尚未开始。
- 正式程序不使用 Electron、Tauri、Python、WebView、浏览器扩展或本地 HTTP 服务。

### 2. 当前 Git 状态

`main` 与 `origin/main` 的已推送基线为 `92ad103`。Phase 3 当前在工作树中实现，尚未提交、尚未推送。

当前 Phase 3 修改：

- `README.md`
- `crates/askbridge-core/src/capture.rs`
- `crates/askbridge-core/src/error.rs`
- `crates/askbridge-core/src/lib.rs`
- `crates/askbridge-core/src/request.rs`
- `crates/askbridge-core/src/workflow.rs`
- `crates/askbridge-win/src/app.rs`
- `crates/askbridge-win/src/capture/mod.rs`
- `crates/askbridge-win/src/capture/overlay.rs`
- `crates/askbridge-win/src/main.rs`
- `crates/askbridge-win/src/prompt.rs`
- `docs/DEVELOPMENT_SPEC.md`
- `docs/HANDOFF.md`
- `docs/PHASE_3_PLAN.md`

### 3. Phase 3 当前实现

- `DispatchMode` 将三条入口映射为截图提问、默认问题截图和纯文字模式。
- `DispatchRequest` 校验 ID、供应商、问题和图片不变量。
- `auto_submit` 在构造和反序列化时均固定为 `false`。
- `WorkflowController` 约束 `Idle`、`SelectingRegion`、`Prompting`、`PreparingDispatch`、取消和错误恢复。
- 原生 Win32 问题窗口包含启用供应商下拉框、多行输入、继续和取消。
- `Enter` 继续，`Shift+Enter` 换行，`Esc` 取消，`Tab` 切换焦点；供应商下拉框保留标准方向键行为。
- 截图提问保留内存 `CapturedImage` 直到用户完成问题；快速截图使用默认供应商和 `quick_prompt`；文字入口不创建图片。
- 已有问题窗口会被置前；其余忙碌状态拒绝启动第二个工作流。
- Phase 3 请求交接只记录请求 ID、模式、供应商、是否带图和 `auto_submit=false`，不记录问题或图片内容。
- 请求准备后显示 Phase 4 尚未接入的提示并安全回到 `Idle`。

### 4. 自动检查与构建产物

以下检查已在当前 Phase 3 源码上通过：

- `cargo fmt --all -- --check`：通过；
- `cargo clippy --workspace --all-targets -- -D warnings`：通过；
- `cargo test --workspace`：43 个通过，0 个失败；
- `cargo build --workspace`：通过；
- `cargo build --workspace --release`：通过。

| 产物 | 大小 | SHA-256 |
| --- | ---: | --- |
| `target\debug\askbridge.exe` | 21,181,881 bytes | `3AE9AC2AE1D70F3E8B3C5DE3E9FC1590A8817E9BAAEC1BD3D2F1EFC8C8403920` |
| `target\release\askbridge.exe` | 621,568 bytes | `3C19CD6411D3550083C560C583C61DEBDFF8ADD6166A329D32A42F0069B1F615` |

当前没有残留的 `askbridge.exe` 进程。桌面端手工验收尚未执行，不能提前标记完成。

### 5. 下一步

1. 审查 Phase 3 diff 和隐私边界。
2. 在获得桌面启动许可后，手工验证问题窗口、键盘操作和三条入口。
3. 更新本交接文档中的手工结果。
4. 输出 Phase 3 检查点并停止；没有用户新授权不得进入 Phase 4。

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
