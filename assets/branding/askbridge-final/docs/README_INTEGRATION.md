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

## 5. Windows 应用图标（askbridge.exe 资源）

把 `icons/askbridge.ico`（5.9 KB，含 6 个尺寸：16/32/48/64/128/256）作为
Windows 资源文件嵌入 `askbridge.exe`。两种接入方式：

### 5.1 用 `winres`（推荐，GNU 与 MSVC 工具链都行）

```toml
# crates/askbridge-win/Cargo.toml
[dependencies]
embed-resource = "3"

[build-dependencies]
embed-resource = "3"
```

```rust
// crates/askbridge-win/build.rs
fn main() {
    embed_resource::compile(
        "../../assets/branding/askbridge-final/icons/askbridge.ico",
        embed_resource::NONE,
    );
}
```

### 5.2 直接塞进 `.rc`

```rc
// crates/askbridge-win/resources/askbridge.rc
1 ICON "../../assets/branding/askbridge-final/icons/askbridge.ico"
```

然后 `windres` 或 MSVC `rc.exe` 编译进二进制。

### 5.3 安装包 / 便携版的快捷方式图标

`build.ps1` / `package.ps1` 阶段把这个文件复制到 `target/release/` 或
便携版根目录，让用户在 Explorer 里看到正确图标：

```powershell
Copy-Item `
  assets\branding\askbridge-final\icons\askbridge.ico `
  target\release\askbridge.ico -Force
```

托盘图标的运行时缩放在 `crates/askbridge-win/src/tray.rs` 里；用
`icons/askbridge-transparent-32.png` 作为托盘源（Windows 托盘默认 16×16，
会自动缩到 16，多给 32 是为了 HiDPI）。PNG → HICON 的转换可以继续用
`winapi` 的 `CreateIconFromResourceEx`，或者直接用 `ico` 里的 16/32 资源。

## 6. 截图工具栏 / 设置窗口里的图标

- 设置窗口左上角：`icons/askbridge-cream-64.png`（带暖白底，圆角友好）。
- 截图工具栏右侧的 "AskBridge" 角标：`icons/askbridge-cream-32.png`。
- 浮在屏幕上的 overlay 文字用单色 `icons/askbridge-mono-16.png` 作为
  参考点（overlay 不会真的显示 logo，只是给个版权痕迹）。

## 7. 发布前自检清单

每发一版前，按下面 4 步走一遍：

1. `cd assets/branding/askbridge-final && python scripts/render_assets.py && python scripts/build_ico.py && python scripts/build_favicon.py && python scripts/build_review_sheets.py && python scripts/build_inventory.py` —— 五脚本全跑一次，确认无报错。
2. 打开 `docs/contact-sheet.png` 和 `docs/favicon-readability-sheet.png`，
   确认 16 px 列还能看出 "框 + 桥" 的轮廓。
3. `cargo build --release`，确认 `.rc` / `embed_resource` 成功。
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
