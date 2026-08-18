# AskBridge

AskBridge 是一个面向 Windows 10/11 的轻量截图问答工具。它在用户框选截图后显示紧凑工具条，将截图准备到所选 AI 网页的编辑区；最终发送始终由用户在网页中确认。

## 使用方式

- `Alt+Q`：框选截图并停留在截图层。工具条可以复制截图、取消、切换模型或使用当前模型继续。
- `Alt+Shift+Q`：框选后使用默认模型和设置中的快速提示词直接准备网页内容，不显示工具条。
- `Alt+W`：直接打开默认模型的网页输入区，文字由用户在网页中输入。
- 截图工具条中，`Esc` 等同“取消”，`Enter` 等同“问问当前模型”。
- 在工具条中选择模型后，该模型会成为下次默认模型。

AskBridge 不会自动点击发送按钮，所有请求的 `auto_submit` 都固定为 `false`。

## 主要能力

- 多显示器区域截图，支持负坐标、副屏、混合 DPI、反向拖动和尺寸提示。
- WebView2 截图工具条，保留原生绘制作为 WebView2 初始化失败时的可用性保护。
- ChatGPT、Gemini、Claude、豆包和自定义 HTTPS 供应商。
- 可为 ChatGPT 选择桌面网页端或 AskBridge 专用 Chrome；截图自动上传需要专用 Chrome。
- 专用 Chrome 使用独立的 `BrowserProfile`，不连接日常 Chrome 配置。
- 通过本机回环 CDP 定位网页输入区、准备文字和图片，并验证页面回执。
- 托盘设置、全局快捷键、当前用户开机启动、日志轮换和便携安装。
- “复制”只在用户明确点击后将截图写入剪贴板。

页面需要登录、发生导航、输入区不唯一、附件控件缺失或网页结构变化时，AskBridge 会直接停止本次准备并给出明确提示，不会猜测控件或要求用户切换到另一套中间流程。

## 数据位置

- 开发工作区：`D:\AskBridge\data`
- 便携或安装版本：`askbridge.exe` 同目录的 `data`
- 显式覆盖：绝对路径环境变量 `ASKBRIDGE_DATA_DIR`

配置位于 `data\config.json`，日志位于 `data\logs\askbridge.log`，专用 Chrome 登录资料位于 `data\BrowserProfile`。网页上传所需的临时 PNG 位于 `data\Temp`，完成、失败或取消后会删除。

## 构建

需要 stable Rust、Windows GNU 或 MSVC 构建链，以及 Microsoft Edge WebView2 Runtime。

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

也可以运行仓库脚本：

```powershell
.\scripts\build.ps1
.\scripts\test.ps1
.\scripts\test-powershell-syntax.ps1
.\scripts\test-release-local.ps1 -AcceptanceRoot D:\AskBridge\target\release-local-acceptance
.\scripts\package.ps1 -ArtifactRoot D:\你明确选择的产物目录
```

`package.ps1` 要求传入绝对、专用、空的 `ArtifactRoot`，不会默认把发布产物保存到 C 盘。安装位置同样由用户显式选择。

## 安全边界

- 不调用模型 API，不运行本地模型。
- 不读取密码、验证码、Cookie、网页正文或历史对话。
- 不记录问题原文、截图内容、剪贴板内容或完整聊天 URL。
- 不连接日常 Chrome 调试端点，不使用固定远程调试端口。
- 不自动发送，不绕过供应商登录或安全机制。

更多信息见 [隐私说明](docs/PRIVACY.md) 和 [故障排查](docs/TROUBLESHOOTING.md)。
