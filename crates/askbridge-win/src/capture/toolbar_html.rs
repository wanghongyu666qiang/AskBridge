use askbridge_core::{AppError, Result};
use serde::Serialize;

use super::toolbar_webview::ToolbarProvider;

#[derive(Serialize)]
struct ProviderPayload<'a> {
    name: &'a str,
    selected: bool,
}

pub(super) fn toolbar_html(
    providers: &[ToolbarProvider],
    menu_height: i32,
    menu_above: bool,
) -> Result<String> {
    let payload = providers
        .iter()
        .map(|provider| ProviderPayload {
            name: &provider.display_name,
            selected: provider.selected,
        })
        .collect::<Vec<_>>();
    let providers_json = serde_json::to_string(&payload)
        .map_err(|error| AppError::CaptureFailed(format!("toolbar payload failed: {error}")))?;
    let bar_top = if menu_above { menu_height + 7 } else { 7 };
    let menu_placement_rule = if menu_above {
        "top: auto; bottom: 44px;"
    } else {
        "top: 44px; bottom: auto;"
    };
    Ok(format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<style>
* {{ box-sizing: border-box; }}
html, body {{
  width: 100%;
  height: 100%;
  margin: 0;
  overflow: hidden;
  background: transparent;
  font-family: "Segoe UI Variable Text", "Microsoft YaHei UI", "Segoe UI", sans-serif;
  color: #f5f5f7;
  user-select: none;
}}
body {{ position: relative; }}
.bar {{
  position: absolute;
  left: 7px;
  right: 7px;
  top: {bar_top}px;
  height: 50px;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 5px 7px;
  border: 1px solid rgba(255, 255, 255, .16);
  border-radius: 18px;
  background: #1c1c1f;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, .08), 0 8px 24px rgba(0, 0, 0, .34), 0 2px 6px rgba(0, 0, 0, .24);
}}
button {{
  height: 40px;
  border: 0;
  border-radius: 11px;
  background: transparent;
  color: #f5f5f7;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 0 13px;
  font: 400 14px "Segoe UI Variable Text", "Microsoft YaHei UI", "Segoe UI", sans-serif;
  letter-spacing: 0;
  white-space: nowrap;
  cursor: default;
}}
button:hover {{ background: rgba(255, 255, 255, .09); }}
button:active {{ background: rgba(255, 255, 255, .14); transform: translateY(1px); }}
button:focus-visible {{ outline: 2px solid rgba(226, 109, 69, .92); outline-offset: -2px; }}
.copy {{ width: 88px; }}
.cancel {{ width: 84px; }}
.divider {{ width: 1px; height: 26px; margin: 0 4px; background: rgba(255, 255, 255, .14); }}
.provider-picker {{ position: relative; width: 188px; height: 40px; flex: 0 0 188px; }}
.provider-toggle {{
  width: 100%;
  justify-content: flex-start;
  padding-left: 13px;
  padding-right: 12px;
  border: 1px solid rgba(255, 255, 255, .11);
  background: rgba(255, 255, 255, .055);
}}
.provider-toggle:hover {{ background: rgba(255, 255, 255, .10); }}
.provider-label {{ flex: 1; overflow: hidden; text-align: left; text-overflow: ellipsis; }}
.ask {{
  width: 112px;
  margin-left: 2px;
  background: #993c1d;
  color: #fffaf6;
  font-weight: 600;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, .16), 0 2px 7px rgba(0, 0, 0, .22);
}}
.ask:hover {{ background: #ad4928; }}
.ask:active {{ background: #8c3419; }}
.chevron {{ width: 16px; height: 16px; transition: transform 120ms ease; }}
body.menu-open .chevron {{ transform: rotate(180deg); }}
.icon {{ width: 19px; height: 19px; flex: 0 0 19px; fill: none; stroke: currentColor; stroke-width: 1.75; stroke-linecap: round; stroke-linejoin: round; }}
.provider-menu {{
  position: absolute;
  left: 0;
  {menu_placement_rule}
  width: 188px;
  max-height: {menu_height}px;
  display: none;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 4px;
  border: 1px solid rgba(255, 255, 255, .15);
  border-radius: 12px;
  background: #1c1c1f;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, .07), 0 10px 28px rgba(0, 0, 0, .38), 0 2px 6px rgba(0, 0, 0, .22);
  color-scheme: dark;
  scrollbar-color: rgba(255, 255, 255, .24) transparent;
  scrollbar-width: thin;
  scrollbar-gutter: stable;
}}
.provider-menu::-webkit-scrollbar {{ width: 8px; }}
.provider-menu::-webkit-scrollbar-thumb {{ border: 2px solid transparent; border-radius: 999px; background: rgba(255, 255, 255, .22); background-clip: padding-box; }}
body.menu-open .provider-menu {{ display: block; }}
.provider-option {{ width: 100%; height: 32px; justify-content: flex-start; padding: 0 9px; gap: 8px; border-radius: 8px; font-size: 13.5px; }}
.provider-option:hover {{ background: rgba(255, 255, 255, .075); }}
.provider-option.selected {{ background: rgba(255, 255, 255, .105); font-weight: 500; }}
.provider-option.selected:hover {{ background: rgba(255, 255, 255, .13); }}
.provider-check {{ width: 16px; height: 16px; margin-left: auto; opacity: 0; }}
.provider-option.selected .provider-check {{ opacity: 1; color: #e77a54; }}
.provider-option span {{ overflow: hidden; text-overflow: ellipsis; }}
</style>
</head>
<body>
<div class="bar">
  <button class="copy" id="copy" title="复制截图">
    <svg class="icon" viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="12" rx="2"></rect><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"></path></svg>
    <span>复制</span>
  </button>
  <button class="cancel" id="cancel" title="取消截图 (Esc)">
    <svg class="icon" viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"></path></svg>
    <span>取消</span>
  </button>
  <div class="divider"></div>
  <div class="provider-picker">
    <button class="provider-toggle" id="provider-toggle" title="切换模型" aria-label="切换模型" aria-haspopup="menu" aria-controls="provider-menu" aria-expanded="false">
      <svg class="icon" viewBox="0 0 24 24"><path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"></path><path d="M9 10h6M9 14h4"></path></svg>
      <span class="provider-label" id="provider-label"></span>
      <svg class="chevron" viewBox="0 0 24 24"><path d="m7 10 5 5 5-5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"></path></svg>
    </button>
    <div class="provider-menu" id="provider-menu" role="menu"></div>
  </div>
  <button class="ask" id="ask" title="用当前模型提问 (Enter)">
    <svg class="icon" viewBox="0 0 24 24"><path d="M12 3v4M12 17v4M3 12h4M17 12h4"></path><path d="m7.5 7.5 2 2M14.5 14.5l2 2M16.5 7.5l-2 2M9.5 14.5l-2 2"></path></svg>
    <span>问问</span>
  </button>
</div>
<script>
const providers = {providers_json};
let selectedIndex = Math.max(0, providers.findIndex(provider => provider.selected));
let menuOpen = false;
let menuRequestedOpen = false;
const providerLabel = document.getElementById('provider-label');
const menu = document.getElementById('provider-menu');
const toggle = document.getElementById('provider-toggle');
function selectedName() {{
  const provider = providers[selectedIndex] || providers[0];
  return provider ? provider.name : "";
}}
function updateProvider() {{
  const name = selectedName();
  providerLabel.textContent = name || "选择模型";
}}
function renderMenu() {{
  menu.replaceChildren();
  providers.forEach((provider, index) => {{
    const option = document.createElement('button');
    option.className = `provider-option${{index === selectedIndex ? ' selected' : ''}}`;
    option.type = 'button';
    option.role = 'menuitemradio';
    option.setAttribute('aria-checked', String(index === selectedIndex));
    option.innerHTML = '<span></span><svg class="provider-check icon" viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"></path></svg>';
    option.querySelector('span').textContent = provider.name;
    option.addEventListener('click', () => {{
      selectedIndex = index;
      updateProvider();
      renderMenu();
      requestMenuOpen(false);
    }});
    menu.appendChild(option);
  }});
}}
function requestMenuOpen(open) {{
  if (menuRequestedOpen === open) return;
  menuRequestedOpen = open;
  if (open) {{
    window.ipc.postMessage('menu:open');
  }} else {{
    applyMenuOpen(false);
    requestAnimationFrame(() => window.ipc.postMessage('menu:close'));
  }}
}}
function applyMenuOpen(open) {{
  menuOpen = open;
  menuRequestedOpen = open;
  document.body.classList.toggle('menu-open', open);
  toggle.setAttribute('aria-expanded', String(open));
  if (open) {{
    requestAnimationFrame(() => menu.querySelector('.selected')?.scrollIntoView({{ block: 'nearest' }}));
  }}
}}
window.applyMenuOpen = applyMenuOpen;
document.getElementById('copy').addEventListener('click', () => window.ipc.postMessage('copy'));
document.getElementById('cancel').addEventListener('click', () => window.ipc.postMessage('cancel'));
document.getElementById('ask').addEventListener('click', () => window.ipc.postMessage(`ask:${{selectedIndex}}`));
toggle.addEventListener('click', event => {{
  event.stopPropagation();
  requestMenuOpen(!menuRequestedOpen);
}});
document.addEventListener('click', event => {{
  if (menuOpen && !menu.contains(event.target)) requestMenuOpen(false);
}});
document.addEventListener('keydown', event => {{
  if (event.key === 'Escape') {{
    event.preventDefault();
    if (menuOpen) {{
      requestMenuOpen(false);
    }} else {{
      window.ipc.postMessage('cancel');
    }}
  }}
  if (event.key === 'Enter') {{
    event.preventDefault();
    window.ipc.postMessage(`ask:${{selectedIndex}}`);
  }}
}});
document.addEventListener('contextmenu', event => event.preventDefault());
updateProvider();
renderMenu();
</script>
</body>
</html>"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_html_contains_selected_provider() {
        let html = toolbar_html(
            &[
                ToolbarProvider {
                    id: "chatgpt".to_owned(),
                    display_name: "ChatGPT".to_owned(),
                    selected: false,
                },
                ToolbarProvider {
                    id: "doubao".to_owned(),
                    display_name: "豆包".to_owned(),
                    selected: true,
                },
            ],
            88,
            true,
        )
        .expect("toolbar html");

        assert!(html.contains("ChatGPT"));
        assert!(html.contains("豆包"));
        assert!(html.contains("window.ipc.postMessage"));
        assert!(html.contains("provider-menu"));
        assert!(html.contains("取消截图 (Esc)"));
        assert!(html.contains("用当前模型提问 (Enter)"));
        assert!(html.contains("event.key === 'Escape'"));
        assert!(html.contains("event.key === 'Enter'"));
        assert!(html.contains("if (menuOpen)"));
        assert!(!html.contains("event.key === 'Enter' && !menuOpen"));
        assert!(!html.contains("<select"));
        assert!(html.contains("background: #1c1c1f"));
        assert!(html.contains("background: #993c1d"));
        assert!(html.contains("class=\"provider-picker\""));
        assert!(html.contains("left: 0;"));
        assert!(!html.contains("right: 140px"));
        assert!(html.contains("id=\"provider-label\""));
        assert!(!html.contains("id=\"ask-label\""));
    }

    #[test]
    fn provider_menu_waits_for_native_clip_before_visual_open() {
        let html = toolbar_html(
            &[ToolbarProvider {
                id: "chatgpt".to_owned(),
                display_name: "ChatGPT".to_owned(),
                selected: true,
            }],
            170,
            true,
        )
        .expect("toolbar html");

        let resize_request = html
            .find("window.ipc.postMessage('menu:open')")
            .expect("native resize request");
        let visual_change = html
            .find("document.body.classList.toggle('menu-open', open)")
            .expect("visual menu change");
        assert!(
            resize_request < visual_change,
            "the host resize request must precede the visual open"
        );
        assert!(html.contains("window.applyMenuOpen"));
    }

    #[test]
    fn toolbar_surfaces_do_not_leak_background_content() {
        let html = toolbar_html(&[], 170, false).expect("toolbar html");

        assert!(html.matches("background: #1c1c1f;").count() >= 2);
        assert!(!html.contains("background: rgba(28, 28, 31, .975)"));
        assert!(!html.contains("background: rgba(28, 28, 31, .985)"));
        assert!(!html.contains("backdrop-filter"));
    }

    #[test]
    fn upward_provider_menu_keeps_closed_toolbar_inside_its_clip() {
        let html = toolbar_html(
            &[ToolbarProvider {
                id: "chatgpt".to_owned(),
                display_name: "ChatGPT".to_owned(),
                selected: true,
            }],
            170,
            true,
        )
        .expect("toolbar html");
        let bar_rule = html
            .split_once(".bar {")
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(rule, _)| rule)
            .expect("bar rule");

        assert!(
            bar_rule.contains("top: 177px;"),
            "the closed toolbar must stay inside the fixed host's visible clip"
        );
        assert!(!html.contains("body.menu-open .bar"));
    }
}
