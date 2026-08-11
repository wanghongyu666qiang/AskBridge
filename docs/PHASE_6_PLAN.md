# Phase 6 实施与验收计划

## 范围

本阶段在 Phase 5 的 `ProviderAdapter.prepare(...)` seam 内增加内置供应商覆盖规则：

- 核验 ChatGPT、Gemini、Claude、豆包的官方网页入口；
- 用带 `schema_version` 的只读内嵌 JSON 声明登录 URL、输入框和图片文件控件选择器；
- 内置规则优先，规则未命中时继续使用 Phase 5 通用候选评分与文件控件回退；
- URL 明确进入登录流程时返回“请在专用浏览器自行登录”，不读取密码、验证码或 Cookie；
- 内置规则和通用回退均无法确认输入框时提示网页可能改版；
- 继续固定 `auto_submit=false`，不点击发送按钮，不执行网络下发脚本。

Phase 6 不实现设置页扩展、自定义供应商 UI、开机启动、安装器或自动发送；这些分别属于后续阶段或 1.0 之外的独立授权。

## 已核验入口

核验日期：2026-08-04。

| 供应商 | 起始页 | 官方依据 |
| --- | --- | --- |
| ChatGPT | `https://chatgpt.com/` | [OpenAI 帮助中心](https://help.openai.com/en/articles/7426629-why-can-t-i-log-in-to-chatgpt)确认 `chatgpt.com/auth/login`；匿名入口实测可访问 |
| Gemini | `https://gemini.google.com/app` | [Google Gemini Apps 帮助](https://support.google.com/gemini/answer/13278668?hl=en-US)确认 Web 应用和 Google Accounts 登录跳转 |
| Claude | `https://claude.ai/new` | [Claude 帮助中心](https://support.claude.com/en/articles/8114491-get-started-with-claude)确认 Web 应用使用 `claude.ai`；匿名根页实测跳转 `/login` |
| 豆包 | `https://www.doubao.com/chat/` | [豆包官方协议](https://www.doubao.com/legal/ey01)明确官方网站；匿名 `/chat/` 与同源 `/login` 实测可访问 |

入口和选择器是编译时基线，不会从网络静默更新。真实网页会改版，因此仍必须保留通用回退、明确错误和手工验收。

## 安全边界

1. 规则文件只接受 HTTPS 登录模式和 CSS 选择器，不接受脚本、换行或语句分隔符。
2. adapter 在准备前后复核 URL；登录跳转、跨站导航、候选歧义或验证失败均停止自动操作。
3. 供应商选择器只缩小候选集合；未命中时回到通用评分，不直接选择页面中的第一个可编辑元素。
4. 图片仍只使用网页正常的 `input[type=file]` 与 `DOM.setFileInputFiles`，并要求唯一、启用且接受 PNG。
5. 普通日志只记录请求 ID、供应商、阶段和布尔结果，不记录目标 URL、问题、截图、网页正文或登录数据。
6. 正式程序不启动本地 HTTP 服务，不连接用户日常 Chrome，不自动发送。

## 当前实现

- `builtin_rules.json`：四家规则、schema 版本和最小选择器集合；
- `rules.rs`：启动时解析、完整性、唯一性和安全值校验；
- `GenericProviderAdapter::for_provider(...)`：按配置选择内置规则或通用 adapter，保持外部 interface 不变；
- 供应商输入选择器优先，找不到时继续通用候选评分；
- 供应商图片选择器优先，找不到时继续通用 PNG 文件控件发现；
- 登录 URL 返回 `LoginInBrowser`；规则与通用定位同时失效返回 `ProviderPageChanged`；
- 人工兜底按恢复提示显示登录或网页改版说明；
- 页面准备日志已移除完整结果对象，避免间接记录目标 URL。
- SPA 页面会在有界超时内轮询等待文件控件、输入框或可见登录结构；同站路由切换只允许重新取得 target 和精确 URL 后重试一次，登录或跨站导航仍立即停止。

## 验证门槛

- `cargo fmt --all -- --check`；
- `cargo clippy --workspace --all-targets --offline -- -D warnings`；
- `cargo test --workspace --offline`；
- Debug 与 Release 离线构建；
- 经用户明确允许后，启动 AskBridge 专用 Chrome，分别对四家真实网页使用非敏感文字进行手工验收；
- 图片验收只使用非敏感测试截图，确认附件和文字准备后仍由用户手动发送；
- 登录态缺失时确认只提示用户自行登录；
- 检查日志不含问题、图片、网页正文、目标 URL 或登录数据。

自动门禁通过不等于真实网页验收完成。真实验收会启动专用 Chrome、访问供应商网页并使用其中已有登录状态，必须另行取得用户启动授权。

2026-08-09 最新结果：格式检查、Clippy、124 个普通测试、Debug 构建、Release 构建和 `git diff --check` 均通过；两个默认忽略的实机入口仍由显式命令运行。隔离 Chrome/CDP round-trip 覆盖真实 Chrome 启动、CDP 握手、规则优先、通用回退、文字回读、PNG 文件控件、登录分类、正常关闭和 D 盘临时配置清理。

Chrome `151.0.7922.77` 与现有 `D:\AskBridge\data\BrowserProfile` 的真实 provider harness 已确认 ChatGPT、Gemini、Claude、豆包都能把非敏感测试文字写入编辑区，且没有自动发送；ChatGPT 的 1×1 PNG 与文字也同时准备成功。Gemini 未直接暴露唯一文件控件，Claude 的控件在同站路由后不稳定，豆包未发现可证明唯一的附件入口，三者图片仍进入 `CopyImageThenText` 人工兜底。CDP harness 不替代真实 AskBridge 热键、Task Dialog 和剪贴板的 Windows UI 串联复核。
