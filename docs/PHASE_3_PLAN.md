# Phase 3 实施计划

## 范围

本阶段只实现问题输入和请求编排：

- 原生 Win32 问题窗口；
- 启用供应商选择；
- 多行问题输入；
- `Enter` 继续、`Shift+Enter` 换行、`Esc` 取消；
- `Tab` 焦点切换和供应商上下方向键选择；
- 截图并提问、截图快速投递、直接文字提问三种工作流；
- `DispatchMode` 与 `DispatchRequest`；
- 基于 `AppState` 的单工作流并发保护；
- 安全的 Phase 4 请求交接边界。

本阶段不实现专用 Chrome、CDP、页面目标、网页适配器、附件上传、文字插入或自动发送。请求准备完成后仅给出轻量提示并回到 `Idle`。

## 核心接口

```rust
DispatchRequest::new(
    id,
    mode,
    provider_id,
    prompt,
    image,
    created_at_ms,
) -> Result<DispatchRequest>
```

请求保证：

1. ID、供应商和问题均非空；
2. 两种截图模式必须带 `CapturedImage`；
3. 文字模式不得带图；
4. `auto_submit` 创建和反序列化后均固定为 `false`；
5. 日志不得记录问题原文或图片内容。

工作流由 `WorkflowController` 统一约束：

```text
Idle
  -> SelectingRegion
  -> Prompting 或 PreparingDispatch

Idle
  -> Prompting
  -> PreparingDispatch

任意活动状态
  -> Cancelling / Error
  -> Idle
```

问题窗口已打开时再次触发入口，只将现有窗口置前；框选或准备请求期间再次触发，不启动第二条工作流。

## Windows 编排

```text
Alt+Q
  -> 内存截图
  -> 原生问题窗口
  -> DispatchRequest(CaptureWithPrompt)

Alt+Shift+Q
  -> 内存截图
  -> 默认供应商 + quick_prompt
  -> DispatchRequest(CaptureWithDefaultPrompt)

Alt+W
  -> 原生问题窗口
  -> DispatchRequest(TextOnlyPrompt)
```

问题和截图只在当前进程内存中存在。Phase 3 的交接函数只记录非敏感元数据并提示 Phase 4 尚未接入。

## 验证

- 单元测试覆盖命令到模式映射、图像不变量、必填字段和 `auto_submit` 反序列化约束；
- 单元测试覆盖三种状态路径、乱序事件、并发拒绝、取消和错误恢复；
- 手工验证原生窗口、键盘操作、三种入口和重复触发保护；
- 最终运行 fmt、clippy、test、Debug build 和 Release build；
- 检查正式进程不启动浏览器、不访问网络、不写剪贴板、不落盘截图。
