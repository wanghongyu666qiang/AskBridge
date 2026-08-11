# Phase 5 实施计划

## 范围

本阶段在 Phase 4 的目标载体 seam 后实现通用页面准备：

- 在 `askbridge-core` 定义 `PreparationPolicy`、`PreparationOutcome`、`DispatchOutcome` 和受控构造/校验；
- 让状态机显式区分自动准备成功、进入人工兜底、重试、取消和最终回到空闲；
- 通过小而深的 `ProviderAdapter.prepare(...)` interface 隐藏定位、附件、输入和验证步骤；
- 专用 Chrome 使用固定内置 CDP 操作完成通用输入框评分、歧义停止、图片文件控件上传、文字插入和结果验证；
- 桌面 PWA 在尚无可靠 UI Automation 证据时停止自动操作，进入剪贴板人工兜底，不猜测窗口或输入控件；
- 兜底保留原始请求并提供“复制图片”“复制问题”“重试自动投递”“取消”；
- 所有路径保持 `auto_submit=false`，最终发送必须由用户完成。

Phase 5 不增加 ChatGPT、Gemini、Claude 或豆包的专用选择器；供应商覆盖规则属于 Phase 6。

## 安全边界

1. 适配器只操作 Phase 4 已确认的目标；准备前后都检查 URL 边界。
2. 输入候选必须可见、可编辑、启用且评分足够；多个高分候选视为歧义并停止。
3. 固定内置 CDP 程序不读取或记录网页正文，不点击发送控件，不接受远程脚本。
4. 图片只在 `<AskBridge 数据目录>\Temp` 创建随机 PNG；文件控件确认后立即删除，启动时只清理带 AskBridge 前缀的过期 PNG。
5. 剪贴板只在人工兜底窗口存续期间修改；关闭后尽力恢复常见文字和位图格式，不承诺第三方私有格式。
6. 请求、问题和截图不进入普通日志；结果对象只包含 URL、布尔状态、失败阶段和恢复提示。
7. 正式程序不启动本地 HTTP 服务；真实页面自动化测试只能使用测试进程的 `127.0.0.1` 随机端口。

## 当前实现切片

- 已完成 core 结果模型、不变量和 Phase 5 状态转换测试；
- 已完成 `ProviderAdapter` 与通用 CDP adapter；
- 已完成唯一可用 PNG 文件控件上传、文字候选评分/插入/验证和导航复核；
- 已完成桌面 PWA 的安全人工兜底 adapter；
- 已完成四动作 Task Dialog、剪贴板文字/位图准备以及尽力恢复；
- 已增加默认忽略的真实 Chrome Phase 5 集成路径，覆盖稳定本地页面的 textarea、文件上传和最终结果；
- 已为 GNU/MSVC 构建嵌入 Common Controls v6 应用清单，避免 `TaskDialogIndirect` 在 Rust `main` 之前因系统 comctl32 v5 静态加载失败；
- 尚未对真实桌面 PWA 建立可证明可靠的 UI Automation 自动插入；当前一律进入人工兜底；
- 真实专用 Chrome Phase 5 集成测试已通过，覆盖 textarea、PNG 上传、结果验证和临时配置清理；
- 桌面 PWA 的文字人工兜底已通过自动复制、“复制问题”、重试、取消和两次剪贴板恢复实机验收；
- 已修复全局热键创建遮罩后的正常 `WM_KILLFOCUS` 被误判为取消的问题；失去焦点不再终止拖选，`Esc`、右键、关闭和 `WM_CANCELMODE` 仍可明确取消；
- 图片人工兜底已通过真实鼠标 `Alt+Q` 框选、内存截图、问题窗口、“复制问题”、“复制图片”和关闭恢复验收；恢复后的剪贴板格式与原状态一致，位图哈希不同于本次截图。

## 验证门槛

- `cargo fmt --all -- --check`；
- `cargo clippy --workspace --all-targets --offline -- -D warnings`；
- `cargo test --workspace --offline`；
- Debug 与 Release 构建；
- 经用户允许后，运行 ignored 实机 Chrome 集成测试；
- 经用户允许后，启动 AskBridge，分别验证专用 Chrome 自动准备和桌面 PWA 人工兜底；
- 验证任何路径都不会自动发送消息。
