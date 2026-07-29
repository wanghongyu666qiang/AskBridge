# Phase 2 实施计划

## 范围

本阶段只实现截图模块：

- 多显示器与虚拟桌面枚举；
- Per-Monitor V2 DPI 下的物理屏幕坐标；
- 全屏半透明遮罩；
- 鼠标正向/反向选区；
- 选区尺寸显示；
- `Esc`、右键和失焦取消；
- 遮罩隐藏后的 GDI 选区捕获；
- BGRA 到 RGBA 转换；
- `CapturedImage` 内存 RGBA 缓冲区；
- 独立 PNG 内存编码；
- 两个截图快捷键的真实事件路由。

本阶段不实现问题输入框、供应商选择、专用 Chrome、CDP、网页适配器、剪贴板兜底或浏览器扩展。截图成功和取消都不修改系统剪贴板。

## 模块接口

应用层只依赖：

```rust
CaptureService::capture() -> Result<CaptureOutcome>
```

成功时该接口保证：

1. 返回的 `CapturedImage` 只保存在内存中；
2. `source_rect` 使用虚拟桌面物理坐标并支持负数；
3. RGBA 缓冲区长度严格等于 `width × height × 4`；
4. PNG 编码器可以从同一 RGBA 图像生成有效内存 PNG；
5. 遮罩在捕获前已经隐藏并完成桌面合成同步；
6. 截图成功路径不修改剪贴板。

取消时返回 `CaptureOutcome::Cancelled`，同样不修改剪贴板。

## 内部模块

```text
capture/monitor.rs  显示器枚举与虚拟桌面几何
capture/overlay.rs  遮罩窗口、选择状态和取消流程
capture/screen.rs   GDI 像素捕获与资源所有权
capture/encoder.rs  BGRA → RGBA 与独立 PNG 内存编码
```

Win32 handle 和 GDI 对象不进入 `askbridge-core`。核心 crate 只暴露 `ScreenRect` 和 `CapturedImage`。

## 验证

- 纯逻辑测试覆盖负坐标、反向拖动、零面积、坐标平移和溢出；
- 编码测试覆盖 BGRA/RGBA 转换和 PNG 文件头；
- 显示器矩形测试覆盖左侧负坐标；
- 手工验证遮罩、取消、选区尺寸、成功及取消均不修改剪贴板；
- 最终运行 fmt、clippy、test、Debug build 和 Release build。
