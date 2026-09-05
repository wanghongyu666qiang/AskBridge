# 安全策略

[English summary below.](#english-summary)

## 支持的版本

只有最新发布的版本（见 [Releases](https://github.com/wanghongyu666qiang/AskBridge/releases)）接收安全修复。更新链路包含离线 Ed25519 签名校验，请始终通过应用内更新或官方 Release 获取新版本。

## 如何报告漏洞

**请不要在公开 Issue 中描述安全漏洞。**

使用 GitHub 的“私下报告漏洞”渠道：打开仓库的 **Security** 标签页 → **Report a vulnerability**。该渠道只有维护者可见，并允许在修复发布前保持私密讨论。

报告时请尽量包含：

- 影响的版本（安装版的版本号，或 `data/logs` 中记录的版本信息）；
- 复现步骤或概念验证；
- 影响评估（例如是否可导致任意代码执行、绕过更新校验、读取用户数据）。

## 范围

属于本项目的攻击面：

- `askbridge.exe` 桌面客户端：截图捕获、剪贴板写入、托盘、设置界面；
- `Setup.exe` 安装器与随包的 `Install-AskBridge.ps1` / `Uninstall-AskBridge.ps1`；
- 应用内更新链路：Release 元数据获取、SHA-256 与 Ed25519 签名校验、安装包落盘与启动；
- 专用 Chrome 的 CDP 自动化边界（调试端口仅限本机回环）与“通用粘贴”路径。

不属于本项目范围：

- Chrome、WebView2、Windows 自身的漏洞（请报告给对应供应商）；
- 已在同一用户会话内取得任意代码执行的恶意软件——AskBridge 的威胁模型假定该场景已沦陷；
- AI 供应商网站自身的问题。

## 响应目标

- 3 个工作日内确认收到；
- 7 天内给出初步评估与处理计划；
- 修复随下一个 Release 发布，并在发布说明中致谢（除非你希望匿名）。

采用协调披露：在修复发布前请勿公开细节。对善意的安全研究不会采取法律行动。

## 维护者：签名密钥应急预案

更新签名私钥只保存在维护者本地（`.update-signing-key.json`，已加入 `.gitignore`，不入库）和 GitHub Actions 保密项 `UPDATE_SIGNING_KEY`。客户端公钥编译期固定，无法远程轮换。一旦私钥疑似泄漏：

1. 立即轮换 Actions 中的 `UPDATE_SIGNING_KEY`，并用 `cargo xtask gen-update-key` 生成新密钥对。
2. 尽快发布包含新内嵌公钥的客户端版本——这是唯一能让已安装客户端信任新签名的途径。
3. 在新客户端发布前的窗口期，已安装客户端无法验证用新私钥签发的清单。通过 README、Release 说明等官方渠道公告“暂勿使用应用内更新，请从官方 Release 手动下载并核对 SHA-256”。
4. 如有证据表明攻击者已用旧私钥签发恶意安装包，同时下架相关 GitHub Release，并在公告中列出受影响的版本号。

## English Summary

Supported: latest release only. Report vulnerabilities privately via GitHub **Security → Report a vulnerability** (no public issues). In scope: the AskBridge client, `Setup.exe` and the packaged install/uninstall scripts, the in-app update chain (metadata fetch, SHA-256 + Ed25519 verification, download and launch), and the dedicated-Chrome/CDP and universal-paste paths. Out of scope: vulnerabilities in Chrome/WebView2/Windows themselves, malware already running as the same user, and issues in the AI providers' own websites. Coordinated disclosure; no legal action against good-faith research. Response target: acknowledge within 3 business days, initial assessment within 7 days, fix ships with the next release.
