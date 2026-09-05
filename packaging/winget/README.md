# winget 清单（提交草案）

本目录存放提交到 [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) 的清单草案，让用户可以通过 `winget install AskBridge` 安装便携版。目录结构刻意与 winget-pkgs 仓库一致，提交时整目录复制即可。

选用便携 ZIP 而不是 `Setup.exe` 的原因：安装器是自研 Rust 存根，不支持静默开关，而 winget 社区仓库要求无人值守安装能力；便携 ZIP 由 winget 以 `portable` 类型解包并创建命令别名。

## 提交步骤（手动，一次性）

1. **验证哈希**：对照 Release 中的 `AskBridge-<版本>-SHA256SUMS.txt` 核对 installer.yaml 里的 `InstallerSha256`（每次发版都会变）。
2. **申请发布者前缀**：若 `wanghongyu666qiang.AskBridge` 尚未被占用，先在 winget-pkgs 仓库开一个 "PackageIdentifier prefix request" Issue。
3. **提交 PR**：Fork `microsoft/winget-pkgs`，把 `manifests/w/wanghongyu666qiang/AskBridge/<版本>/` 整目录复制到 Fork 的同路径下，提交 PR。提交信息用 `New package: wanghongyu666qiang.AskBridge version 2.0.3`。
4. **后续发版**：每个新版本新增一个版本号目录（哈希取自该版本的 SHA256SUMS），不改旧版本目录。

## 本地验证

```powershell
winget validate .\manifests\w\wanghongyu666qiang\AskBridge\2.0.3\wanghongyu666qiang.AskBridge.yaml
```

安装验收（提交 PR 前建议做一次）：

```powershell
winget install --manifest .\manifests\w\wanghongyu666qiang\AskBridge\2.0.3\ --interactive
```
