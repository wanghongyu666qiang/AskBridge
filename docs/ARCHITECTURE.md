# AskBridge 架构概览

AskBridge 是一个 Windows 原生的截图到 AI 网页输入区的轻量桥接工具。它只负责准备内容，最终发送始终由用户完成。

## 工程职责

- `askbridge-core`：不依赖 Win32 的领域类型、配置约束和请求模型，包括 `DispatchRequest` 及固定为 `false` 的 `auto_submit` 边界。
- `askbridge-win`：截图覆盖层、托盘与设置 UI、Windows 生命周期、专用 Chrome 管理、浏览器工作线程、CDP 和供应商 Adapter。
- `xtask`：可在 Rust 单元测试中验证的工程逻辑，例如性能报告和发布产物的结构、metadata、哈希及安全边界。

## 主链路

```text
Screenshot
  -> DispatchRequest
  -> BrowserService / single browser worker
  -> CdpClient
  -> Provider Adapter
  -> 网页输入区（不发送）
```

截图完成后，应用构造 `DispatchRequest` 并交给单一浏览器工作线程。浏览器服务只使用已选择供应商的 Adapter 来准备网页输入；目标、页面或附件状态不明确时流程停止，不猜测点击或输入。

## 专用 Chrome Profile

受管理的 Chrome 使用 AskBridge 独立 `BrowserProfile`，remote debugging endpoint 必须是 loopback。这样可以隔离日常 Chrome 配置，并避免读取或控制用户日常浏览器的 Cookie、密码、验证码和会话。

## Persistent CDP 与 TargetSession

一个 `CdpClient` 持有一个 persistent WebSocket。`BrowserConnection` 复用该连接，并维护 `target_id -> session_id` 映射和有上限的事件队列。目标首次使用时通过 `Target.attachToTarget(flatten=true)` 建立 session；`TargetSession` 在一次同步操作中复用它，并在 detach 或导航状态不确定时 fail-closed。

当前模型刻意保持简单：single browser worker、persistent WebSocket、同步 command。页面 ready 检查可以使用短间隔 polling，但必须有超时和取消路径，且不能让事件与 command response 混淆。

## Human in the loop

`auto_submit` 在请求、配置、打包 metadata 和验收中都必须为 `false`。AskBridge 可以准备截图与文字，但不会点击发送按钮，也不会读取网页聊天正文；用户始终负责最终确认和发送。

## PowerShell 与 xtask

- PowerShell：Windows 编排、安装/卸载、Setup 与真实 UI/browser smoke test。
- Rust `xtask`：纯文件、纯 metadata、哈希、报告和发布产物验证。

安装与卸载脚本仍保留为 PowerShell。可测试的工程判断应优先进入 `xtask`，避免在多个编排脚本中复制验证逻辑。
