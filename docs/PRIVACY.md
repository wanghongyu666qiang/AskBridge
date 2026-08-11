# AskBridge 隐私说明

版本：1.0

AskBridge 是本机 Windows 快捷操作层，不是 AI 服务。它把用户主动输入的问题和可选截图准备到用户选择的 AI 网页编辑区，最终发送必须由用户在网页中确认。

## AskBridge 会处理什么

- 用户主动框选的屏幕区域：先以 RGBA 保存在当前进程内存中；网页附件接口需要路径时，短暂写入 AskBridge 数据目录的 `Temp` 子目录，成功、失败或取消后删除。
- 用户主动输入的问题、快速提示词和所选供应商：只用于当前内存工作流；问题原文不写入日志或历史记录。
- AskBridge 设置：保存在 AskBridge 数据目录的 `config.json`。
- 专用 Chrome 登录状态：由 Chrome 保存在 AskBridge 数据目录的 `BrowserProfile`。其中可能含供应商登录状态；AskBridge 不读取密码、验证码、Cookie、Local Storage、历史对话或网页正文。

## AskBridge 不会做什么

- 不调用模型 API，不运行本地模型，不在本地渲染回答；
- 不自动点击发送按钮，不模拟发送按键，1.0 的 `auto_submit` 始终为 `false`；
- 不连接日常 Chrome 用户目录，不读取其 Cookie、密码、历史记录或网页内容；
- 不保存截图、问题历史、聊天 ID 或对话正文；
- 不使用浏览器扩展、WebView、Electron、Tauri 或常驻 Python 进程；
- 正式程序不监听本地 HTTP 服务，不把 Chrome 调试端口开放到局域网；
- 不要求管理员权限，不安装全局键盘钩子，不修改系统代理，不关闭安全软件，不禁用 Chrome sandbox。

## 日志

日志位于 `<AskBridge 数据目录>\logs\askbridge.log`，超过 5 MiB 时保留一份轮换文件。允许字段包括时间、程序版本、请求 ID、投递模式、供应商 ID、阶段、耗时、错误类别和布尔结果。禁止字段包括问题原文、截图或 Base64、剪贴板、页面 HTML、对话、Cookie、Local Storage、令牌、用户名密码、CDP 完整正文、调试端口和含会话标识的完整聊天 URL。

## 剪贴板人工兜底

自动化不确定时 AskBridge 会停止并可进入原生人工兜底。打开兜底窗口时会先把当前请求的截图或问题文字放入剪贴板，窗口内的“复制图片”和“复制问题”可在两者之间切换；对话框关闭后尽力恢复先前的 Unicode 文本和位图格式。Windows 剪贴板由多个进程共享，因此无法保证恢复未支持的私有格式。

## 数据位置、升级和卸载

开发工作区使用 `D:\AskBridge\data`。便携或安装版本默认使用 `askbridge.exe` 同目录下的 `data`，也可由用户显式设置绝对 `ASKBRIDGE_DATA_DIR`。升级保留 `data`。卸载默认保留配置和专用 Chrome 资料；只有用户明确输入确认或传入 `-RemoveData` 时才删除，并会提示其中可能含登录状态。

## 第三方网页

网页内容准备完成后，AI 供应商将按其自身隐私政策处理用户最终确认发送的内容。AskBridge 不代表这些供应商，也不改变其账号、数据保留或内容政策。
