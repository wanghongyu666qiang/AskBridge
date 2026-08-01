# Phase 4 实施计划

## 范围

本阶段实现可选择的目标载体基础：现有桌面 PWA 的启动，以及 AskBridge 专用 Chrome 的生命周期和 CDP：

- 为目标载体建立统一 seam，支持 `desktop_pwa` 与 `dedicated_chrome` 两个 adapter；
- ChatGPT 默认发现并启动用户桌面的 `ChatGPT.lnk`，复用其中的登录状态；
- 设置页允许切换 ChatGPT 桌面网页端模式；
- 桌面 PWA adapter 只接受存在的绝对 `.lnk`，不读取 Cookie、历史记录、密码或网页正文；

- 按“用户配置、Windows 注册信息、常见位置”顺序发现 Chrome；
- 创建并保护 AskBridge 数据目录中的 `BrowserProfile`；开发工作区默认将数据目录放在仓库根目录的 `data`，便携版本默认放在程序旁，也允许通过绝对路径 `ASKBRIDGE_DATA_DIR` 覆盖；
- 集中构造专用 Chrome 启动参数；
- 使用 `--remote-debugging-port=0`，从专用目录读取动态端点；
- 只连接回环地址，并验证配置目录、进程和端点的关联；
- 建立 CDP 连接并执行协议握手；
- 枚举、创建和激活页面目标；
- 显式建模 `Confirmed(TargetId)` 与 `Unknown` 聚焦证据；
- 提供有界超时、取消、一次重连和首次登录提示；
- 遵守配置中的 Chrome 生命周期。

专用 Chrome/CDP adapter 只用测试进程在 `127.0.0.1` 随机端口提供的本地页面验证；桌面 PWA adapter 使用用户明确确认的真实快捷方式做启动验收。两种模式都不会实现输入框发现、附件上传、文字插入、剪贴板兜底或自动发送。

## 安全边界

1. 不连接 Chrome 默认用户目录，也不允许把专用目录设置为默认用户目录。
2. 不接受配置或外部进程提供的任意 CDP 地址。
3. 调试端点只来自 AskBridge 专用目录中的 `DevToolsActivePort`。
4. 调试连接只允许 `127.0.0.1`，端口不写入普通日志。
5. 只正常关闭当前 AskBridge 实例确认管理的 Chrome；不强制结束其他 Chrome。
6. 日志只记录状态、目标 ID 等非敏感元数据，不记录页面正文、Cookie、存储、请求正文、问题或截图。
7. 所有连接、等待和重试都有明确上限；UI 消息线程不执行阻塞网络操作。
8. 桌面 PWA 只通过用户选择或自动发现的 `.lnk` 启动，不接管日常 Chrome 的调试端点，不读取登录资料。

## 模块边界

### `askbridge-core`

- `TargetResolver`：根据 URL 匹配集合和可靠聚焦证据作出唯一决策。
- `WorkflowController`：集中约束 Phase 4 的浏览器状态转换。
- 纯逻辑不依赖 Win32、HTTP 或 CDP 类型。

### `askbridge-win`

- `ChromeDiscovery`：用户配置、注册信息和常见位置。
- `ManagedProfile`：展开、规范化和保护专用目录。
- `ChromeManager`：唯一的启动参数构造点，持有本实例启动的子进程。
- `DevToolsEndpoint`：严格解析专用目录中的动态端点。
- `CdpClient`：回环 HTTP/WebSocket、协议请求 ID、目标枚举/创建/激活。
- `BrowserWorker`：在后台线程执行启动、连接和等待，通过消息回到 UI 状态机。
- `DesktopPwaLauncher`：发现、校验并通过 Windows Shell 启动桌面 PWA；不暴露快捷方式或 Shell 细节给调用方。

## 目标选择规则

```text
可靠聚焦目标属于匹配集合
  -> 使用该目标

否则匹配集合为空
  -> 创建新目标

否则匹配集合只有一个
  -> 使用唯一目标

否则
  -> 创建新目标
```

未知聚焦不等于第一个目标。不得用标题、数组顺序或过期的“最后激活时间”猜测。

## 验证

- 单元测试覆盖每条目标选择规则和乱序状态转换；
- 单元测试覆盖路径保护、端点解析、回环限制和协议响应关联；
- 集成测试临时监听 `127.0.0.1:0`，覆盖目标列表、创建和激活；
- 本机使用 AskBridge 专用临时配置启动 Chrome，验证动态端点和 CDP 握手；
- 验证默认 Chrome 配置未被访问，专用 Chrome 可正常关闭且登录目录保留；
- 验证 `Alt+W` 能打开现有 ChatGPT PWA 并复用登录会话，且不会启动专用 Chrome；
- 最终运行 fmt、clippy、test、Debug build 和 Release build。

## 参考行为

- Chrome 136 起，默认用户数据目录会忽略远程调试开关；专用非默认 `--user-data-dir` 是必需边界。
- CDP 浏览器端点提供 `/json/version`、`/json/list`、`PUT /json/new?...` 与 `/json/activate/{id}`。
