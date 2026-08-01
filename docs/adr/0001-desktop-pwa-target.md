# ADR 0001：为 ChatGPT 增加桌面 PWA 目标

日期：2026-08-01
状态：已批准并实施

## 背景

Phase 4 最初只提供隔离的专用 Chrome 与 CDP。真实桌面验收发现，用户在专用 Chrome 中无法完成 ChatGPT 登录，而桌面已有的 Chrome PWA 已保持有效登录会话。该 PWA 由桌面的 `ChatGPT.lnk` 启动，使用日常 Chrome 的 `Default` 配置。

用户明确批准 AskBridge 优先复用现有桌面 ChatGPT 网页端；全部 AskBridge 配置和运行数据仍保存在 D 盘。

## 决策

在浏览器目标 seam 下保留两个 adapter：

1. `DesktopPwaLauncher`：发现或使用显式配置的绝对 `.lnk` 路径，通过 Windows Shell 启动桌面 PWA；Phase 4 不读取其 Cookie、密码、历史记录或网页正文。
2. 专用 Chrome/CDP adapter：继续负责隔离配置目录、动态调试端点、目标选择和页面就绪，供其他供应商或用户主动关闭 PWA 模式时使用。

`BrowserTargetPreference` 负责每个供应商的选择。ChatGPT 默认选择 `desktop_pwa`，其他供应商默认选择 `dedicated_chrome`；设置页允许用户切换 ChatGPT 模式。

Phase 4 只保证目标载体被打开并置前。桌面 PWA 的输入、附件和验证属于 Phase 5，将通过 UI Automation/剪贴板 adapter 实现；CDP adapter 继续用于专用 Chrome。两者最终遵守同一个页面准备 interface。

## 被否决的方案

- 强行用 CDP 接管日常 Chrome `Default` 配置：会扩大隐私权限，且现代 Chrome 对默认配置目录的远程调试有限制。
- 要求用户重新在专用 Chrome 登录：真实验收已经证明该路径在当前账户上被安全策略阻止。
- 浏览器扩展：当前正式架构明确不把扩展作为依赖，权限与部署成本也更高。

## 影响与风险

- 优点：复用现有登录状态，解决当前 ChatGPT 登录阻塞；不复制或迁移凭据。
- 风险：Windows UI Automation 对 Chromium PWA 的输入和附件操作可能比 CDP 更脆弱，因此 Phase 5 必须保留剪贴板人工兜底并建立真实桌面验收。
- 安全：自动发现只接受桌面上的 `ChatGPT.lnk`；显式配置也只接受存在的绝对 `.lnk`。AskBridge 不解析或记录登录资料。
- 迁移：现有配置缺少新字段时使用默认目标偏好；当前用户配置写入 `target_preferences.chatgpt = "desktop_pwa"`。
