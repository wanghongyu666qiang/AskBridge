# Phase 0 / Phase 1 实施计划

本文以用户提供的 `DEVELOPMENT_SPEC.md` 为功能和架构基线，只覆盖其中 Phase 0 与 Phase 1。

## Phase 0：工程初始化

1. 建立 `askbridge-core` 与 `askbridge-win` 两个 crate 的 Rust workspace。
2. 在核心 crate 中定义应用配置、Provider、快捷键、应用命令、状态和统一错误。
3. 实现带 schema version、默认字段补齐、校验、损坏文件保留和默认恢复的 JSON 配置仓储。
4. 使用 Win32 命名互斥体建立单实例 guard，并用注册窗口消息通知已有实例。
5. 建立 `tracing` 结构化日志入口，禁止记录截图、问题和剪贴板内容。
6. 提供 README 与 build/test/package PowerShell 脚本。

## Phase 1：托盘与快捷键

1. 创建隐藏 Win32 顶层窗口和阻塞式消息循环，不增加轮询或本地服务。
2. 使用 `Shell_NotifyIconW` 创建系统托盘及 Phase 1 菜单。
3. 使用 `RegisterHotKey`、`WM_HOTKEY` 和 `UnregisterHotKey` 管理三个默认命令，并启用 `MOD_NOREPEAT`。
4. 在注册前检测内部重复、无修饰键和危险组合；以 Windows 注册结果检测外部占用。
5. 用动态注册 ID 实现事务式替换：先注册所有变化后的新组合，保存配置成功后再注销旧组合。
6. 提供原生 Win32 最小设置窗：编辑字符串、启用/禁用、应用和恢复默认。
7. 快捷键触发仅生成结构化路由事件和托盘通知，不进入 Phase 2 及之后的工作流。
8. 单元测试覆盖默认配置、JSON 默认字段、保存/加载、损坏恢复、快捷键解析、危险组合与内部冲突。

## 验收

- 自动：`cargo fmt`、`cargo clippy`、`cargo test`、Debug build、Release build。
- 手工：三个默认快捷键路由正确；修改/禁用/恢复默认立即生效；冲突失败不破坏旧注册；第二实例不创建第二个托盘。
