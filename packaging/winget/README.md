# winget 清单（提交草案）

本目录存放提交到 [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) 的清单草案，让用户可以通过 `winget install AskBridge` 安装便携版。目录结构刻意与 winget-pkgs 仓库一致，提交时整目录复制即可。

选用便携 ZIP 而不是 `Setup.exe` 的原因：安装器是自研 Rust 存根，不支持静默开关，而 winget 社区仓库要求无人值守安装能力；便携 ZIP 由 winget 以 `portable` 类型解包并创建命令别名。

## 提交步骤（手动）

1. **验证哈希**：对照 Release 中的 `AskBridge-<版本>-SHA256SUMS.txt` 核对 installer.yaml 里的 `InstallerSha256`（每次发版都会变）。
2. **提交 PR**：Fork `microsoft/winget-pkgs`，把 `manifests/w/wanghongyu666qiang/AskBridge/<版本>/` 整目录复制到 Fork 的同路径下，提交 PR（标题 `New package: wanghongyu666qiang.AskBridge version X.Y.Z`）。流水线会自动校验清单、URL 和安装包，并在需要时提示补充发布者前缀申请；签署 Microsoft CLA 是提交者本人的动作。
3. **后续发版**：每个新版本新增一个版本号目录（哈希取自该版本的 SHA256SUMS），不改旧版本目录。

首次提交是 PR [#429938](https://github.com/microsoft/winget-pkgs/pull/429938)（v2.0.3）。注意 1.12 规范的两个坑：清单必须声明 `InstallerType: zip`；默认语言清单文件名必须是 `<ID>.locale.<PackageLocale>.yaml`（而不是老的 `<ID>.defaultLocale.yaml`）。

## 本地验证

```powershell
winget validate .\manifests\w\wanghongyu666qiang\AskBridge\2.0.3\wanghongyu666qiang.AskBridge.yaml
```

安装验收（提交 PR 前建议做一次）：

```powershell
winget install --manifest .\manifests\w\wanghongyu666qiang\AskBridge\2.0.3\ --interactive
```
