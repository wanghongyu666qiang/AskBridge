//! The settings page rendered inside the WebView2 child window.
//!
//! The page is a single self-contained document: no network requests, no
//! remote assets, and no navigation. It receives one JSON snapshot of the
//! configuration, edits a local draft, and posts that draft back over IPC.
//! Validation stays on the Rust side; the page only reports what the user
//! typed.

use askbridge_core::{AppError, Result};

/// Placeholder replaced with the JSON view model. Kept as a token rather than
/// `format!` so the CSS braces stay readable.
const VIEW_MODEL_TOKEN: &str = "/*__VIEW_MODEL__*/null";

pub(super) fn settings_html(view_model_json: &str) -> Result<String> {
    if view_model_json.contains("</script") || view_model_json.contains("<!--") {
        return Err(AppError::ConfigurationInvalid(
            "settings view model contains an unsafe script terminator".to_owned(),
        ));
    }
    Ok(PAGE.replace(VIEW_MODEL_TOKEN, view_model_json).to_owned())
}

const PAGE: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="color-scheme" content="light">
<title>AskBridge 设置</title>
<style>
* { box-sizing: border-box; }
:root {
  --bg: #fbf9f6;
  --surface: #ffffff;
  --surface-sunken: #f6f3ee;
  --text: #1b1b1b;
  --muted: #6b6862;
  --faint: #9a968f;
  --line: #e8e3db;
  --line-strong: #d6d0c6;
  --accent: #993c1d;
  --accent-hover: #ad4928;
  --accent-press: #8c3419;
  --accent-soft: #f7e9e3;
  --ok: #1d7a4c;
  --ok-soft: #e6f4ec;
  --warn: #8a5a00;
  --warn-soft: #fdf1d8;
  --danger: #a32020;
  --danger-soft: #fbe9e9;
  --radius: 14px;
  --radius-sm: 9px;
  --ease-out: cubic-bezier(.22, 1, .36, 1);
  --ease-spring: cubic-bezier(.16, 1.16, .3, 1);
}
html, body {
  height: 100%;
  margin: 0;
  overflow: hidden;
  background: var(--bg);
  color: var(--text);
  font: 400 14px/1.5 "Segoe UI Variable Text", "Microsoft YaHei UI", "Segoe UI", sans-serif;
  -webkit-font-smoothing: antialiased;
}
body { display: flex; flex-direction: column; }
body, button, input, textarea { font-family: inherit; }
::selection { background: var(--accent-soft); }
::-webkit-scrollbar { width: 10px; height: 10px; }
::-webkit-scrollbar-thumb {
  border: 3px solid transparent;
  border-radius: 999px;
  background: var(--line-strong);
  background-clip: padding-box;
}
::-webkit-scrollbar-thumb:hover { background: var(--faint); background-clip: padding-box; }

/* ---------- header ---------- */
header {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 22px 28px 18px;
  border-bottom: 1px solid var(--line);
}
/* The mark keeps its own warm cream plate: the brand rule is charcoal
   brackets on cream with a terracotta bridge, so recoloring it to the accent
   would break the identity. */
.mark {
  width: 44px;
  height: 44px;
  flex: 0 0 44px;
  border-radius: 12px;
  background: #faeeda;
  border: 1px solid #ecdfc8;
  display: grid;
  place-items: center;
  box-shadow: 0 1px 3px rgba(44, 44, 42, .10);
}
.mark svg { width: 30px; height: 30px; color: #2c2c2a; }
.mark .mark-bridge { fill: #d85a30; }
.title-block { min-width: 0; }
h1 {
  margin: 0;
  font-size: 17px;
  font-weight: 600;
  letter-spacing: .2px;
}
.subtitle {
  margin: 2px 0 0;
  font-size: 12.5px;
  color: var(--muted);
}
.version {
  margin-left: auto;
  flex: 0 0 auto;
  padding: 4px 9px;
  border-radius: 999px;
  background: var(--surface-sunken);
  border: 1px solid var(--line);
  font-size: 11.5px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}

/* ---------- shell ---------- */
.shell { flex: 1; display: flex; min-height: 0; }
nav {
  width: 178px;
  flex: 0 0 178px;
  padding: 16px 12px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  position: relative;
}
.pill {
  position: absolute;
  left: 12px;
  right: 12px;
  height: 38px;
  border-radius: 10px;
  background: var(--surface);
  border: 1px solid var(--line);
  box-shadow: 0 1px 2px rgba(27, 27, 27, .05);
  transition: transform 320ms var(--ease-spring);
  pointer-events: none;
}
.nav-item {
  position: relative;
  z-index: 1;
  height: 38px;
  padding: 0 12px;
  display: flex;
  align-items: center;
  gap: 10px;
  border: 0;
  border-radius: 10px;
  background: transparent;
  color: var(--muted);
  font-size: 13.5px;
  text-align: left;
  cursor: default;
  transition: color 140ms ease;
}
.nav-item svg {
  width: 17px;
  height: 17px;
  flex: 0 0 17px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.7;
  stroke-linecap: round;
  stroke-linejoin: round;
  transition: stroke 140ms ease;
}
.nav-item:hover { color: var(--text); }
.nav-item.active { color: var(--accent); font-weight: 600; }
.nav-foot {
  margin-top: auto;
  padding: 10px 12px 2px;
  font-size: 11.5px;
  line-height: 1.55;
  color: var(--faint);
}

main { flex: 1; min-width: 0; overflow-y: auto; padding: 22px 28px 26px; }
.page { display: none; }
.page.active { display: block; animation: page-in 260ms var(--ease-out); }
@keyframes page-in {
  from { opacity: 0; transform: translateY(10px); }
  to { opacity: 1; transform: none; }
}
.page.active .card {
  animation: card-in 380ms var(--ease-out) backwards;
}
.page.active .card:nth-child(1) { animation-delay: 30ms; }
.page.active .card:nth-child(2) { animation-delay: 80ms; }
.page.active .card:nth-child(3) { animation-delay: 130ms; }
.page.active .card:nth-child(4) { animation-delay: 180ms; }
@keyframes card-in {
  from { opacity: 0; transform: translateY(14px) scale(.994); }
  to { opacity: 1; transform: none; }
}

/* ---------- cards ---------- */
.card {
  margin: 0 0 16px;
  padding: 18px 20px 20px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--surface);
  box-shadow: 0 1px 2px rgba(27, 27, 27, .04);
}
.card > h2 {
  margin: 0 0 3px;
  font-size: 13.5px;
  font-weight: 600;
  letter-spacing: .2px;
}
.card > .hint {
  margin: 0 0 16px;
  font-size: 12px;
  color: var(--muted);
}
.card > .hint:last-child { margin-bottom: 0; }
.row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 11px 0;
  border-top: 1px solid var(--line);
}
.row:first-of-type { border-top: 0; padding-top: 2px; }
.row-text { min-width: 0; flex: 1; }
.row-label { font-size: 13.5px; }
.row-desc { margin-top: 2px; font-size: 12px; color: var(--muted); }
.row-control { flex: 0 0 auto; display: flex; align-items: center; gap: 10px; }

/* ---------- inputs ---------- */
input[type=text], textarea {
  width: 100%;
  padding: 8px 11px;
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text);
  font-size: 13.5px;
  outline: none;
  transition: border-color 130ms ease, box-shadow 130ms ease, background-color 130ms ease;
}
input[type=text]:hover, textarea:hover { border-color: var(--faint); }
input[type=text]:focus, textarea:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(153, 60, 29, .13);
}
input[type=text][readonly] {
  background: var(--surface-sunken);
  color: var(--muted);
}
input[type=text].invalid, textarea.invalid {
  border-color: var(--danger);
  box-shadow: 0 0 0 3px rgba(163, 32, 32, .12);
}
textarea { resize: vertical; min-height: 84px; line-height: 1.6; }
.mono { font-family: "Cascadia Mono", Consolas, "Microsoft YaHei UI", monospace; font-size: 12.5px; }
.hotkey { width: 168px; text-align: center; letter-spacing: .4px; font-variant-numeric: tabular-nums; }
.hotkey.recording {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--accent);
  box-shadow: 0 0 0 3px rgba(153, 60, 29, .13);
}
.url { width: 100%; }

/* ---------- switch ---------- */
.switch {
  position: relative;
  width: 42px;
  height: 24px;
  flex: 0 0 42px;
  border: 0;
  padding: 0;
  border-radius: 999px;
  background: var(--line-strong);
  cursor: default;
  transition: background-color 200ms var(--ease-out);
}
.switch::after {
  content: "";
  position: absolute;
  top: 3px;
  left: 3px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, .22);
  transition: transform 300ms var(--ease-spring);
}
.switch:hover { background: #c7c1b6; }
.switch[aria-checked=true] { background: var(--accent); }
.switch[aria-checked=true]:hover { background: var(--accent-hover); }
.switch[aria-checked=true]::after { transform: translateX(18px); }
.switch:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }

/* ---------- segmented ---------- */
.segmented {
  display: inline-flex;
  padding: 3px;
  gap: 2px;
  border: 1px solid var(--line);
  border-radius: 11px;
  background: var(--surface-sunken);
}
.segmented button {
  height: 30px;
  padding: 0 14px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  font-size: 13px;
  cursor: default;
  transition: background-color 160ms ease, color 160ms ease, box-shadow 160ms ease;
}
.segmented button:hover { color: var(--text); }
.segmented button[aria-pressed=true] {
  background: var(--surface);
  color: var(--text);
  font-weight: 500;
  box-shadow: 0 1px 3px rgba(27, 27, 27, .1);
}

/* ---------- radios ---------- */
.option {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  background: var(--surface);
  cursor: default;
  transition: border-color 150ms ease, background-color 150ms ease, transform 150ms var(--ease-out);
}
.option + .option { margin-top: 8px; }
.option:hover { border-color: var(--line-strong); background: #fdfcfa; }
.option:active { transform: scale(.995); }
.option[aria-checked=true] {
  border-color: var(--accent);
  background: var(--accent-soft);
}
.option .dot {
  position: relative;
  width: 17px;
  height: 17px;
  flex: 0 0 17px;
  margin-top: 2px;
  border: 1.5px solid var(--line-strong);
  border-radius: 50%;
  transition: border-color 150ms ease;
}
.option .dot::after {
  content: "";
  position: absolute;
  inset: 3px;
  border-radius: 50%;
  background: var(--accent);
  transform: scale(0);
  transition: transform 240ms var(--ease-spring);
}
.option[aria-checked=true] .dot { border-color: var(--accent); }
.option[aria-checked=true] .dot::after { transform: scale(1); }
.option .option-label { font-size: 13.5px; }
.option .option-desc { margin-top: 2px; font-size: 12px; color: var(--muted); }

/* ---------- provider table ---------- */
.providers { display: flex; flex-direction: column; gap: 8px; }
.provider {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 9px 12px;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  background: var(--surface);
  transition: border-color 150ms ease, background-color 150ms ease, opacity 200ms ease;
}
.provider:hover { border-color: var(--line-strong); }
.provider[data-enabled=false] { background: var(--surface-sunken); opacity: .72; }
.provider-name {
  flex: 0 0 108px;
  font-size: 13.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.provider .url { flex: 1; min-width: 0; padding: 6px 10px; font-size: 12.5px; }
.status {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 9px;
  border-radius: 999px;
  font-size: 11.5px;
  white-space: nowrap;
}
.status::before {
  content: "";
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.status[data-health=unknown] { background: var(--surface-sunken); color: var(--faint); }
.status[data-health=healthy] { background: var(--ok-soft); color: var(--ok); }
.status[data-health=login_required],
.status[data-health=attachment_unsupported],
.status[data-health=composer_missing] { background: var(--warn-soft); color: var(--warn); }
.status[data-health=network_error] { background: var(--danger-soft); color: var(--danger); }
.status.pulse::before { animation: status-pulse 900ms ease-in-out 2; }
@keyframes status-pulse { 50% { opacity: .25; } }

/* ---------- custom provider editor ---------- */
.custom-row {
  display: grid;
  grid-template-columns: 110px 1fr 1fr 1fr 32px;
  gap: 8px;
  align-items: center;
  padding: 7px 0;
  border-top: 1px solid var(--line);
  animation: card-in 260ms var(--ease-out);
}
.custom-row:first-child { border-top: 0; }
.custom-head {
  display: grid;
  grid-template-columns: 110px 1fr 1fr 1fr 32px;
  gap: 8px;
  padding-bottom: 6px;
  font-size: 11.5px;
  color: var(--muted);
}
.icon-button {
  width: 32px;
  height: 32px;
  display: grid;
  place-items: center;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  cursor: default;
  transition: background-color 140ms ease, color 140ms ease, border-color 140ms ease;
}
.icon-button svg { width: 15px; height: 15px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; }
.icon-button:hover { background: var(--danger-soft); border-color: #f0cccc; color: var(--danger); }

/* ---------- buttons ---------- */
button.primary, button.ghost {
  height: 34px;
  padding: 0 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  border-radius: var(--radius-sm);
  font-size: 13px;
  font-weight: 500;
  cursor: default;
  transition: background-color 150ms ease, border-color 150ms ease,
              transform 120ms var(--ease-out), box-shadow 150ms ease;
}
button.primary svg, button.ghost svg { width: 15px; height: 15px; fill: none; stroke: currentColor; stroke-width: 1.9; stroke-linecap: round; stroke-linejoin: round; }
button.primary {
  border: 1px solid var(--accent);
  background: var(--accent);
  color: #fffaf6;
  box-shadow: 0 1px 2px rgba(153, 60, 29, .28);
}
button.primary:hover { background: var(--accent-hover); border-color: var(--accent-hover); box-shadow: 0 3px 10px rgba(153, 60, 29, .26); }
button.primary:active { background: var(--accent-press); transform: scale(.975); }
button.ghost {
  border: 1px solid var(--line-strong);
  background: var(--surface);
  color: var(--text);
}
button.ghost:hover { background: var(--surface-sunken); border-color: var(--faint); }
button.ghost:active { transform: scale(.975); }
button.primary:focus-visible, button.ghost:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
button:disabled {
  opacity: .5;
  cursor: default;
  transform: none;
  box-shadow: none;
}
.actions { display: flex; align-items: center; gap: 10px; margin-top: 14px; }

/* ---------- footer ---------- */
footer {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 28px 16px;
  border-top: 1px solid var(--line);
  background: var(--bg);
}
.status-line {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: color 200ms ease;
}
.status-line[data-tone=ok] { color: var(--ok); }
.status-line[data-tone=error] { color: var(--danger); }
.dirty {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--accent);
  opacity: 0;
  transform: translateX(-4px);
  transition: opacity 200ms ease, transform 200ms var(--ease-out);
}
.dirty.visible { opacity: 1; transform: none; }
.dirty::before {
  content: "";
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
}

/* ---------- toast ---------- */
.toast {
  position: fixed;
  left: 50%;
  bottom: 74px;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 10px 16px;
  border-radius: 11px;
  background: var(--text);
  color: #fdfbf8;
  font-size: 13px;
  box-shadow: 0 8px 28px rgba(27, 27, 27, .28);
  opacity: 0;
  transform: translate(-50%, 14px) scale(.97);
  transition: opacity 200ms ease, transform 320ms var(--ease-spring);
  pointer-events: none;
  max-width: 70%;
}
.toast.show { opacity: 1; transform: translate(-50%, 0) scale(1); }
.toast[data-tone=error] { background: var(--danger); }
.toast svg { width: 16px; height: 16px; flex: 0 0 16px; fill: none; stroke: currentColor; stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round; }
.toast .toast-check { stroke-dasharray: 24; stroke-dashoffset: 24; }
.toast.show .toast-check { animation: draw-check 380ms 90ms var(--ease-out) forwards; }
@keyframes draw-check { to { stroke-dashoffset: 0; } }

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 1ms !important;
    animation-delay: 0ms !important;
    transition-duration: 1ms !important;
  }
}
</style>
</head>
<body>
<header>
  <div class="mark" aria-hidden="true">
    <!-- The official AskBridge mark, inlined from
         assets/branding/askbridge-final/source/askbridge-mark-transparent.svg:
         four corner brackets around the terracotta bridge. Do not redraw it. -->
    <svg viewBox="0 0 1024 1024">
      <g fill="none" stroke="currentColor" stroke-width="68" stroke-linecap="square" stroke-linejoin="miter">
        <path d="M 100 320 L 100 100 L 320 100"></path>
        <path d="M 924 320 L 924 100 L 704 100"></path>
        <path d="M 100 704 L 100 924 L 320 924"></path>
        <path d="M 924 704 L 924 924 L 704 924"></path>
      </g>
      <path class="mark-bridge" d="M 292 760 L 292 480 A 220 220 0 0 1 732 480 L 732 760 L 660 760 L 660 540 A 148 148 0 0 0 364 540 L 364 760 Z"></path>
    </svg>
  </div>
  <div class="title-block">
    <h1>AskBridge</h1>
    <p class="subtitle">截图问答 · 本机设置</p>
  </div>
  <span class="version" id="version"></span>
</header>

<div class="shell">
  <nav id="nav" aria-label="设置分区">
    <div class="pill" id="pill"></div>
  </nav>
  <main id="main">
    <section class="page" data-page="hotkeys">
      <div class="card">
        <h2>全局快捷键</h2>
        <p class="hint">点击输入框后按下组合键即可录制；也可直接输入，例如 <span class="mono">Alt+Shift+Q</span>。按 Esc 取消录制。</p>
        <div id="hotkeys"></div>
      </div>
      <div class="card">
        <h2>快捷键说明</h2>
        <p class="hint">修改后立即生效；注册或保存失败时会保留原绑定。快捷键必须至少包含一个修饰键。</p>
        <div class="actions">
          <button class="ghost" id="restore-hotkeys">
            <svg viewBox="0 0 24 24"><path d="M4 10a8 8 0 1 1 2.3 6.3"></path><path d="M4 4v6h6"></path></svg>
            恢复默认快捷键
          </button>
        </div>
      </div>
    </section>

    <section class="page" data-page="providers">
      <div class="card">
        <h2>默认供应商</h2>
        <p class="hint">截图投递和文字提问默认使用这个供应商。</p>
        <div class="row">
          <div class="row-text"><div class="row-label">默认供应商</div></div>
          <div class="row-control">
            <div class="segmented" id="default-provider" role="group" aria-label="默认供应商"></div>
            <button class="ghost" id="check-providers">
              <svg viewBox="0 0 24 24"><path d="M4 12h4l2.5 6 3-12 2.5 6h4"></path></svg>
              检测连接
            </button>
          </div>
        </div>
      </div>
      <div class="card">
        <h2>内置供应商</h2>
        <p class="hint">入口只匹配同一 HTTPS 域名；修改起始网址后会按新域名重新推导匹配前缀。</p>
        <div class="providers" id="providers"></div>
      </div>
      <div class="card">
        <h2>自定义供应商</h2>
        <p class="hint">每行一个供应商。匹配前缀可填多个，用逗号分隔。</p>
        <div class="custom-head"><span>ID</span><span>名称</span><span>起始网址</span><span>匹配前缀</span><span></span></div>
        <div id="custom-providers"></div>
        <div class="actions">
          <button class="ghost" id="add-custom">
            <svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"></path></svg>
            添加一行
          </button>
        </div>
      </div>
    </section>

    <section class="page" data-page="browser">
      <div class="card">
        <h2>ChatGPT 打开方式</h2>
        <p class="hint">自动降级只用于截图；纯文字仍只走专用 Chrome。</p>
        <div id="targets"></div>
      </div>
      <div class="card">
        <h2>专用 Chrome</h2>
        <div class="row">
          <div class="row-text">
            <div class="row-label">Chrome 可执行文件</div>
            <div class="row-desc">留空自动检测；填写时必须是现有 chrome.exe 的绝对路径。</div>
          </div>
        </div>
        <input type="text" id="chrome-path" class="mono" spellcheck="false" placeholder="留空自动检测">
        <div class="row">
          <div class="row-text"><div class="row-label">生命周期</div></div>
          <div class="row-control">
            <div class="segmented" id="lifecycle" role="group" aria-label="浏览器生命周期"></div>
          </div>
        </div>
        <div class="actions">
          <button class="primary" id="open-browser">
            <svg viewBox="0 0 24 24"><path d="M14 4h6v6"></path><path d="M20 4 10 14"></path><path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4"></path></svg>
            打开 AskBridge 浏览器
          </button>
          <button class="ghost" id="check-browser">检查连接</button>
          <button class="ghost" id="open-login">打开默认供应商登录页面</button>
        </div>
        <p class="hint" style="margin-top:14px">浏览器工具只控制 AskBridge 专用配置，不连接日常 Chrome。</p>
      </div>
      <div class="card">
        <h2>数据目录</h2>
        <p class="hint">配置、日志与浏览器配置都保存在这个目录下。</p>
        <input type="text" id="data-root" class="mono" readonly>
      </div>
    </section>

    <section class="page" data-page="general">
      <div class="card">
        <h2>截图快速投递</h2>
        <p class="hint">使用「截图快速投递」快捷键时自动填入这段提示词。</p>
        <textarea id="quick-prompt" spellcheck="false" placeholder="请输入提示词"></textarea>
      </div>
      <div class="card">
        <h2>启动与日志</h2>
        <div class="row">
          <div class="row-text">
            <div class="row-label">登录 Windows 后启动 AskBridge</div>
            <div class="row-desc">当前用户，不需管理员权限</div>
          </div>
          <div class="row-control">
            <button class="switch" id="start-on-login" role="switch" aria-checked="false" aria-label="登录 Windows 后启动 AskBridge"></button>
          </div>
        </div>
        <div class="row">
          <div class="row-text">
            <div class="row-label">启用调试日志</div>
            <div class="row-desc">立即生效；日志仍不记录问题、截图或网页正文</div>
          </div>
          <div class="row-control">
            <button class="switch" id="debug-logging" role="switch" aria-checked="false" aria-label="启用调试日志"></button>
          </div>
        </div>
      </div>
      <div class="card">
        <h2>发送边界</h2>
        <p class="hint">AskBridge 没有自动发送开关；所有请求始终由你在网页中确认发送。截图、问题原文与网页内容都不会被记录。</p>
      </div>
    </section>
  </main>
</div>

<footer>
  <div class="status-line" id="status" data-tone=""></div>
  <span class="dirty" id="dirty">未保存的更改</span>
  <button class="ghost" id="close">关闭</button>
  <button class="primary" id="apply">
    <svg viewBox="0 0 24 24"><path d="m5 12 4.5 4.5L19 7"></path></svg>
    应用更改
  </button>
</footer>

<div class="toast" id="toast" role="status" aria-live="polite">
  <svg viewBox="0 0 24 24" id="toast-icon"><path class="toast-check" d="m5 12 4.5 4.5L19 7"></path></svg>
  <span id="toast-text"></span>
</div>

<script>
"use strict";
const model = /*__VIEW_MODEL__*/null;

const $ = id => document.getElementById(id);
const el = (tag, className) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
};

let currentPage = null;
let saved = null;
let draft = null;

/* ---------- model <-> draft ---------- */
function cloneModel(value) {
  return JSON.parse(JSON.stringify(value));
}

function draftFromModel(value) {
  return {
    hotkeys: {
      capture_with_prompt: { enabled: value.hotkeys[0].enabled, text: value.hotkeys[0].text },
      capture_quick_dispatch: { enabled: value.hotkeys[1].enabled, text: value.hotkeys[1].text },
      text_only_prompt: { enabled: value.hotkeys[2].enabled, text: value.hotkeys[2].text }
    },
    default_provider_id: value.default_provider_id,
    quick_prompt: value.quick_prompt,
    start_on_login: value.start_on_login,
    debug_logging: value.debug_logging,
    chrome_path: value.chrome_path,
    lifecycle: value.lifecycle,
    chatgpt_target: value.chatgpt_target,
    providers: value.providers.map(row => ({
      id: row.id,
      enabled: row.enabled,
      start_url: row.start_url
    })),
    custom_providers: value.custom_providers.map(row => ({
      id: row.id,
      display_name: row.display_name,
      start_url: row.start_url,
      url_patterns: row.url_patterns
    }))
  };
}

const HOTKEY_IDS = ["capture_with_prompt", "capture_quick_dispatch", "text_only_prompt"];

function markDirty() {
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  $("dirty").classList.toggle("visible", dirty);
  return dirty;
}

/* ---------- navigation ---------- */
const NAV_ICONS = {
  hotkeys: '<path d="M4 7h16M4 12h10M4 17h13"></path>',
  providers: '<path d="M4 7h16v10H4z"></path><path d="M8 11h4M8 14h7"></path>',
  browser: '<circle cx="12" cy="12" r="8"></circle><path d="M4 12h16M12 4c2.5 2.4 2.5 13.6 0 16"></path>',
  general: '<circle cx="12" cy="12" r="3"></circle><path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1"></path>'
};

function buildNav() {
  const nav = $("nav");
  // The page hydrates again whenever the runtime pushes a refresh, so the
  // previous entries and the indicator must be cleared first.
  nav.querySelectorAll(".nav-item").forEach(node => node.remove());
  model.nav.forEach((item, index) => {
    const button = el("button", "nav-item");
    button.type = "button";
    button.dataset.page = item.id;
    button.setAttribute("aria-label", item.label);
    button.innerHTML = `<svg viewBox="0 0 24 24">${NAV_ICONS[item.id] || ""}</svg><span></span>`;
    button.querySelector("span").textContent = item.label;
    button.addEventListener("click", () => showPage(item.id));
    nav.appendChild(button);
    if (index === 0 && !currentPage) requestAnimationFrame(() => movePill(button));
  });
  const active = nav.querySelector(".nav-item.active") || nav.querySelector(".nav-item");
  if (active) requestAnimationFrame(() => movePill(active));
}

function movePill(button) {
  const pill = $("pill");
  if (!button) return;
  // The indicator is a child of `nav`, so `offsetTop` already includes the
  // nav's own padding.
  pill.style.transform = `translateY(${button.offsetTop}px)`;
  pill.style.height = `${button.offsetHeight}px`;
}

function showPage(id) {
  if (currentPage === id) return;
  currentPage = id;
  document.querySelectorAll(".nav-item").forEach(button => {
    const active = button.dataset.page === id;
    button.classList.toggle("active", active);
    if (active) movePill(button);
  });
  document.querySelectorAll(".page").forEach(page => {
    page.classList.toggle("active", page.dataset.page === id);
  });
  $("main").scrollTop = 0;
}

/* ---------- hotkeys ---------- */
function buildHotkeys() {
  const host = $("hotkeys");
  host.replaceChildren();
  model.hotkeys.forEach((row, index) => {
    const wrap = el("div", "row");
    const text = el("div", "row-text");
    const label = el("div", "row-label");
    label.textContent = row.label;
    const desc = el("div", "row-desc");
    desc.textContent = row.description;
    text.append(label, desc);

    const control = el("div", "row-control");
    const input = el("input", "hotkey mono");
    input.type = "text";
    input.id = `hotkey-${row.id}`;
    input.setAttribute("aria-label", row.label);
    input.spellcheck = false;
    input.value = row.text;
    input.addEventListener("input", () => {
      draft.hotkeys[row.id].text = input.value;
      input.classList.remove("invalid");
      markDirty();
    });
    input.addEventListener("keydown", event => {
      if (event.key === "Escape") {
        input.blur();
        return;
      }
      const recorded = recordHotkey(event);
      if (!recorded) return;
      event.preventDefault();
      input.value = recorded;
      input.classList.remove("invalid");
      draft.hotkeys[row.id].text = recorded;
      markDirty();
    });
    input.addEventListener("focus", () => input.classList.add("recording"));
    input.addEventListener("blur", () => input.classList.remove("recording"));

    const toggle = el("button", "switch");
    toggle.type = "button";
    toggle.id = `enable-${row.id}`;
    toggle.setAttribute("role", "switch");
    toggle.setAttribute("aria-label", row.label);
    toggle.setAttribute("aria-checked", String(row.enabled));
    toggle.addEventListener("click", () => {
      const next = toggle.getAttribute("aria-checked") !== "true";
      toggle.setAttribute("aria-checked", String(next));
      draft.hotkeys[row.id].enabled = next;
      markDirty();
    });

    control.append(input, toggle);
    wrap.append(text, control);
    host.appendChild(wrap);
  });
}

const MODIFIER_KEYS = {
  Control: "Ctrl",
  Alt: "Alt",
  Shift: "Shift",
  Meta: "Win"
};

function recordHotkey(event) {
  if (MODIFIER_KEYS[event.key]) return "";
  const modifiers = [];
  if (event.ctrlKey) modifiers.push("Ctrl");
  if (event.altKey) modifiers.push("Alt");
  if (event.shiftKey) modifiers.push("Shift");
  if (event.metaKey) modifiers.push("Win");
  if (!modifiers.length) return "";
  let key = event.key;
  if (key.length === 1) key = key.toUpperCase();
  else if (key === "Delete") key = "Delete";
  else if (/^F\d{1,2}$/.test(key)) key = key;
  else return "";
  return [...modifiers, key].join("+");
}

/* ---------- providers ---------- */
function buildProviders() {
  const host = $("providers");
  host.replaceChildren();
  model.providers.forEach(row => {
    const wrap = el("div", "provider");
    wrap.dataset.enabled = String(row.enabled);
    wrap.dataset.provider = row.id;

    const name = el("div", "provider-name");
    name.textContent = row.display_name;

    const url = el("input", "url mono");
    url.type = "text";
    url.spellcheck = false;
    url.id = `url-${row.id}`;
    url.value = row.start_url;
    url.setAttribute("aria-label", `${row.display_name} 起始网址`);
    url.addEventListener("input", () => {
      const target = draft.providers.find(item => item.id === row.id);
      if (target) target.start_url = url.value;
      url.classList.remove("invalid");
      markDirty();
    });

    const status = el("span", "status");
    status.dataset.health = row.health;
    status.textContent = row.health_label;
    status.id = `health-${row.id}`;

    const toggle = el("button", "switch");
    toggle.type = "button";
    toggle.id = `enable-${row.id}`;
    toggle.setAttribute("role", "switch");
    toggle.setAttribute("aria-label", row.display_name);
    toggle.setAttribute("aria-checked", String(row.enabled));
    toggle.addEventListener("click", () => {
      const next = toggle.getAttribute("aria-checked") !== "true";
      toggle.setAttribute("aria-checked", String(next));
      const target = draft.providers.find(item => item.id === row.id);
      if (target) target.enabled = next;
      wrap.dataset.enabled = String(next);
      markDirty();
    });

    wrap.append(name, url, status, toggle);
    host.appendChild(wrap);
  });
}

function buildDefaultProvider() {
  const host = $("default-provider");
  host.replaceChildren();
  model.providers.forEach(row => {
    const button = el("button");
    button.type = "button";
    button.dataset.provider = row.id;
    button.id = `default-provider-${row.id}`;
    button.textContent = row.display_name;
    button.setAttribute("aria-pressed", String(row.id === draft.default_provider_id));
    button.addEventListener("click", () => {
      draft.default_provider_id = row.id;
      host.querySelectorAll("button").forEach(other => {
        other.setAttribute("aria-pressed", String(other.dataset.provider === row.id));
      });
      markDirty();
    });
    host.appendChild(button);
  });
}

/* ---------- custom providers ---------- */
function buildCustomProviders() {
  const host = $("custom-providers");
  host.replaceChildren();
  draft.custom_providers.forEach((row, index) => {
    const wrap = el("div", "custom-row");

    const fields = [
      { key: "id", placeholder: "example", label: "ID" },
      { key: "display_name", placeholder: "示例", label: "名称" },
      { key: "start_url", placeholder: "https://example.com/chat", label: "起始网址" },
      { key: "url_patterns", placeholder: "https://example.com/", label: "匹配前缀" }
    ];
    fields.forEach(field => {
      const input = el("input", field.key === "display_name" ? "" : "mono");
      input.type = "text";
      input.spellcheck = false;
      input.placeholder = field.placeholder;
      input.id = `custom-${index}-${field.key}`;
      input.value = row[field.key] || "";
      input.setAttribute("aria-label", `自定义供应商 ${index + 1} ${field.label}`);
      input.addEventListener("input", () => {
        draft.custom_providers[index][field.key] = input.value;
        input.classList.remove("invalid");
        markDirty();
      });
      wrap.appendChild(input);
    });

    const remove = el("button", "icon-button");
    remove.type = "button";
    remove.setAttribute("aria-label", `删除自定义供应商 ${index + 1}`);
    remove.innerHTML = '<svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6 6 18"></path></svg>';
    remove.addEventListener("click", () => {
      draft.custom_providers.splice(index, 1);
      buildCustomProviders();
      markDirty();
    });
    wrap.appendChild(remove);
    host.appendChild(wrap);
  });
}

/* ---------- browser ---------- */
function buildTargets() {
  const host = $("targets");
  host.replaceChildren();
  model.targets.forEach(option => {
    const wrap = el("div", "option");
    wrap.dataset.target = option.value;
    wrap.setAttribute("role", "radio");
    wrap.setAttribute("aria-checked", String(option.value === draft.chatgpt_target));
    // No `aria-label` here on purpose: labelling a container hides its
    // descendants from the accessibility tree, which would drop the
    // consequence text that tells the user what each mode actually does.
    wrap.tabIndex = 0;

    const dot = el("div", "dot");
    const body = el("div");
    const label = el("div", "option-label");
    label.textContent = option.label;
    const desc = el("div", "option-desc");
    desc.textContent = option.description;
    body.append(label, desc);
    wrap.append(dot, body);

    const select = () => {
      draft.chatgpt_target = option.value;
      host.querySelectorAll(".option").forEach(other => {
        other.setAttribute("aria-checked", String(other.dataset.target === option.value));
      });
      markDirty();
    };
    wrap.addEventListener("click", select);
    wrap.addEventListener("keydown", event => {
      if (event.key === " " || event.key === "Enter") {
        event.preventDefault();
        select();
      }
    });
    host.appendChild(wrap);
  });
}

function buildLifecycle() {
  const host = $("lifecycle");
  host.replaceChildren();
  model.lifecycles.forEach(option => {
    const button = el("button");
    button.type = "button";
    button.dataset.value = option.value;
    button.textContent = option.label;
    button.setAttribute("aria-pressed", String(option.value === draft.lifecycle));
    button.addEventListener("click", () => {
      draft.lifecycle = option.value;
      host.querySelectorAll("button").forEach(other => {
        other.setAttribute("aria-pressed", String(other.dataset.value === option.value));
      });
      markDirty();
    });
    host.appendChild(button);
  });
}

/* ---------- general ---------- */
function buildGeneral() {
  $("quick-prompt").value = draft.quick_prompt;
  $("start-on-login").setAttribute("aria-checked", String(draft.start_on_login));
  $("debug-logging").setAttribute("aria-checked", String(draft.debug_logging));
  $("chrome-path").value = draft.chrome_path;
  $("data-root").value = model.data_root;
}

/* ---------- status + toast ---------- */
let toastTimer = 0;
function toast(message, tone) {
  const node = $("toast");
  node.dataset.tone = tone || "ok";
  $("toast-text").textContent = message;
  node.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => node.classList.remove("show"), tone === "error" ? 5200 : 2600);
}

function setStatus(message, tone) {
  const node = $("status");
  node.textContent = message;
  node.dataset.tone = tone || "";
}

/* ---------- host bridge ---------- */
function post(message) {
  window.ipc.postMessage(message);
}

$("apply").addEventListener("click", () => {
  document.querySelectorAll(".invalid").forEach(node => node.classList.remove("invalid"));
  post(`apply:${JSON.stringify(draft)}`);
});
$("close").addEventListener("click", () => post("close"));
$("restore-hotkeys").addEventListener("click", () => post("restore_defaults"));
$("check-providers").addEventListener("click", () => post("check_providers"));
$("open-browser").addEventListener("click", () => post("open_browser"));
$("check-browser").addEventListener("click", () => post("open_browser"));
$("open-login").addEventListener("click", () => post("open_login"));
$("add-custom").addEventListener("click", () => {
  draft.custom_providers.push({ id: "", display_name: "", start_url: "", url_patterns: "" });
  buildCustomProviders();
  const rows = $("custom-providers").querySelectorAll(".custom-row");
  const last = rows[rows.length - 1];
  if (last) last.querySelector("input").focus();
  markDirty();
});
$("start-on-login").addEventListener("click", () => {
  draft.start_on_login = !draft.start_on_login;
  $("start-on-login").setAttribute("aria-checked", String(draft.start_on_login));
  markDirty();
});
$("debug-logging").addEventListener("click", () => {
  draft.debug_logging = !draft.debug_logging;
  $("debug-logging").setAttribute("aria-checked", String(draft.debug_logging));
  markDirty();
});
$("quick-prompt").addEventListener("input", () => {
  draft.quick_prompt = $("quick-prompt").value;
  markDirty();
});
$("chrome-path").addEventListener("input", () => {
  draft.chrome_path = $("chrome-path").value;
  markDirty();
});

document.addEventListener("contextmenu", event => event.preventDefault());
document.addEventListener("keydown", event => {
  if (event.key === "Escape" && document.activeElement) document.activeElement.blur();
});

/* ---------- host -> page ---------- */
window.hydrate = function (json) {
  const next = typeof json === "string" ? JSON.parse(json) : json;
  Object.assign(model, next);
  saved = draftFromModel(model);
  draft = cloneModel(saved);
  const previousPage = currentPage;
  $("version").textContent = "v" + model.version;
  buildNav();
  buildHotkeys();
  buildProviders();
  buildDefaultProvider();
  buildCustomProviders();
  buildTargets();
  buildLifecycle();
  buildGeneral();
  setStatus(model.status || "");
  $("dirty").classList.remove("visible");
  // Re-hydration keeps the user on the page they were reading.
  const page = model.nav.some(item => item.id === previousPage)
    ? previousPage
    : model.nav[0].id;
  currentPage = null;
  showPage(page);
};

window.applyResult = function (ok, message) {
  if (ok) {
    saved = cloneModel(draft);
    $("dirty").classList.remove("visible");
    setStatus(message, "ok");
    toast(message, "ok");
  } else {
    setStatus(message, "error");
    toast(message, "error");
  }
};

window.applyStatus = function (message, tone) {
  setStatus(message, tone);
};

window.applyHealth = function (json) {
  const reports = typeof json === "string" ? JSON.parse(json) : json;
  reports.forEach(report => {
    const node = document.getElementById(`health-${report.provider_id}`);
    if (!node) return;
    node.dataset.health = report.health;
    node.textContent = report.health_label;
    node.classList.remove("pulse");
    void node.offsetWidth;
    node.classList.add("pulse");
    const row = model.providers.find(item => item.id === report.provider_id);
    if (row) {
      row.health = report.health;
      row.health_label = report.health_label;
    }
  });
};

window.markInvalid = function (selector) {
  const node = document.querySelector(selector);
  if (node) node.classList.add("invalid");
};

// `hydrate` builds the navigation itself; calling `buildNav` here as well
// would duplicate every entry.
if (model) window.hydrate(model);
// Tell the host the page can accept pushes. Until this arrives the host only
// caches view models, because `evaluate_script` is not safe to call yet.
post("ready");
</script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_v2::view_model::SettingsViewModel;
    use askbridge_core::AppConfig;
    use std::path::Path;

    fn page() -> String {
        let config = AppConfig::default();
        let json = SettingsViewModel::build(&config, Path::new("D:/AskBridge/data"), "", &[])
            .expect("view model")
            .to_json()
            .expect("json");
        settings_html(&json).expect("settings html")
    }

    #[test]
    fn page_embeds_the_view_model_without_escaping_hazards() {
        let html = page();
        assert!(html.contains("\"default_provider_id\""));
        assert!(html.contains("D:/AskBridge/data"));
        assert!(!html.contains(VIEW_MODEL_TOKEN));
    }

    #[test]
    fn page_keeps_the_no_auto_submit_boundary() {
        let html = page();
        let forbidden = ["CHECK_", "AUTO_", "SUBMIT"].concat();
        assert!(!html.contains(&forbidden));
        assert!(html.contains("AskBridge 没有自动发送开关"));
    }

    #[test]
    fn page_exposes_every_browser_target_choice() {
        let html = page();
        for label in [
            "桌面网页端",
            "AskBridge 专用 Chrome",
            "通用粘贴",
            "专用 Chrome 优先",
            "复用现有登录，但截图需要手动上传",
            "支持自动上传图片，需要单独登录",
            "支持浏览器或 AI 桌面端，仅模拟 Ctrl+V（不验证结果）",
            "安全失败后自动用 Ctrl+V 粘贴截图",
        ] {
            assert!(html.contains(label), "missing target copy: {label}");
        }
        let forbidden = ["截图提问会", "自动使用 AskBridge 专用 Chrome"].concat();
        assert!(!html.contains(&forbidden));
    }

    #[test]
    fn page_rejects_a_view_model_that_could_close_its_script_block() {
        let error = settings_html(r#"{"status":"</script>"}"#);
        assert!(matches!(
            error,
            Err(askbridge_core::AppError::ConfigurationInvalid(_))
        ));
        assert!(settings_html(r#"{"status":"<!--"}"#).is_err());
    }

    #[test]
    fn page_never_references_remote_resources() {
        let html = page();
        for scheme in [
            "http://",
            "https://cdn",
            "//cdn.",
            "src=\"http",
            "src='http",
        ] {
            assert!(
                !html.contains(scheme),
                "the settings page must stay offline; found {scheme}"
            );
        }
        assert!(!html.contains("fetch("));
        assert!(!html.contains("XMLHttpRequest"));
    }

    #[test]
    fn page_uses_the_official_mark_instead_of_a_hand_drawn_icon() {
        let html = page();
        // The brand mark is a specific geometry: four corner brackets plus the
        // terracotta bridge path. It must come from the brand kit, not be
        // re-invented, so its defining path data is asserted here.
        assert!(
            html.contains("M 292 760"),
            "the bridge path from the brand kit is missing"
        );
        assert!(
            html.contains("M 100 320 L 100 100 L 320 100"),
            "the corner-bracket path from the brand kit is missing"
        );
        assert!(html.contains(".mark .mark-bridge"));
        // The retired hand-drawn glyph had a rounded chat-bubble outline.
        assert!(
            !html.contains("M4 8V5.5A1.5 1.5 0 0 1 5.5 4H8"),
            "a hand-drawn icon replaced the official mark"
        );
    }

    #[test]
    fn page_reports_readiness_so_the_host_can_push_updates() {
        let html = page();
        assert!(
            html.contains("post(\"ready\")"),
            "the page must announce readiness or host pushes are dropped"
        );
    }

    #[test]
    fn page_animation_respects_reduced_motion() {
        let html = page();
        assert!(html.contains("prefers-reduced-motion: reduce"));
    }
}
