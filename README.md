# AskBridge

AskBridge 是一个面向 Windows 10/11 的超轻量桌面快捷操作层。当前仓库实现开发基线中的 Phase 0 与 Phase 1：原生 Win32 托盘、全局快捷键、配置持久化、单实例和最小快捷键设置入口。

## 当前能力

- 启动后仅驻留系统托盘，不创建可见主窗口。
- 注册三个默认全局快捷键：
  - `Alt+Q`：截图并提问；
  - `Alt+Shift+Q`：截图快速投递；
  - `Alt+A`：直接文字提问。
- 快捷键可在托盘“设置…”中修改、禁用、恢复默认，并在应用后立即生效。
- 修改快捷键时先占用新组合；系统注册或配置保存失败时保留旧组合。
- 检测 AskBridge 内部重复、缺少修饰键、危险系统组合和被其他程序占用的组合。
- 配置保存至 `%LOCALAPPDATA%\AskBridge\config.json`；损坏配置会备份为 `config.corrupt-<timestamp>.json` 并恢复默认值。
- 使用当前用户会话的命名互斥体保证只有一个托盘实例；第二次启动会通知已有实例打开设置。
- 快捷键触发后记录不含用户内容的结构化事件，并显示轻量托盘通知。

## 当前明确不包含

Phase 0/1 不实现真实截图、问题输入框、剪贴板操作、网页自动粘贴、浏览器扩展、开机启动和安装器。项目不使用 Electron、Tauri、Python、WebView、本地 HTTP 服务或内嵌浏览器。

## 架构

```text
askbridge-core
  领域命令、配置模型、Provider、快捷键解析/校验、统一错误、配置仓储

askbridge-win
  Win32 消息循环、单实例、RegisterHotKey、系统托盘、原生设置窗口
```

核心配置和规则不依赖 UI；Windows handle 由对应的 RAII 对象释放。常驻路径没有轮询、网络请求或高频计时器。

## 构建

需要 stable Rust 和 Windows GNU 或 MSVC 构建链。

```powershell
cargo fmt
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
2. 分别按 `Alt+Q`、`Alt+Shift+Q`、`Alt+A`，确认托盘通知中的命令名称正确。
3. 从托盘打开设置，修改一个快捷键并应用，确认旧组合失效、新组合立即生效。
4. 尝试重复组合、`Ctrl+C` 或已被系统占用的组合，确认配置不生效且原组合仍可用。
5. 取消“启用”后应用，再使用“恢复默认”确认两种操作均立即生效并在重启后保留。
