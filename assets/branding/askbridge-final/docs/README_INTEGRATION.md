# AskBridge · README 集成说明

> 本文档说明怎么把 `assets/branding/askbridge-final/` 的资源接到项目的
> `README.md`、`README_EN.md`、Windows 安装包资源、托盘图标和未来官网。
> 它是发布流程的可执行清单，不是设计文档（设计规则在 `docs/BRAND_GUIDELINES.md`）。

---

## 1. README 顶部 banner

把 `github/askbridge-readme-header.png`（1600 × 400，~24 KB）放进仓库根
目录，README 顶部用一行标准 Markdown 引用即可：

```markdown
<p align="center">
  <img src="github/askbridge-readme-header.png" alt="AskBridge — screenshot to AI bridge for Windows" width="800">
</p>
```

中文版（`README.md`）和英文版（`README_EN.md`）共用同一张 PNG；纯视觉
资源没有翻译成本。如果以后要加中文 banner，复制 `source/askbridge-readme-header.svg`，
把 "Screenshot → AI bridge for Windows" 改成 "截图 → AI 桥接工具"，再
重新渲染到 `github/askbridge-readme-header-zh.png`。

## 2. 社交预览卡（GitHub Social Preview）

1. 在 GitHub 仓库页面 **Settings → General → Social preview** 上传
   `github/askbridge-social-card.png`（1280 × 640，~30 KB）。
2. 文件一旦上传就会被缓存到 GitHub CDN；以后改图要清浏览器缓存才能预览。

## 3. 头像（用户 / 组织）

- 个人头像：`github/askbridge-avatar.png`（460 × 460，~3.8 KB）。
- 仓库组织头像：同样文件，超过 200 × 200 GitHub 会自动加圆角。
- 透明背景版要避免在暗色主题下出现黑边，**请上传 `askbridge-cream-256.png`**
  的副本作为头像，或者用 `askbridge-avatar.png`（已经带暖白底）。

## 4. favicon（项目官网 / 文档站）

`favicon/` 目录里的所有文件直接拷贝到网站根目录即可：

```text
favicon.ico
favicon-16x16.png
favicon-32x32.png
apple-touch-icon.png
android-chrome-192x192.png
android-chrome-512x512.png
site.webmanifest
browserconfig.xml
```

然后在 HTML `<head>` 里挂上：

```html
<link rel="icon" href="/favicon.ico" sizes="any">
<link rel="icon" type="image/png" sizes="16x16" href="/favicon-16x16.png">
<link rel="icon" type="image/png" sizes="32x32" href="/favicon-32x32.png">
<link rel="apple-touch-icon" sizes="180x180" href="/apple-touch-icon.png">
<link rel="manifest" href="/site.webmanifest">
<meta name="theme-color" content="#FAEEDA">
```

## 5. Windows 应用图标（exe 资源，已接入）

`crates/askbridge-win/askbridge.rc` 同时声明 manifest 与图标：

```rc
1 24 "app.manifest"
1 ICON "../../assets/branding/askbridge-final/icons/askbridge.ico"
```

`crates/askbridge-win/build.rs` 按工具链编译这份 .rc，并把产物链进两个
bin（`askbridge` 和 `askbridge-setup`，安装包 Setup.exe 即后者追加 payload，
图标自动继承）：

- **Windows GNU**：`windres` 编译为 COFF 目标文件后经 `cargo:rustc-link-arg-bin`
  链接（binutils 2.30 已验证可透传 ICO 内的 PNG 条目）。
- **Windows MSVC**：build.rs 定位 Windows SDK 的 `rc.exe`（先 `where`，再
  遍历 `Windows Kits\10\bin\10.*\x64|x86`）编译为 .res 后同样链接。manifest
  由 .rc 的 `RT_MANIFEST` 提供（不再走 `/MANIFEST:EMBED`），与 GNU 行为一致，
  并保证 `askbridge-setup.exe` 因 `asInvoker` 声明不触发 UAC 安装程序检测。

`icons/askbridge.ico` 使用 **cream 变体**（深浅任务栏都立得住），含 8 个
尺寸：16/20/24/32/48/64/128/256。其中 16/20/24 由 `build_pixel_icons.py`
在整像素网格上手工生成，覆盖托盘在 100% / 125% / 150% DPI 下的请求；
其余尺寸为矢量源直接渲染。

运行时加载在 `crates/askbridge-win/src/app_icon.rs`：托盘按 `SM_CXSMICON`
取尺寸，窗口类（设置窗口等，见 `app/events.rs` 的 `register_window_class`）
按 `SM_CXICON`，都从 exe 自身资源加载；资源缺失时回退系统默认图标并记
`warn` 日志。

### 5.1 开始菜单 / 资源管理器图标

快捷方式与 Explorer 图标自动来自 exe 资源，无需额外复制 .ico。若未来需要
独立的 .ico 文件（例如网站下载页），从 `icons/askbridge.ico` 取用即可。

## 6. 截图工具栏 / 设置窗口里的图标

- 设置窗口左上角：`icons/askbridge-cream-64.png`（带暖白底，圆角友好）。
- 截图工具栏右侧的 "AskBridge" 角标：`icons/askbridge-cream-32.png`。
- 浮在屏幕上的 overlay 文字用单色 `icons/askbridge-mono-16.png` 作为
  参考点（overlay 不会真的显示 logo，只是给个版权痕迹）。

## 7. 发布前自检清单

每发一版前，按下面 4 步走一遍：

1. `cd assets/branding/askbridge-final && python scripts/render_assets.py && python scripts/build_pixel_icons.py && python scripts/build_ico.py && python scripts/build_favicon.py && python scripts/build_review_sheets.py && python scripts/build_inventory.py` —— 六脚本全跑一次，确认无报错（改了小尺寸像素版时 `build_pixel_icons.py` 必跑）。
2. 打开 `docs/contact-sheet.png` 和 `docs/favicon-readability-sheet.png`，
   确认 16 px 列还能看出 "框 + 桥" 的轮廓。
3. `cargo build --release`，确认 `.rc`（windres / rc.exe）编译成功。
4. 在 Windows 资源管理器里右键 `target/release/askbridge.exe` → 属性 →
   详细信息，看图标是不是新的（多尺寸 ICO 一般会显示 256×256）。

## 8. 修改 logo 的工作流

如果以后要改 logo（例如换配色、加新变体），按以下顺序操作：

1. 改 `source/askbridge-*.svg`。
2. 跑上面那 5 个脚本重新生成全套。
3. 跑 `git diff docs/contact-sheet.png docs/favicon-readability-sheet.png`，
   肉眼确认无回归。
4. 提交：`source/*.svg`、`scripts/*.py`、`icons/*.png`、`web/*.png`、
   `favicon/*`、`github/*.png`、`docs/*.png`、`docs/BRAND_GUIDELINES.md`、
   `INVENTORY.md`。
5. **不要**单独提交某几个尺寸的 PNG，CI 重新跑就能复现全套，避免二进制
   漂移。

---

_对应代码版本：见 `Cargo.toml` 的 `[workspace.package] version`。本文件与
`docs/BRAND_GUIDELINES.md` 同步更新。_
