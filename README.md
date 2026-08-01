# AskBridge

AskBridge 是一个面向 Windows 10/11 的超轻量桌面快捷操作层。当前仓库实现开发基线中的 Phase 0 至 Phase 4：原生 Win32 托盘、全局快捷键、配置持久化、单实例、区域截图、问题输入、请求工作流、现有桌面 PWA 复用，以及隔离的专用 Chrome 与 CDP 基础设施。

当前功能和架构基线见 [`docs/DEVELOPMENT_SPEC.md`](docs/DEVELOPMENT_SPEC.md)。

## 当前能力

- 启动后仅驻留系统托盘，不创建可见主窗口。
- 注册三个默认全局快捷键：
  - `Alt+Q`：截图并提问；
  - `Alt+Shift+Q`：截图快速投递；
  - `Alt+W`：直接文字提问。
- 快捷键可在托盘“设置…”中修改、禁用、恢复默认，并在应用后立即生效。
- 修改快捷键时先占用新组合；系统注册或配置保存失败时保留旧组合。
- 检测 AskBridge 内部重复、缺少修饰键、危险系统组合和被其他程序占用的组合。
- 配置使用 schema v3 并保存至 AskBridge 数据目录中的 `config.json`；开发工作区默认使用仓库根目录下的 `data`，便携版本默认使用程序旁的 `data`，也可通过绝对路径环境变量 `ASKBRIDGE_DATA_DIR` 覆盖。
- 使用当前用户会话的命名互斥体保证只有一个托盘实例；第二次启动会通知已有实例打开设置。
- `Alt+Q` 和 `Alt+Shift+Q` 会打开覆盖虚拟桌面的原生区域选择遮罩。
- 截图选择支持反向拖动、负坐标显示器、当前尺寸提示、`Esc`/右键取消和高 DPI。
- 确认选区后先隐藏遮罩并同步桌面合成，再通过 GDI 捕获实际屏幕像素，遮罩不会进入截图。
- 截图以 RGBA 像素保存在内存中；独立 PNG 编码器供后续网页上传流程使用。
- 截图成功或取消都不会修改系统剪贴板，也不会把截图落盘。
- `Alt+Q` 截图后打开原生问题窗口，`Alt+W` 直接打开同一文字提问入口。
- 问题窗口支持启用供应商选择、多行文本、`Enter` 继续、`Shift+Enter` 换行、`Esc` 取消和 `Tab` 焦点切换。
- `Alt+Shift+Q` 使用默认供应商和默认问题直接准备请求，不显示问题窗口。
- 三条入口统一构造 `DispatchRequest`，并由状态机保证同一时刻只有一个主要工作流。
- AskBridge 1.0 始终将 `auto_submit` 固定为 `false`。
- ChatGPT 默认打开用户桌面的 `ChatGPT.lnk`，复用该 PWA 已有的登录状态；设置页可以关闭此模式。
- 桌面 PWA adapter 只发现、校验并启动用户选择的 `.lnk`，不读取日常 Chrome 的 Cookie、历史记录、密码或网页正文。
- 其他供应商以及关闭桌面 PWA 模式后的 ChatGPT 使用 AskBridge 专用 Chrome；该 Chrome 使用 AskBridge 数据目录中的 `BrowserProfile`。
- Chrome 默认按注册信息和常见位置自动检测，也可在设置页手动填写 `chrome.exe` 完整路径。
- 调试端口由 Chrome 动态分配；AskBridge 只读取专用目录里的运行时端点，并只连接回环地址。
- 后台浏览器工作线程执行 CDP 握手、目标枚举、唯一选择、创建、激活和页面就绪等待，不阻塞托盘 UI。
- 首次使用专用 Chrome 时用户自行登录；桌面 PWA 模式直接复用现有会话。AskBridge 不读取密码、验证码、Cookie 或网页正文。
- Phase 4 在桌面 PWA 成功打开或专用 Chrome 目标页面就绪后停止；网页输入、附件上传和内容准备属于 Phase 5。
- 结构化日志只记录命令、状态、请求 ID、供应商、尺寸和是否带图，不记录问题原文或截图内容。

## 当前明确不包含

Phase 0–4 不实现网页适配器、输入框发现、附件上传、文字插入、剪贴板兜底、自动发送、开机启动和安装器。浏览器扩展不属于正式架构。项目不使用 Electron、Tauri、Python、WebView 或内嵌浏览器；正式程序不启动本地 HTTP 服务，测试进程只按开发规格临时监听 `127.0.0.1` 随机端口。

## 架构

```text
askbridge-core
  领域命令、配置模型、Provider、DispatchRequest、工作流状态机、
  目标选择规则、快捷键解析/校验、统一错误、配置仓储

askbridge-win
  Win32 消息循环、单实例、RegisterHotKey、系统托盘、原生设置窗口、
  原生问题窗口、多显示器枚举、截图遮罩、GDI 屏幕捕获、
  RGBA 转换、PNG 编码、桌面 PWA 快捷方式 adapter、Chrome 生命周期、
  回环 HTTP/WebSocket CDP 和后台目标编排
```

核心配置和规则不依赖 UI；Windows handle 由对应的 RAII 对象释放。常驻路径没有轮询、网络请求或高频计时器。

## 构建

需要 stable Rust 和 Windows GNU 或 MSVC 构建链。

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
cargo build --workspace --release
```

也可以使用：

```powershell
.\scripts\build.ps1
.\scripts\test.ps1
.\scripts\package.ps1
```

`package.ps1` 目前只生成便携目录 `artifacts\AskBridge-0.1.0`；正式安装器属于后续发布阶段。

## 手工验收 Phase 1

1. 运行 `target\debug\askbridge.exe`。
2. 分别按 `Alt+Q`、`Alt+Shift+Q`、`Alt+W`，确认托盘通知中的命令名称正确。
3. 从托盘打开设置，修改一个快捷键并应用，确认旧组合失效、新组合立即生效。
4. 尝试重复组合、`Ctrl+C` 或已被系统占用的组合，确认配置不生效且原组合仍可用。
5. 取消“启用”后应用，再使用“恢复默认”确认两种操作均立即生效并在重启后保留。

## 手工验收 Phase 2

1. 按 `Alt+Q`，确认虚拟桌面被半透明遮罩覆盖并显示操作说明。
2. 从任意方向拖动鼠标，确认选区透明、边框可见且尺寸实时更新。
3. 在剪贴板中预先放置可识别内容，按 `Esc` 或右键取消，确认剪贴板不变。
4. 完成非零选区，确认遮罩立即隐藏并出现“截图已捕获”及正确尺寸提示。
5. 再次检查预先放置的剪贴板内容，确认截图成功路径同样没有修改剪贴板。
6. 运行自动化测试，确认 BGRA→RGBA 转换、RGBA 缓冲区校验和内存 PNG 文件头测试通过。
7. 在 125%/150% 缩放、左侧负坐标副屏和跨屏选区场景重复验证。

## 手工验收 Phase 3

1. 按 `Alt+W`，确认原生问题窗口显示启用供应商和多行问题输入框。
2. 用上下方向键切换供应商，用 `Tab` 在供应商、问题和按钮之间移动焦点。
3. 在问题框中按 `Shift+Enter`，确认插入换行且窗口保持打开；按 `Enter`，确认准备请求并关闭窗口。
4. 再次打开窗口后按 `Esc`，确认取消且下一次快捷键仍能正常启动工作流。
5. 按 `Alt+Q` 完成区域截图，确认随后打开同一问题窗口并能准备带图请求。
6. 按 `Alt+Shift+Q` 完成区域截图，确认不打开问题窗口并直接准备默认问题请求。
7. 在截图框选或问题窗口打开期间重复触发快捷键，确认不会启动第二个工作流。
8. 确认三种成功路径都生成同一结构的请求并交给 Phase 4；不会修改剪贴板或自动发送。

## 手工验收 Phase 4

1. 从设置页确认“ChatGPT 使用桌面网页端”和“Chrome 可执行文件”设置存在。
2. 保持桌面网页端模式启用，按 `Alt+W` 提交非敏感测试问题，确认桌面的 ChatGPT PWA 打开并复用现有登录状态。
3. 确认收到“ChatGPT 桌面网页端已打开；内容准备将在 Phase 5 接入”的通知。
4. 关闭桌面网页端模式后再次测试 ChatGPT，确认 AskBridge 专用 Chrome 按需启动。
5. 在 Chrome 启动信息中确认专用目录为 AskBridge 数据目录中的 `BrowserProfile`，而不是日常 `User Data`。
6. 确认目标标签被创建或激活；唯一匹配标签可复用，多匹配且无可靠焦点证据时新建标签。
7. 操作期间确认托盘和设置窗口仍能响应；默认生命周期下专用 Chrome 保持运行。
8. 两种模式都不得写入网页输入区、上传附件、修改剪贴板或自动发送。
