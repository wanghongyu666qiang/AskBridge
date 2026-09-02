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

运行时只在内存中保留最近一次截图，供用户通过托盘明确复制到剪贴板后自行补救；该入口不会自动聚焦网页、粘贴或发送。专用 Chrome 的文件输入使用数据目录中的临时 PNG。页面缩略图可能早于网页完成实际读取，因此文件赋值一旦开始，临时 PNG 会保留 10 分钟租约，再由后台清理；进程异常退出的遗留文件在后续启动时清理。

### 剪贴板粘贴目标

`BrowserTargetPreference::ClipboardPaste` 是一条完全不经过 CDP 的旁路：截图写入系统剪贴板后，`paste_mode` 模块按标题关键词（内置产品名或自定义显示名/URL 主机）枚举窗口，并用所属进程的可执行文件名将候选限制为受支持的浏览器或 ChatGPT、Claude、豆包桌面客户端。对于 Windows 打包应用还会检查应用包标识，避免把同样使用 `chatgpt.exe` 的 Codex 客户端误判为 ChatGPT。多个候选会按窗口枚举顺序逐个尝试；找不到可激活窗口时用默认浏览器打开 `start_url`，在可取消的预算内继续轮询。每次激活都必须确认前台归属，并且 UI Automation 必须定位到唯一的可见、可用、非密码编辑控件；目标有歧义时在写入前停止。只有这些前置条件成立才合成一次 Ctrl+V；按键注入后不会换目标或再次粘贴。完成状态还要求在该编辑器附近的有界区域连续检测到新的图片和附件分组结构；回执不确定时按“可能已经写入附件”失败关闭，并提示用户先检查页面。提示词不会自动填入。

## 专用 Chrome Profile

受管理的 Chrome 使用 AskBridge 独立 `BrowserProfile`，remote debugging endpoint 必须是 loopback。这样可以隔离日常 Chrome 配置，并避免读取或控制用户日常浏览器的 Cookie、密码、验证码和会话。

## Persistent CDP 与 TargetSession

一个 `CdpClient` 持有一个 persistent WebSocket。`BrowserConnection` 复用该连接，并维护 `target_id -> session_id` 映射和有上限的事件队列。目标首次使用时通过 `Target.attachToTarget(flatten=true)` 建立 session；`TargetSession` 在一次同步操作中复用它，并在 detach 或导航状态不确定时 fail-closed。

当前模型刻意保持简单：single browser worker、persistent WebSocket、同步 command。页面 ready 检查可以使用短间隔 polling，但必须有超时和取消路径，且不能让事件与 command response 混淆。

## Human in the loop

`auto_submit` 在请求、配置、打包 metadata 和验收中都必须为 `false`。AskBridge 可以准备截图与文字，但不会点击发送按钮，也不会读取网页聊天正文；用户始终负责最终确认和发送。

## 应用更新

`UpdateService` 是独立于截图和浏览器工作流的单工作线程模块。它以 `check_now`、`download`、`launch_installer` 为小接口，内部负责 GitHub Release 解析、版本比较、官方资产地址约束、大小限制、SHA256SUMS 的离线 Ed25519 签名校验（公钥编译进 `askbridge-core::RELEASE_SIGNING_PUBLIC_KEY`）、流式落盘（边下载边写入 `.partial` 并计算 SHA-256，校验通过后才改名为正式文件，失败自动清理）、节流的进度事件、`data/Updates` 原子发布和事件队列。启动时检查一次，此后最多每 24 小时检查一次；只有用户在托盘确认后才下载。安装器启动失败后，托盘菜单的安装入口会优先复用已校验的本地安装包。

主进程不直接覆盖自身。校验通过后，它以既有安装目录、父进程 PID 和重启标记启动独立 `Setup.exe`，然后走正常退出；Setup 等待并验证对应 AskBridge 进程已经退出，再调用既有安装事务原位置升级、保留 `data` 并重启。普通手工安装不带这些一次性环境变量，行为保持不变。

## PowerShell 与 xtask

- PowerShell：Windows 编排、安装/卸载、Setup 与真实 UI/browser smoke test。
- Rust `xtask`：纯文件、纯 metadata、哈希、报告和发布产物验证。

安装与卸载脚本仍保留为 PowerShell。可测试的工程判断应优先进入 `xtask`，避免在多个编排脚本中复制验证逻辑。
