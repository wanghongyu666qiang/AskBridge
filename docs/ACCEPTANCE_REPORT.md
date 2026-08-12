# AskBridge 开发与验收报告

更新时间：2026-08-11

## 当前结论

Phase 0–8 的候选实现和本机可执行的真实 Windows/安装器验收已经通过。一次独立的规范/代码标准审查发现的高风险项也已修复：旧任务取消不会被新任务复活，浏览器失败保留请求并进入兜底，配置使用 Windows 原子替换，运行时日志/Chrome 设置立即刷新，附件必须出现网页回执且结束忙碌状态，安装器选择的开机启动能跨首次启动保留，生成的 Setup EXE 经过实际安装链路验收。

当前仍保持预发布版本，不能标记 `1.0.0`。尚未闭环的条件不止单一硬件场景：

1. `D:\AskBridge\data\BrowserProfile` 当前停在 `https://claude.ai/login`；真实供应商测试已改为“登录不足即失败”，登录后的 Claude 文字准备仍需复验。
2. 新的附件完成回执门禁已通过回环 Chrome；ChatGPT 真实图片路径需在专用配置可用后重跑，不能沿用旧的“文件控件已有文件”证据。
3. 当前机器只有 1 台 2560×1600 显示器，桌面 DPI 为 144（150%）；应用的 Per-Monitor V2 日志使用物理虚拟桌面 `(0,0) 2560×1600`。Windows 10/11、双屏/三屏、左侧/上方负坐标、100/125/200%、混合 DPI、深浅色与高对比度的完整人工矩阵没有可用硬件证据。
4. 规格中的 Chrome 真实异常矩阵（未安装、手动路径、用户中途关闭、端点不可用、配置被占用等）已有单元/回环覆盖，但尚未逐项在独立真实机器上人工执行。
5. 最终版本的 5 分钟桌面/Chrome 性能与二进制哈希必须在版本改为 `1.0.0` 后重跑；现有报告只证明预发布候选。
6. 最终 ZIP/Setup/SHA-256 的保存目录需要用户明确指定；`scripts\package.ps1` 强制要求绝对 `ArtifactRoot`，不会默认写入任何位置。

## 已实现

- 原生 Win32 托盘、单实例、三个可配置全局快捷键、暂停与原子回滚。
- 多显示器虚拟桌面截图、原生问题窗口、三种请求入口、忙碌/取消状态机。
- ChatGPT 桌面 PWA 启动 seam 与隔离的 AskBridge 专用 Chrome/CDP。
- 通用 `ProviderAdapter.prepare(...)`、四家内置规则、文字/图片准备验证和安全人工兜底。
- `auto_submit=false` 固定边界；没有自动发送开关、模型 API、内嵌浏览器或网络下发规则。
- 四页原生设置、自定义 HTTPS 供应商、快速提示词、Chrome 生命周期和浏览器工具。
- 浏览器生命周期使用可序列化枚举；“每次准备后询问关闭”只在用户确认已完成发送后正常关闭受管 Chrome。
- 当前用户 Run 键开机启动、配置迁移/损坏备份、结构化轮换日志、临时图片定向清理。
- 用户级安装、覆盖升级、默认保留数据卸载、明确删除数据卸载、便携 ZIP、IExpress Setup 生成和真实 Setup 验收脚本。

## 当前自动门禁

2026-08-10 对当前源树执行并通过；2026-08-11 已将 `scripts\build.ps1` 与 `scripts\test.ps1` 对齐为离线门禁，并通过脚本入口复跑：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
cargo build --workspace --offline
cargo build --workspace --release --offline
git diff --check
```

测试结果：

- `askbridge-core`：51 passed；
- `askbridge-win`：89 passed、4 ignored；
- 4 个 ignored 项都是需要真实 Chrome 或长时间采样的显式验收测试，不属于默认单元测试失败。

GNU 链接器仍输出非致命的 `corrupt .drectve at end of def file` 警告；所有相关命令退出码为 0。

## 真实 Windows 与浏览器验收

### 设置与系统集成

`scripts\test-settings-ui.ps1` 在隔离 D 盘数据目录上通过：

- 四个设置分页和关键控件存在；
- 自定义 HTTPS 供应商通过真实 UI 添加并读回，默认供应商切换到 Gemini 后读回，再恢复为 ChatGPT 并清空自定义项；
- 快速提示词通过真实 Win32 控件写入并持久化；
- 开机启动启用、注册表校验、禁用和原值恢复通过；
- 结构化日志写入 D 盘；
- 真实进程恢复验收通过：损坏配置恢复为 schema v3 并生成恰好一个 `config.corrupt-*.json` 备份；5,242,881 B 旧日志完整轮换到 `askbridge.log.1`，新活动日志创建成功；隔离目录和 Run 值在测试后恢复/清理。
- 设置窗口按当前桌面 144 DPI 缩放；UI Automation 确认关键控件可见并完成读写。本轮临时截图位于隔离目录内并已随测试清理，没有保留或声称完成跨主题视觉矩阵。
- 三个默认组合的重复注册探针均得到 Win32 1409，证明系统注册存在；标准 `WM_HOTKEY` 路由验证 `Alt+W` 打开问题窗口、`Alt+Q` 完成 120×90 内存截图后打开问题窗口、`Alt+Shift+Q` 进入截图并可用 Esc 取消。
- 设置 UI 真实修改 `Alt+Q` 为 `Ctrl+Alt+Shift+F12`、禁用、恢复默认和冲突回滚均通过；第二实例退出且只保留原进程，调试日志开关无需重启即可重载。
- `WM_HOTKEY` 到文字框 153 ms；到截图遮罩 11 ms；鼠标抬起到带图问题框 145 ms；到取消遮罩 58 ms。
- 热键/截图验收恢复了原鼠标位置，未修改剪贴板、未保存截图，并删除了隔离 D 盘数据目录。

### CDP 与供应商

- 本地回环真实 Chrome 测试通过：动态端点、目标创建/激活、页面就绪、文字准备、图片文件设置、可见附件回执与忙碌结束、登录分类、正常关闭和临时配置删除。
- ChatGPT：真实文字写入通过；1×1 PNG 附件与文字同时准备通过；未发送。
- Gemini：真实文字写入通过；未发送。
- 豆包：真实文字写入通过；未发送。
- Claude：当前页面为 `https://claude.ai/login`；当前安全返回登录不足，测试不再把 `LoginInBrowser` 当作成功。登录后写入待复验。
- ChatGPT 连续两次真实准备通过，浏览器均通过 `Browser.close` 正常退出。
- `D:\AskBridge\data\BrowserProfile\Temp` 没有残留 `askbridge-*.png`。

## 性能证据

报告均在 `D:\AskBridge\target\acceptance`：

| 指标 | 结果 |
| --- | ---: |
| 桌面进程冷启动 | 331.93 ms |
| 桌面进程空闲 CPU | 0%（整机） |
| 桌面进程平均 / 峰值 Working Set | 12.33 / 12.50 MiB |
| 桌面进程外部 TCP 连接 | 0 |
| 桌面常驻进程数 | 1 |
| 专用 Chrome 实际采样窗口 | 300.74 s，249 个样本 |
| 专用 Chrome 最大进程数 | 10 |
| 专用 Chrome 平均 / 峰值 Working Set | 880.80 / 944.89 MiB |
| 专用 Chrome 平均 Private Bytes | 535.96 MiB |
| 专用 Chrome 进程树累计 CPU | 约单核 1% |
| 专用 Chrome 冷启动 | 596.60 ms |
| 首次 / 连续准备 | 6241.58 / 3892.78 ms |
| 当前 Release EXE | 1.11 MiB |

证据文件：

- `askbridge-desktop-performance.json`
- `askbridge-chrome-performance.json`
- `askbridge-preparation-timings.json`

桌面性能脚本已改为按实际截止时间采样，并逐样本记录同路径进程数和外部 TCP 连接数，不再硬编码进程数或只在结束时采一次网络。最终版本号变更和 Release 重建会改变二进制哈希，因此在标记 `1.0.0` 后必须重跑 5 分钟报告，不能沿用上表的预发布哈希。

2026-08-11 增加 `scripts\validate-performance-report.ps1`，用于发布前校验桌面性能报告、Chrome 性能报告和准备耗时报告的基本字段。2026-08-12 进一步收紧为完整最终证据包：`ChromeReportPath`、`TimingsReportPath`、`ExecutablePath` 和 `ExpectedChromeProfilePath` 均为必需；脚本会确认桌面和 Chrome 报告中的 Release EXE 路径与哈希仍对应当前二进制，并要求 Chrome 报告的 `profile_path` 是绝对路径且匹配 AskBridge 专用资料目录。本轮核对发现当前 `target\acceptance\askbridge-desktop-performance.json` 记录的哈希已不同于当前 `target\release\askbridge.exe`，因此这些性能数据只能作为预发布历史证据，最终发布前必须按用户指定目录重跑并重新校验。

`scripts\test-performance-report-validator.ps1` 已在 `D:\AskBridge\target\codex-performance-validator` 执行并通过；目录在结束时清理：

- 匹配当前 EXE 路径和哈希、桌面/Chrome 时长达到 300 秒且包含进程数、外部连接数和 provider 等必要证据的完整桌面、Chrome 和准备耗时报报告会被接受；
- 缺少 Chrome 报告、准备耗时报、当前 Release EXE 或 AskBridge 专用 Chrome profile 的最终校验会被拒绝；
- `measure-chrome-performance.ps1` 必须显式传入当前 Release EXE，避免生成没有 `executable_sha256` 的 Chrome 性能报告；
- EXE 路径或内容变化后，旧桌面或旧 Chrome 报告会因 `executable` 或 `executable_sha256` 不匹配被拒绝；
- 相对路径会被拒绝，避免验收报告位置含糊；
- Chrome 报告的 `profile_path` 缺失、相对或不匹配 AskBridge 专用资料目录时会被拒绝；
- 超过桌面 CPU 验收上限的报告会被拒绝；
- 缺失桌面外部 TCP 连接证据的报告会被拒绝，不能把缺字段当作 0 连接。
- 缺失或无法解析 `measured_at` 的桌面/Chrome 性能报告、低于 300 秒的桌面/Chrome 性能报告，以及缺少 provider 或 `measured_at_unix_ms` 的准备耗时报报告会被拒绝，不能把短采样、无测量时间或来源不明的耗时当最终证据。

## 安装、升级与卸载

`scripts\test-installer.ps1` 已在 `D:\AskBridge\target\phase8-installer-final` 执行并通过；目录在结束时清理：

1. 首次安装到显式 D 盘目标；
2. `-StartOnLogin` 同时写配置与当前用户 Run，首次启动后仍保留；
3. 覆盖升级保留 `data` 哨兵；
4. 默认卸载保留 `data`；
5. 明确 `RemoveData` 后删除数据；
6. 当前用户 AskBridge Run 值在测试后恢复；
7. 验收目录由脚本清理。

安装与卸载均不要求管理员权限、不捆绑 Chrome、不写全局 Rust/Chrome 配置。最终包尚未生成，等待用户指定 `ArtifactRoot`。

`scripts\test-package.ps1` 已在 `D:\AskBridge\target\codex-package-acceptance` 执行并通过；目录在结束时清理：

- `package.ps1` 只能写入显式 D 盘临时根下的 `package` 目录；
- `validate-package-artifacts.ps1` 对生成目录执行非破坏性复核；
- 产物集合包含且仅包含一个便携目录、一个 ZIP、一个 Setup EXE 和一个 SHA-256 清单，ArtifactRoot 顶层不允许混入 `.sed`、日志、旧包或其他残留；
- 便携目录恰好包含 7 个预期文件且无子目录，隐藏文件/目录也不得残留；ZIP 必须具备 ZIP 文件头，Setup EXE 必须具备 PE DOS 头、有效 PE 偏移和 `PE\0\0` 签名，ZIP 内部条目名和逐文件 SHA-256 与该扁平便携目录一致；
- 产物名称和 `package.json` 版本与当前 `askbridge-win` Cargo 元数据一致；
- 包内 README、隐私说明、故障排查和安装/卸载脚本与当前源树文件哈希一致；
- SHA-256 清单只允许 ZIP、Setup EXE 和 `askbridge.exe` 三个目标，且逐项重新计算一致；
- `package.json` 只允许固定 1.0 字段集，`product`/`version`/`architecture` 必须是 JSON string，且 `auto_submit` 和 `chrome_bundled` 必须是 JSON boolean `false`；
- 便携目录未捆绑 Chrome、Rust、Cargo、DLL 或 MSI；
- 包内 `askbridge.exe` 与当前 `target\release\askbridge.exe` 哈希一致；
- Release EXE、Setup EXE 和静态资源体积在发布检查边界内。

2026-08-11 增加 `scripts\validate-package-artifacts.ps1`，用于对用户最终指定的发布目录做只读校验，不重新打包也不删除目录。本轮通过 `scripts\test-package.ps1 -AcceptanceRoot D:\AskBridge\target\codex-package-validator` 验证了该校验器能检查真实生成的便携目录、ZIP、Setup EXE、SHA-256、包元数据、Cargo 版本一致性、ZIP 内容一致性、包内 EXE 与当前 Release EXE 哈希一致性、包内文档/安装脚本与当前源树一致性、体积和外部运行时排除；临时目录在结束时清理。

`scripts\test-package-artifact-validator.ps1` 已在 `D:\AskBridge\target\codex-package-artifact-validator` 执行并通过；目录在结束时清理：

- 最小合法发布目录会被 `validate-package-artifacts.ps1` 接受；
- 缺少 `ExpectedVersion`、`ExpectedReleaseExePath` 或 `ExpectedSourceRoot` 的最终校验，以及相对 `ArtifactRoot`、相对 `ExpectedReleaseExePath`、版本不匹配、损坏哈希、多余 SHA-256 目标、错误 ZIP/Setup 文件头、ZIP 内容不一致、包内 EXE 与指定 Release EXE 不一致、包内文档/安装脚本与指定源树不一致、捆绑外部运行时和额外顶层残留都会被拒绝。

2026-08-12 增加 `scripts\test-release-local.ps1`，用于串联本机可自动执行且不触发真实浏览器、全局快捷键、安装器启动或长时间性能采样的发布门槛。该脚本要求显式传入仓库 `target` 下的全新绝对 `AcceptanceRoot`，不会使用默认验收目录。本轮已在 `D:\AskBridge\target\codex-release-local` 执行并通过；它覆盖 PowerShell 脚本语法检查、验收根目录保护、包校验器坏包自测、性能报告校验器坏报告自测、安装包元数据安全门禁、临时打包验收、`scripts\test.ps1`、`scripts\build.ps1` 和 `git diff --check`，并在结束时清理临时目录。`-SelfTestFailureHandling` 已验证原生命令非零退出码会停止聚合门槛。

2026-08-12 增加 `scripts\test-acceptance-root-guards.ps1`，在不启动 AskBridge、不写注册表、不打开浏览器、不触发截图的前提下验证验收脚本会先拒绝缺省或相对验收根、仓库 `target` 外验收根和已存在目录。`scripts\test-settings-ui.ps1` 现在还要求截图路径位于隔离的 `AcceptanceDataRoot` 内，避免为任意绝对截图路径创建仓库外目录。

2026-08-12 增加 `scripts\test-package-root-guards.ps1`，验证 `scripts\package.ps1` 会在构建、IExpress 或文件复制前拒绝相对 `ArtifactRoot`、仓库根、仓库 `target` 根和非空产物目录。`package.ps1` 现在只接受不存在或为空的专用绝对产物目录，避免最终 ZIP/Setup/SHA-256 与旧产物或源码目录混写。

2026-08-12 增加 `scripts\test-install-metadata-validator.ps1`，在不写启动项、不启动程序、不运行真实 Setup EXE 的前提下验证 `Install-AskBridge.ps1` 会接受安全 `package.json`，并拒绝仓库根、仓库 `target` 根、包目录等不安全安装目标，以及 `auto_submit=true`、`chrome_bundled=true`、字符串 `"false"` 类型的安全开关、非字符串产品/版本/架构、错误架构、缺失版本和额外字段的安装包元数据。

`scripts\test-setup.ps1` 已在 `D:\AskBridge\target\phase8-setup-final` 执行并通过；目录在结束时清理：

- 三项 SHA-256 清单重新计算一致；
- IExpress 输出在文件大小稳定且可独占读取后才进入哈希步骤；
- Setup EXE 使用显式 D 盘安装根，完成自解压和安装后干净退出，不残留完成提示进程；
- 已安装 AskBridge 首次启动保持运行；
- 真实卸载删除程序和隔离数据；
- ZIP 和 Setup 不捆绑 Chrome。

2026-08-11 临时打包烟测发现旧 `package.ps1` 可能在 IExpress 输出尚未完全稳定时写入 Setup EXE 哈希，导致 `SHA256SUMS.txt` 与最终文件不一致；已修复为连续多次确认文件长度和写入时间稳定、可独占读取后再写清单，并在脚本内部立即逐项复核。修复后在 `D:\AskBridge\target\codex-package-smoke-2` 重新生成临时包并通过外部哈希复核；该目录只作为本轮烟测证据，不是最终交付目录。

2026-08-12 `validate-package-artifacts.ps1` 收紧为完整最终包证据校验：必须传入预期版本、当前 Release EXE 和当前源树，且 ArtifactRoot 顶层只允许便携目录、ZIP、Setup EXE 和 SHA-256 清单；本轮聚合门禁据此发现 IExpress 生成后残留 `~AskBridge-<version>-Setup.CAB`，已修复 `package.ps1` 在 Setup EXE 稳定后清理该临时 CAB，再进入哈希写入和产物校验。

## 1.0 收尾顺序

1. 用户在已打开的 AskBridge 专用 Chrome 中重新登录 Claude。
2. 重跑 Claude 真实文字准备，确认 `text_inserted=true` 且未发送。
3. 重跑 ChatGPT 真实图片准备，确认网页附件回执成立；若无法证明则明确验收安全人工兜底，不声称自动附件成功。
4. 在可用机器完成 Windows/显示器/DPI/主题和 Chrome 异常矩阵，或由用户对无法提供的外部矩阵作明确发布豁免。
5. 用户明确给出最终绝对产物目录和性能报告目录。
6. 把 workspace 版本改为 `1.0.0`，重跑完整门禁、设置 UI、关键 Chrome 验收及最终 5 分钟 Release 性能/哈希校验。
7. 运行 `scripts\package.ps1 -ArtifactRoot <用户指定绝对目录>`，用 `validate-package-artifacts.ps1 -ExpectedVersion 1.0.0 -ExpectedReleaseExePath D:\AskBridge\target\release\askbridge.exe -ExpectedSourceRoot D:\AskBridge` 复核最终目录，再用真实 Setup 做一次最终安装/卸载烟测。
