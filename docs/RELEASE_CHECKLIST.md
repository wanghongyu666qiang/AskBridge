# AskBridge 1.0 发布检查清单

## 代码与自动门禁

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --offline -- -D warnings`
- [ ] `cargo test --workspace --offline`
- [ ] `test-powershell-syntax.ps1` 解析全部 PowerShell 脚本
- [ ] `test-acceptance-root-guards.ps1` 确认验收脚本先拒绝相对路径、仓库 `target` 外路径、已有目录和隔离目录外截图
- [ ] `test-package-root-guards.ps1` 确认打包入口先拒绝相对产物目录、仓库根、仓库 `target` 根和非空产物目录
- [ ] Debug 与 Release 离线构建
- [ ] `test-release-local.ps1 -AcceptanceRoot <target 下全新绝对目录>` 通过并完成清理，且无默认验收目录
- [ ] 4 个 ignored Chrome/性能测试按适用范围通过：回环 CDP、真实供应商、Chrome 长采样、准备耗时
- [ ] `git diff --check`
- [ ] Release 二进制、ZIP、Setup EXE 的 SHA-256 已记录

## Windows 与真实功能

- [ ] 三个默认快捷键、修改/禁用/恢复和失败回滚
- [ ] 单实例、托盘菜单和正常退出
- [ ] 单屏、多屏、负坐标、100/125/150/200% 与混合 DPI 截图矩阵
- [ ] 三种请求入口及忙碌/取消路径
- [ ] 设置四页、快速提示词、默认/启用/自定义供应商和生命周期
- [ ] 当前用户开机启动启用、重启/二次启动检查和撤销
- [ ] 配置迁移、损坏备份恢复、临时图片清理和日志脱敏
- [ ] ChatGPT、Gemini、Claude、豆包文字准备；安全图片准备或明确人工兜底；从不自动发送

## 性能与体积

- [ ] 桌面进程空闲 5 分钟 CPU 平均不高于 0.2%
- [ ] 桌面进程空闲 Working Set 目标 20 MiB、验收上限 35 MiB
- [ ] 桌面进程空闲外部 TCP 连接为 0，常驻进程为 1
- [ ] 快捷键到遮罩和文字输入框延迟已实测记录
- [ ] 专用 Chrome 未启动、冷启动、首次/连续准备耗时、进程数和总内存单独记录，桌面/Chrome 性能采样均显式传入当前 Release EXE
- [ ] `test-performance-report-validator.ps1` 验证性能报告校验器的成功和失败路径
- [ ] `validate-performance-report.ps1` 使用完整证据包确认桌面/Chrome 性能报告与当前 Release EXE 路径和哈希一致、Chrome 报告来自 AskBridge 专用 profile、桌面/Chrome 时长达到 300 秒，且桌面/Chrome/准备耗时三份报告都有测量时间戳
- [ ] Release EXE 尽量不高于 15 MiB，安装包尽量不高于 25 MiB，规则/静态资源尽量不高于 2 MiB

## 安装、升级与卸载

- [ ] 安装目标由用户显式选择，普通用户权限可完成
- [ ] `test-package-artifact-validator.ps1` 验证最终产物校验器的成功和失败路径
- [ ] `test-install-metadata-validator.ps1` 验证安装入口拒绝仓库根/target 根/包目录等不安全安装目标、不安全 package metadata、非字符串身份字段、字符串安全开关和额外字段
- [ ] `test-package.ps1`、`test-installer.ps1` 与 `test-setup.ps1` 在全新 D 盘隔离目录通过并完成清理
- [ ] `validate-package-artifacts.ps1` 使用完整证据包对最终产物目录完成只读复核，版本匹配，便携目录只含预期 7 个文件且无隐藏残留，ZIP 文件头有效，Setup EXE 具备有效 PE 签名，ZIP 条目名和内容哈希匹配便携目录，包元数据只含固定字段集，身份字段为 JSON string，安全开关为 JSON boolean `false`，SHA-256 清单只含 ZIP/Setup EXE/`askbridge.exe`，包内 EXE 与当前 Release EXE 哈希一致，包内文档/安装脚本与当前源树一致，顶层没有额外残留
- [ ] 安装包不捆绑 Chrome，不写全局 Rust/Chrome 配置
- [ ] 覆盖升级保留 `data`、设置和专用 Chrome 登录状态
- [ ] 可选启动项只写当前用户 Run 键
- [ ] 卸载清理程序、命名启动项和可选快捷方式
- [ ] 卸载默认保留 `data`；删除前明确提示其中可能含登录状态
- [ ] D 盘临时验收安装目录和残留已按用户授权清理

## 文档与交付

- [ ] README、隐私说明、故障排查、阶段交接和发布说明与当前实现一致
- [ ] `Cargo.toml` 仅在全部门禁通过后标记 `1.0.0`
- [ ] 没有自动发送开关、扩展或网络下发规则进入 1.0
- [ ] Definition of Done 逐项有当前证据，缺失项不标记完成
