# AskBridge

[简体中文](README.md) | [English](README_EN.md)

AskBridge 是一个 Windows 截图问答工具。框选屏幕内容后，可以复制截图、切换 AI 模型，或将截图和预设文字准备到 AI 网页的输入区。最终是否发送始终由用户决定。

## 下载与打开

1. 前往 [GitHub Releases](https://github.com/wanghongyu666qiang/AskBridge/releases) 下载最新版 `AskBridge-版本号-Setup.exe`。
2. 双击安装程序并选择安装位置。
3. 安装完成后运行 `askbridge.exe`。程序启动后常驻 Windows 托盘，右键托盘图标可以打开设置或退出。

普通用户不需要打开 PowerShell，也不需要运行仓库 `scripts` 目录中的任何命令。

在本仓库参与开发时，编译后的调试程序位于 `target/debug/askbridge.exe`，发布程序位于 `target/release/askbridge.exe`。

## 快捷键

| 快捷键 | 操作 |
| --- | --- |
| `Alt+Q` | 框选截图并显示工具条 |
| `Alt+Shift+Q` | 框选后使用默认模型和快速提示词准备网页内容，不显示工具条 |
| `Alt+W` | 打开默认模型网页，直接在网页中输入文字 |
| `Esc` | 在截图界面取消 |
| `Enter` | 在截图工具条中确认使用当前模型 |

在截图工具条中切换模型后，新选择会保存为下次的默认模型。点击“复制”时才会把截图写入剪贴板。

## 浏览器选择

AskBridge 支持 ChatGPT、Gemini、Claude、豆包和自定义 HTTPS 供应商。

ChatGPT 可以在“设置 > 浏览器”中选择三种打开方式：

- **桌面网页端**：复用现有登录，截图需要手动上传。
- **AskBridge 专用 Chrome**：支持自动上传截图，需要在独立浏览器中登录一次。
- **通用粘贴**：把截图写入剪贴板，聚焦目标网页窗口后模拟一次 Ctrl+V，由你确认并手动发送。该模式使用日常浏览器中的登录状态，不验证粘贴结果；找不到已打开的页面时会自动打开新页面并稍作等待。

专用 Chrome 使用独立的 `BrowserProfile`，不会连接或修改日常 Chrome 配置。

## 使用边界

- AskBridge 不调用模型 API，也不运行本地模型。
- AskBridge 不读取密码、验证码、Cookie、网页正文或历史对话。
- AskBridge 不记录问题原文、截图内容、剪贴板内容或完整聊天 URL。
- AskBridge 不自动点击网页发送按钮，所有请求的 `auto_submit` 固定为 `false`。
- 登录失效、网页结构变化或附件准备失败时，本次操作会停止并显示原因。

## 数据位置

- 源码开发环境：仓库根目录下的 `data`（从 `target` 中的调试程序运行时自动识别）
- 安装版或便携版：`askbridge.exe` 同目录下的 `data`
- 自定义位置：设置绝对路径环境变量 `ASKBRIDGE_DATA_DIR`

配置、日志和专用浏览器资料分别位于 `data/config.json`、`data/logs` 和 `data/BrowserProfile`。网页上传使用的临时截图在操作完成、失败或取消后删除。

更多信息见 [隐私说明](docs/PRIVACY.md) 和 [故障排查](docs/TROUBLESHOOTING.md)。

## 开发

需要 stable Rust、Windows GNU 或 MSVC 构建链，以及 Microsoft Edge WebView2 Runtime。

```powershell
cargo test --workspace
cargo build --workspace --release
cargo xtask help
```

`scripts` 目录是项目维护自动化，不是用户操作步骤：

- `build.ps1` 和 `test.ps1` 是日常构建与测试入口。
- `package.ps1` 和 `test-release-local.ps1` 是打包与完整发布验收入口。
- `test-*`、`validate-*` 和 `measure-*` 是被上述入口调用的独立检查器，用于验证安装、路径保护、真实 UI 和性能数据。
- `cargo xtask` 承担可纯 Rust 测试的性能报告与发布产物验证逻辑。
- `Install-AskBridge.ps1` 与 `Uninstall-AskBridge.ps1` 会被打包进发布产物。

<details>
<summary>发布维护命令</summary>

完整本地发布验收：

```powershell
./scripts/test-release-local.ps1 -AcceptanceRoot D:/AskBridge/target/release-local-acceptance
```

生成安装包和便携包时必须显式指定一个空目录：

```powershell
./scripts/package.ps1 -ArtifactRoot D:/你选择的发布目录
```

脚本不会默认把发布产物写入 C 盘。

</details>

## 许可证

本项目基于 [Apache License 2.0](LICENSE) 开源。
