# AskBridge 故障排查

本文适用于 AskBridge 1.0。所有运行数据默认位于程序旁的 `data` 目录；开发工作区使用 `D:\AskBridge\data`。除非显式设置绝对路径环境变量 `ASKBRIDGE_DATA_DIR`，AskBridge 不会把项目运行数据迁移到其他磁盘。

## 程序无法启动或没有托盘图标

1. 在任务管理器确认是否已有一个 `askbridge.exe`。AskBridge 只允许一个实例，第二个实例会通知已有实例打开设置后退出。
2. 检查 `data\logs\askbridge.log` 的最后一条结构化事件。日志只记录阶段、结果、请求 ID、供应商 ID、尺寸和错误类别，不记录问题原文、截图、剪贴板、网页正文、Cookie 或调试端口。
3. 如果日志提示配置恢复，检查 `data` 中带时间戳的 `config.corrupt-*.json`。原损坏文件会被保留，程序会创建经过校验的默认配置。
4. 如果 Debug 构建在启动前退出，从 PowerShell 运行 `target\debug\askbridge.exe` 查看加载器错误。正式版仍会显示原生错误对话框。

## 快捷键没有反应

1. 从托盘打开“设置”，确认对应快捷键已启用。
2. 点击“应用更改”。若 Windows 报告组合被占用，AskBridge 会保留旧绑定和旧配置；请改用其他包含修饰键的组合。
3. 本机已知 `Alt+A` 可能与截图工具冲突，项目默认文字入口为 `Alt+W`。
4. “暂停全局快捷键”开启时，托盘仍然可用；从托盘恢复即可。

## 专用 Chrome 无法启动或连接

1. 在设置的“浏览器”页确认 Chrome 路径为空（自动检测）或指向现有的 `chrome.exe`。
2. 点击“检查连接”。该操作只启动 AskBridge 专用 Chrome，并通过本机回环 CDP 连接；不会连接日常 Chrome 配置。
3. 若提示专用配置正在使用，先关闭所有由 AskBridge 启动的 Chrome 窗口，再重试。AskBridge 不会删除仍有活动调试端点的 `DevToolsActivePort`。
4. 首次使用或登录失效时，点击“打开默认供应商登录页面”，只在 AskBridge 专用 Chrome 中自行登录。AskBridge 不读取密码、验证码、Cookie 或网页正文。
5. 不要添加 `--no-sandbox`、`--disable-gpu` 或固定远程调试端口作为规避手段。

## 网页没有收到文字或图片

1. 确认当前页面仍在所选供应商的 HTTPS 边界内。导航到登录页或跨供应商页面时，AskBridge 会停止。
2. 页面存在多个同等可信输入框、没有唯一 PNG 文件控件或页面结构变化时，AskBridge 会进入人工兜底，不会猜测控件或误报成功。
3. 在人工兜底中使用“复制图片”“复制问题”后手动粘贴。关闭对话框时程序会尽力恢复原有 Unicode 文本和位图剪贴板格式。
4. Gemini、Claude 和豆包的图片路径在无法证明唯一附件控件时会安全进入 `CopyImageThenText`；这属于既定安全行为，不等同于自动图片准备成功。
5. 所有 1.0 路径都不会点击发送按钮或模拟发送按键；内容准备后必须由用户在网页中确认发送。

## 截图或多显示器问题

1. `Esc`、右键、窗口关闭和 `WM_CANCELMODE` 会取消框选；正常焦点切换不会取消。
2. 负坐标副屏和混合 DPI 使用虚拟桌面坐标；若出现偏移，请记录显示器相对位置和各自缩放比例，但不要提交真实截图内容。
3. `data\Temp` 中只应短暂出现 `askbridge-*.png`。正常路径会立即删除；启动时只清理超过 24 小时的 AskBridge 命名图片，不触碰其他文件。

## 开机启动

开机启动只写入当前用户的 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\AskBridge`，值为带引号的当前 `askbridge.exe` 绝对路径，不要求管理员权限。关闭设置后会删除该值。卸载脚本也会无条件清理这个命名值，不删除其他启动项。

## 提交诊断信息

可提交程序版本、Windows/Chrome 版本、失败阶段、布尔结果和 `askbridge.log` 中已脱敏的相关事件。提交前仍应人工检查日志。不要提交问题原文、截图、剪贴板内容、网页 HTML、对话、Cookie、令牌、账号或完整聊天 URL。
