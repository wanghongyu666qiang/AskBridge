# 更新日志

本文件记录面向用户的重要变化，格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本遵循语义化版本。每个版本的完整发布说明同时见 [GitHub Releases](https://github.com/wanghongyu666qiang/AskBridge/releases)。

## [未发布]

### 安全

- 应用内更新在哈希校验与安装包启动之间持有拒绝写入/删除共享的文件句柄，防止校验通过后被其他进程替换（TOCTOU 加固）。
- 应用清单声明 supportedOS 与 Per-Monitor V2 DPI，作为启动时代码设置的兜底。

### 变更

- 设置窗口重排：四个页面改为"小节标题 + 分隔线"的分区布局，输入框改为扁平 1px 边框，页签整行化并加页眉分隔线，切换页签时正确重绘选中态；强调色从通用蓝改为品牌橙，中性色与工具条一致；"恢复默认快捷键"移入快捷键页；修正副标题等三处过期或冗长文案。
- 切换到供应商页不再自动启动能力检测（此前会自动打开 Chrome）；新增"检测供应商连接"按钮，由用户手动触发。

### 工程

- 状态机回滚、开机启动项回滚和缓存安装包校验失败现在会记录 warning 日志；托盘"重试安装"失败时不再丢失具体原因。
- 可选的远程供应商规则通道在获取、解析、缓存任一环节失败时分别记录原因。
- 构建脚本支持用 `ASKBRIDGE_WEBVIEW2LOADER_PATH` 指定 WebView2Loader.dll；找不到时明确提示运行时后果与解决办法。
- 日志隐私守卫改为构建时自动扫描 `src` 下全部源码文件，新模块自动纳入检查。
- 新增 `SECURITY.md` 安全策略（含签名密钥泄漏应急预案）与 `CHANGELOG.md` 更新日志。
- CI 新增 PowerShell 脚本语法检查、测试覆盖率汇总、每周定时依赖安全扫描，并引入 Dependabot 自动更新 Rust 依赖与 GitHub Actions。

## [2.0.3] - 2026-09-03

### 修复

- 截图交付加固：框选结果改为从覆盖层已冻结的桌面快照裁剪，避免动态画面与用户所见不一致；安全回退统一 3 秒截止时间。
- 网页上传前程序异常退出时保留临时截图，供手动上传恢复。
- 安装器取消“登录后自动启动”时，同步清理注册表启动项并持久化关闭状态。

## [2.0.2] - 2026-09-01

### 新增

- 安装器支持交互式选项：桌面快捷方式、开始菜单快捷方式、登录后自动启动。

## [2.0.1] - 2026-08-30

### 变更

- 重新设计截图工具条与供应商菜单。

## [2.0.0] - 2026-08-28

### 安全

- 自更新加固：`SHA256SUMS.txt` 必须通过内嵌公钥的 Ed25519 离线签名校验；安装包流式下载并实时计算 SHA-256，校验通过后才落盘为正式文件。

### 新增

- 扩展 AI 供应商支持；品牌图标嵌入 Win32 资源（窗口与托盘）。

### 变更

- 最低 Rust 版本（MSRV）提升至 1.88；CI 工作流固定到完整 commit SHA，Release 附加构建溯源证明。

### 修复

- PWA（网页应用）截图交付稳定性与卸载流程问题。

## [1.0.2] - 2026-08-26

### 新增

- 应用内自更新支持：后台检查、确认下载、校验、原位置升级并重启。

## [1.0.1] - 2026-08-24

### 新增

- “通用粘贴”目标：支持日常浏览器与 ChatGPT、Claude、豆包等 AI 桌面客户端的登录状态。
- 专用 Chrome 准备失败且确认未写入任何内容时，安全降级为剪贴板复制。

### 变更

- 截图覆盖层拆分为子模块并跨绘制复用 GDI 资源，减少框选拖拽闪烁。

### 工程

- CI 增加 Windows GNU 构建链路与 cargo-deny 依赖检查；新增贡献指南、Issue 模板与英文 README；许可证确定为 Apache-2.0。

## [1.0.0] - 2026-08-13

### 新增

- 首个公开版本：框选截图与工具条、把截图和预设文字准备到 AI 网页输入区、专用 Chrome 与 CDP 对接、发布打包与安装路径防护。

[未发布]: https://github.com/wanghongyu666qiang/AskBridge/compare/v2.0.3...HEAD
[2.0.3]: https://github.com/wanghongyu666qiang/AskBridge/compare/v2.0.2...v2.0.3
[2.0.2]: https://github.com/wanghongyu666qiang/AskBridge/compare/v2.0.1...v2.0.2
[2.0.1]: https://github.com/wanghongyu666qiang/AskBridge/compare/v2.0.0...v2.0.1
[2.0.0]: https://github.com/wanghongyu666qiang/AskBridge/compare/v1.0.2...v2.0.0
[1.0.2]: https://github.com/wanghongyu666qiang/AskBridge/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/wanghongyu666qiang/AskBridge/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/wanghongyu666qiang/AskBridge/releases/tag/v1.0.0
