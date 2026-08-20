use askbridge_core::{AppError, Result};

pub(super) fn login_detection_expression(selectors: &[String]) -> Result<String> {
    let selectors = serde_json::to_string(selectors).map_err(|_| {
        AppError::InvalidPreparation("login selectors could not be encoded".to_owned())
    })?;
    Ok(format!(
        r#"(() => {{
  const selectors = {selectors};
  const visible = (element) => {{
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return rect.width > 0 && rect.height > 0 && style.display !== 'none' &&
      style.visibility !== 'hidden' && Number(style.opacity || '1') > 0;
  }};
  const matches = [...new Set(selectors.flatMap(
    (selector) => [...document.querySelectorAll(selector)]
  ))].filter(visible);
  return {{ loginDetected: matches.length > 0, url: location.href }};
}})()"#
    ))
}

pub(super) fn composer_insertion_expression(
    prompt: &str,
    preferred_selectors: &[String],
    login_selectors: &[String],
    expected_url: &str,
) -> Result<String> {
    let prompt = serde_json::to_string(prompt).map_err(|_| {
        AppError::InvalidPreparation("prompt could not be encoded for browser input".to_owned())
    })?;
    let preferred_selectors = serde_json::to_string(preferred_selectors).map_err(|_| {
        AppError::InvalidPreparation("provider selectors could not be encoded".to_owned())
    })?;
    let login_selectors = serde_json::to_string(login_selectors).map_err(|_| {
        AppError::InvalidPreparation("login selectors could not be encoded".to_owned())
    })?;
    let expected_url = serde_json::to_string(expected_url).map_err(|_| {
        AppError::InvalidPreparation("expected target URL could not be encoded".to_owned())
    })?;
    Ok(format!(
        r#"(() => {{
  const prompt = {prompt};
  const preferredSelectors = {preferred_selectors};
  const loginSelectors = {login_selectors};
  const expectedUrl = {expected_url};
  if (location.href !== expectedUrl) {{
    return {{ status: 'navigation_changed', url: location.href }};
  }}
  const visible = (el) => {{
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width >= 80 && rect.height >= 20 && style.display !== 'none' &&
      style.visibility !== 'hidden' && Number(style.opacity || '1') > 0;
  }};
  const editable = (el) => !el.disabled && !el.readOnly &&
    (el.matches('textarea,input') || el.isContentEditable || el.getAttribute('role') === 'textbox');
  const visibleLoginElements = [...new Set(loginSelectors.flatMap(
    (selector) => [...document.querySelectorAll(selector)]
  ))].filter(visible);
  if (visibleLoginElements.length > 0) {{
    return {{ status: 'login_detected', url: location.href }};
  }}
  const preferredElements = [...new Set(preferredSelectors.flatMap(
    (selector) => [...document.querySelectorAll(selector)]
  ))].filter((el) => visible(el) && editable(el));
  const source = preferredElements.length ? preferredElements : [...new Set(
    document.querySelectorAll('textarea,[contenteditable="true"],[role="textbox"]')
  )];
  const candidates = source.filter((el) => visible(el) && editable(el)).map((el) => {{
    const rect = el.getBoundingClientRect();
    const label = [el.getAttribute('aria-label'), el.getAttribute('placeholder'),
      el.getAttribute('name'), el.id, el.className].filter(Boolean).join(' ').toLowerCase();
    let score = el.matches('textarea') ? 45 : (el.isContentEditable ? 40 : 35);
    if (preferredElements.includes(el)) score += 100;
    if (/message|prompt|ask|chat|send|提问|消息|输入/.test(label)) score += 30;
    if (/search|feedback|account|login|搜索|反馈|账号|登录/.test(label)) score -= 80;
    if (rect.top > innerHeight * 0.45) score += 20;
    if (rect.width > Math.min(360, innerWidth * 0.35)) score += 15;
    if (el.getAttribute('aria-multiline') === 'true') score += 10;
    return {{ el, score }};
  }}).sort((a, b) => b.score - a.score);
  if (!candidates.length || candidates[0].score < 60) {{
    return {{ status: 'missing', url: location.href,
      providerRuleMiss: preferredSelectors.length > 0 && preferredElements.length === 0 }};
  }}
  if (candidates.length > 1 && candidates[1].score >= candidates[0].score - 10) {{
    return {{ status: 'ambiguous', url: location.href }};
  }}
  const el = candidates[0].el;
  el.focus();
  if (!prompt.trim()) {{
    return {{ status: 'focused', url: location.href }};
  }}
  if (el.matches('textarea,input')) {{
    const prototype = el.matches('textarea') ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(prototype, 'value').set;
    setter.call(el, prompt);
    el.dispatchEvent(new InputEvent('input', {{ bubbles: true, inputType: 'insertText', data: prompt }}));
  }} else {{
    const selection = getSelection();
    const range = document.createRange();
    range.selectNodeContents(el);
    selection.removeAllRanges();
    selection.addRange(range);
    const inserted = document.execCommand('insertText', false, prompt);
    if (!inserted) {{
      el.textContent = prompt;
      el.dispatchEvent(new InputEvent('input', {{ bubbles: true, inputType: 'insertText', data: prompt }}));
    }}
  }}
  const actual = el.matches('textarea,input') ? el.value : (el.innerText || el.textContent || '');
  return {{ status: actual === prompt ? 'inserted' : 'verification_failed', url: location.href }};
}})()"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_program_never_contains_submit_actions() {
        let program = composer_insertion_expression(
            "question",
            &["#prompt-textarea".to_owned()],
            &[],
            "https://chatgpt.com/",
        )
        .expect("program");
        assert!(!program.contains(".click("));
        assert!(!program.contains("requestSubmit"));
        assert!(!program.contains("form.submit"));
    }
}
