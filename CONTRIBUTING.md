# 贡献指南

感谢关注 AskBridge。本文面向想参与开发的贡献者；使用问题请先看 [README](README.md) 和 [故障排查](docs/TROUBLESHOOTING.md)。

## 环境要求

- stable Rust（`rust-version = "1.85"` 及以上），Windows GNU 或 MSVC 构建链
- Microsoft Edge WebView2 Runtime
- 仅支持在 Windows 上运行和测试

## 本地开发

```powershell
git clone https://github.com/wanghongyu666qiang/AskBridge.git
cd AskBridge
cargo test --workspace
cargo build --workspace --release
```

日常入口：

- `./scripts/build.ps1`、`./scripts/test.ps1`：构建与测试。
- `./scripts/package.ps1 -ArtifactRoot <空目录>`：打包（必须显式指定空目录，脚本不会默认写 C 盘）。
- `./scripts/test-release-local.ps1`：完整发布验收。
- `cargo xtask help`：发布产物校验与性能报告。

调试程序位于 `target/debug/askbridge.exe`。源码开发环境下数据目录自动解析为仓库根目录的 `data`。

## 提交与 PR 约定

- 一个提交只做一件事，避免上千行的混合提交。
- 提交信息建议使用前缀：`feat:`、`fix:`、`refactor:`、`docs:`、`test:`、`ci:`。
- PR 前请确保以下命令全部通过（CI 会以同样标准检查）：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
```

## 项目红线

以下约束是 AskBridge 的设计承诺，改动不得违反：

1. `auto_submit` 必须保持 `false`。程序永远不代替用户点击网页发送按钮，相关校验分布在配置加载、请求构造和打包元数据中。
2. 不引入遥测、崩溃上报或任何形式的网络数据回传。
3. 不读取密码、验证码、Cookie、网页正文或历史对话；专用 Chrome 配置仅限回环地址远程调试。
4. 遵循 fail-closed：目标、页面或附件状态不确定时停止并说明原因，而不是猜测点击或输入。

有违反上述约束的改动会被拒绝。

## 报告问题

提交 Bug 请使用 Issue 模板，尽量附上 `data/logs` 中的相关日志片段（AskBridge 不记录问题原文和截图内容，但贴日志前仍建议自行确认无敏感信息）。功能建议请说明使用场景，而不是只提实现方案。
