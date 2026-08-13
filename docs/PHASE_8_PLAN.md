# Phase 8：性能、安装与发布

## 目标

把通过 Phase 7 的 Windows 原生程序交付为可复现的便携 ZIP 和用户级自解压安装包，完成性能、覆盖升级、卸载、隐私和发布证据闭环。安装位置由用户显式选择，不默认写入 C 盘。

## 实现

- Release 保持 LTO、单 codegen unit、size 优化、panic abort 和符号剥离；版本在全部验收前保持预发布值。
- `package.ps1` 离线构建，要求绝对且不存在或为空的专用 `ArtifactRoot`，并拒绝直接使用仓库根或仓库 `target` 根，生成扁平便携目录、ZIP、可控 Rust Setup stub 附加 payload EXE 和 SHA-256 清单；等待 Setup 文件稳定后才计算哈希，不捆绑 Chrome。
- 安装脚本要求绝对目标，逐文件临时写入后替换，覆盖升级保留 `data`；可选当前用户启动项和开始菜单快捷方式，并同步持久化 `start_on_login`，首次启动不会撤销安装选择。
- 安装入口会拒绝不安全安装目标和不安全的 `package.json` 元数据：安装目标不能是盘根、包目录或其子目录、源码仓库根或源码仓库 `target` 根；产品、架构、版本必须是 JSON string 且匹配，`auto_submit` 和 `chrome_bundled` 必须是 JSON boolean `false`，且不允许额外字段；`test-install-metadata-validator.ps1` 覆盖该门禁且不写启动项、不启动程序、不运行真实 Setup EXE。
- 卸载脚本只接受匹配固定字段集、类型正确、文件清单为预期 AskBridge payload、数据目录属于安装根且开始菜单快捷方式只指向当前用户 `AskBridge.lnk` 的安装清单，清理程序和命名启动项；默认保留 `data`，只有明确确认才递归删除包含登录状态的专用资料。
- 性能脚本把桌面进程和专用 Chrome 分开采样，按实际截止时间报告 CPU、Working Set、Private Bytes、逐样本进程数、逐样本外部连接、当前 Release EXE 路径与哈希和专用 Chrome profile 路径；桌面/Chrome 采样脚本都要求显式传入当前 Release EXE，报告路径必须显式指定。发布前用 `validate-performance-report.ps1` 校验完整证据包，必须同时传入桌面报告、Chrome 报告、准备耗时报、当前 Release EXE 和 AskBridge 专用 Chrome profile；校验覆盖桌面/Chrome 报告字段、当前 Release EXE 路径与哈希、AskBridge 专用 Chrome profile、桌面/Chrome 至少 300 秒采样，以及桌面/Chrome/准备耗时三份报告的测量时间戳，防止沿用旧构建、错误 profile、短采样、无测量时间、来源不明或缺局部报告的性能证据；`test-performance-report-validator.ps1` 覆盖匹配路径和哈希、缺失最终证据路径、旧哈希、相对路径、错误 profile、超限指标、缺失外部连接证据、短采样、缺失 provider 和缺失时间戳。
- `validate-package-artifacts.ps1` 对最终产物目录做完整证据包的非破坏性复核，必须传入预期版本、当前 Release EXE 和当前源树，覆盖顶层产物集合、便携目录精确文件集、隐藏残留、ZIP/Setup 文件头、ZIP 条目名和内容哈希、SHA-256 清单仅包含 ZIP/Setup EXE/`askbridge.exe` 三个目标、包元数据固定字段集、string identity fields 和 boolean safety flags、版本、体积边界、包内 EXE 与当前 Release EXE 哈希一致、包内文档/安装脚本与当前源树一致、不捆绑外部运行时和无额外残留；`test-package-artifact-validator.ps1` 覆盖该校验器的成功和坏包拒绝路径，`test-package.ps1` 覆盖纯打包并复用该校验器，`test-installer.ps1` 覆盖脚本级安装/升级/卸载，`test-setup.ps1` 覆盖自解压、干净退出、首次启动和卸载。
- `test-release-local.ps1` 串联本机安全发布门槛，且要求显式传入仓库 `target` 下的全新绝对 `AcceptanceRoot`：先自检原生命令非零退出码会停止聚合门槛，再执行 PowerShell 脚本语法检查、验收根目录保护自测、打包根目录保护自测、包校验器自测、性能报告校验器自测、安装包元数据安全门禁、临时打包验收、自动测试、Debug/Release 构建和 `git diff --check`；真实浏览器、全局快捷键、安装器启动、长时间性能和最终产物验收仍单独执行。
- 提供隐私说明与逐项发布检查清单。

## 验收门槛

- 全量自动门禁、Debug/Release 构建和真实 Chrome ignored tests；
- D 盘显式临时目录上的首次安装、开机启动持久性、覆盖升级、默认保留数据卸载、明确删除数据卸载，以及真实 Setup EXE 首次启动；
- 5 分钟桌面空闲与专用 Chrome 独立资源测量，冷启动和连续准备耗时记录；
- 真实 UI、四家供应商、安全兜底、无自动发送、日志脱敏和残留清理；
- 全部证据通过后才把版本改为 `1.0.0` 并重新生成最终包。
