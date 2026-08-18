use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use askbridge_core::{
    AppError, DispatchRequest, PreparationFailureStage, PreparationOutcome, PreparationPolicy,
    RecoveryHint, Result, matches_any_pattern,
};
use serde_json::Value;

use crate::{
    browser::{CdpClient, CdpTarget, FileInputResult},
    capture::encoder::encode_png,
};

mod rules;

use rules::{ProviderRule, load_rule};

pub(crate) fn validate_builtin_rules() -> Result<()> {
    rules::validate_builtin_rules()
}

pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn matches_url(&self, url: &str) -> bool;
    fn prepare(
        &self,
        page: &mut PageSession<'_>,
        request: &DispatchRequest,
        policy: &PreparationPolicy,
    ) -> Result<PreparationOutcome>;
}

pub enum PageSession<'a> {
    DedicatedChrome {
        client: &'a CdpClient,
        target: &'a CdpTarget,
        temp_root: &'a Path,
        cancelled: &'a AtomicBool,
    },
    DesktopPwa {
        target_url: &'a str,
    },
}

pub struct GenericProviderAdapter {
    provider_id: String,
    url_patterns: Vec<String>,
    rule: Option<ProviderRule>,
}

impl GenericProviderAdapter {
    pub fn for_provider(
        provider_id: impl Into<String>,
        adapter_override: Option<&str>,
        url_patterns: Vec<String>,
    ) -> Result<Self> {
        let provider_id = provider_id.into();
        let rule = load_rule(adapter_override)?;
        Ok(Self {
            provider_id,
            url_patterns,
            rule,
        })
    }

    fn prepare_dedicated_chrome(
        &self,
        client: &CdpClient,
        target: &CdpTarget,
        temp_root: &Path,
        cancelled: &AtomicBool,
        request: &DispatchRequest,
        policy: &PreparationPolicy,
    ) -> Result<PreparationOutcome> {
        let timeout = Duration::from_millis(policy.timeout_ms);
        let current = current_target(client, &target.id)?;
        if self
            .rule
            .as_ref()
            .is_some_and(|rule| rule.matches_login_url(&current.url))
        {
            return Ok(manual_fallback(
                &current.url,
                PreparationFailureStage::PageReadiness,
                RecoveryHint::LoginInBrowser,
                false,
                false,
            ));
        }
        if !self.matches_url(&current.url) {
            return Ok(manual_fallback(
                &current.url,
                PreparationFailureStage::NavigationChanged,
                RecoveryHint::ReopenProviderPage,
                false,
                false,
            ));
        }

        if let Some(rule) = self
            .rule
            .as_ref()
            .filter(|rule| !rule.login_selectors().is_empty())
        {
            let expression = login_detection_expression(rule.login_selectors())?;
            let result = client.evaluate_in_target(&current, &expression, cancelled, timeout)?;
            let value = result.pointer("/result/value").ok_or_else(|| {
                AppError::BrowserProtocol("login detection returned no value".to_owned())
            })?;
            let target_url = value
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or(&current.url);
            if rule.matches_login_url(target_url)
                || value
                    .get("loginDetected")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            {
                return Ok(manual_fallback(
                    target_url,
                    PreparationFailureStage::PageReadiness,
                    RecoveryHint::LoginInBrowser,
                    false,
                    false,
                ));
            }
            if !self.matches_url(target_url) {
                return Ok(manual_fallback(
                    target_url,
                    PreparationFailureStage::NavigationChanged,
                    RecoveryHint::ReopenProviderPage,
                    false,
                    false,
                ));
            }
        }

        let attachment_prepared = if let Some(image) = &request.image {
            if !client.target_url_matches(&current, &current.url, cancelled, timeout)? {
                return Ok(manual_fallback(
                    &current.url,
                    PreparationFailureStage::NavigationChanged,
                    RecoveryHint::ReopenProviderPage,
                    false,
                    false,
                ));
            }
            let temp_image = TempImage::create(temp_root, &request.id, &encode_png(image)?)?;
            let preferred_selectors = self
                .rule
                .as_ref()
                .map_or(&[][..], |rule| rule.file_input_selectors());
            let prepare_file_input = |target: &CdpTarget| {
                poll_file_input_preparation(cancelled, timeout, |attempt_timeout| {
                    client.set_file_input(
                        target,
                        &target.url,
                        temp_image.path(),
                        preferred_selectors,
                        cancelled,
                        attempt_timeout,
                    )
                })
            };
            let mut file_input_result = prepare_file_input(&current)?;
            if matches!(file_input_result, FileInputResult::NavigationChanged) {
                let refreshed = current_target(client, &current.id)?;
                if self
                    .rule
                    .as_ref()
                    .is_some_and(|rule| rule.matches_login_url(&refreshed.url))
                {
                    return Ok(manual_fallback(
                        &refreshed.url,
                        PreparationFailureStage::PageReadiness,
                        RecoveryHint::LoginInBrowser,
                        false,
                        false,
                    ));
                }
                if !self.matches_url(&refreshed.url) {
                    return Ok(manual_fallback(
                        &refreshed.url,
                        PreparationFailureStage::NavigationChanged,
                        RecoveryHint::ReopenProviderPage,
                        false,
                        false,
                    ));
                }
                file_input_result = prepare_file_input(&refreshed)?;
            }
            match file_input_result {
                FileInputResult::Prepared => true,
                FileInputResult::NavigationChanged => {
                    return Ok(manual_fallback(
                        &current.url,
                        PreparationFailureStage::NavigationChanged,
                        RecoveryHint::ReopenProviderPage,
                        false,
                        false,
                    ));
                }
                FileInputResult::NotFound | FileInputResult::Ambiguous => {
                    return Ok(manual_fallback(
                        &current.url,
                        PreparationFailureStage::AttachmentPreparation,
                        RecoveryHint::CopyImageThenText,
                        false,
                        false,
                    ));
                }
                FileInputResult::VerificationFailed => {
                    return Ok(manual_fallback(
                        &current.url,
                        PreparationFailureStage::Verification,
                        RecoveryHint::CopyImageThenText,
                        false,
                        false,
                    ));
                }
            }
        } else {
            false
        };

        let composer_target = current_target(client, &current.id)?;
        if self
            .rule
            .as_ref()
            .is_some_and(|rule| rule.matches_login_url(&composer_target.url))
        {
            return Ok(manual_fallback(
                &composer_target.url,
                PreparationFailureStage::PageReadiness,
                RecoveryHint::LoginInBrowser,
                false,
                attachment_prepared,
            ));
        }
        if !self.matches_url(&composer_target.url) {
            return Ok(manual_fallback(
                &composer_target.url,
                PreparationFailureStage::NavigationChanged,
                RecoveryHint::ReopenProviderPage,
                false,
                attachment_prepared,
            ));
        }
        let expected_url = composer_target.url.clone();
        let preferred_selectors = self
            .rule
            .as_ref()
            .map_or(&[][..], |rule| rule.composer_selectors());
        let login_selectors = self
            .rule
            .as_ref()
            .map_or(&[][..], |rule| rule.login_selectors());
        let expression = composer_insertion_expression(
            &request.prompt,
            preferred_selectors,
            login_selectors,
            &expected_url,
        )?;
        let result = poll_composer_preparation(cancelled, timeout, |attempt_timeout| {
            client.evaluate_in_target(&composer_target, &expression, cancelled, attempt_timeout)
        })?;
        let value = result.pointer("/result/value").ok_or_else(|| {
            AppError::BrowserProtocol("composer preparation returned no value".to_owned())
        })?;
        let target_url = value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or(&expected_url);
        let verified_target = current_target(client, &composer_target.id)?;
        if self.rule.as_ref().is_some_and(|rule| {
            rule.matches_login_url(target_url) || rule.matches_login_url(&verified_target.url)
        }) {
            return Ok(manual_fallback(
                target_url,
                PreparationFailureStage::PageReadiness,
                RecoveryHint::LoginInBrowser,
                false,
                attachment_prepared,
            ));
        }
        if value.get("status").and_then(Value::as_str) == Some("navigation_changed") {
            return Ok(manual_fallback(
                target_url,
                PreparationFailureStage::NavigationChanged,
                RecoveryHint::ReopenProviderPage,
                false,
                attachment_prepared,
            ));
        }
        if !self.matches_url(target_url) || !self.matches_url(&verified_target.url) {
            return Ok(manual_fallback(
                target_url,
                PreparationFailureStage::NavigationChanged,
                RecoveryHint::ReopenProviderPage,
                false,
                attachment_prepared,
            ));
        }
        match value.get("status").and_then(Value::as_str) {
            Some("inserted") => Ok(PreparationOutcome::prepared(
                target_url,
                true,
                attachment_prepared,
            )),
            Some("focused") => Ok(PreparationOutcome::prepared(
                target_url,
                false,
                attachment_prepared,
            )),
            Some("login_detected") => Ok(manual_fallback(
                target_url,
                PreparationFailureStage::PageReadiness,
                RecoveryHint::LoginInBrowser,
                false,
                attachment_prepared,
            )),
            Some("missing") => Ok(manual_fallback(
                target_url,
                PreparationFailureStage::ComposerDiscovery,
                if value
                    .get("providerRuleMiss")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    RecoveryHint::ProviderPageChanged
                } else {
                    RecoveryHint::FocusComposerAndPaste
                },
                false,
                attachment_prepared,
            )),
            Some("ambiguous") => Ok(manual_fallback(
                target_url,
                PreparationFailureStage::ComposerDiscovery,
                RecoveryHint::FocusComposerAndPaste,
                false,
                attachment_prepared,
            )),
            Some("verification_failed") => Ok(manual_fallback(
                target_url,
                PreparationFailureStage::Verification,
                RecoveryHint::FocusComposerAndPaste,
                false,
                attachment_prepared,
            )),
            _ => Err(AppError::BrowserProtocol(
                "composer preparation returned an invalid status".to_owned(),
            )),
        }
    }
}

impl ProviderAdapter for GenericProviderAdapter {
    fn id(&self) -> &str {
        &self.provider_id
    }

    fn matches_url(&self, url: &str) -> bool {
        matches_any_pattern(url, &self.url_patterns)
    }

    fn prepare(
        &self,
        page: &mut PageSession<'_>,
        request: &DispatchRequest,
        policy: &PreparationPolicy,
    ) -> Result<PreparationOutcome> {
        request.validate()?;
        let outcome = match page {
            PageSession::DedicatedChrome {
                client,
                target,
                temp_root,
                cancelled,
            } => {
                self.prepare_dedicated_chrome(client, target, temp_root, cancelled, request, policy)
            }
            PageSession::DesktopPwa { target_url }
                if !request.expects_text() && request.image.is_none() =>
            {
                Ok(PreparationOutcome::prepared(*target_url, false, false))
            }
            PageSession::DesktopPwa { target_url } => Ok(manual_fallback(
                target_url,
                PreparationFailureStage::ComposerDiscovery,
                if request.image.is_some() {
                    RecoveryHint::CopyImageThenText
                } else {
                    RecoveryHint::FocusComposerAndPaste
                },
                false,
                false,
            )),
        }?;
        Ok(outcome)
    }
}

pub(crate) fn cleanup_stale_temp_images(data_root: &Path) -> Result<()> {
    cleanup_temp_images_older_than(data_root, Duration::from_secs(24 * 60 * 60))
}

fn cleanup_temp_images_older_than(data_root: &Path, minimum_age: Duration) -> Result<()> {
    let temp_root = data_root.join("Temp");
    let entries = match fs::read_dir(&temp_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::io(
                "reading temporary image directory",
                &temp_root,
                error,
            ));
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_askbridge_png = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("askbridge-") && name.ends_with(".png"));
        if !is_askbridge_png {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= minimum_age);
        if stale {
            fs::remove_file(&path)
                .map_err(|error| AppError::io("removing stale temporary image", path, error))?;
        }
    }
    Ok(())
}

fn current_target(client: &CdpClient, target_id: &str) -> Result<CdpTarget> {
    client
        .list_targets()?
        .into_iter()
        .find(|candidate| candidate.id == target_id && candidate.kind == "page")
        .ok_or(AppError::TargetNotFound)
}

fn manual_fallback(
    target_url: &str,
    stage: PreparationFailureStage,
    hint: RecoveryHint,
    text_inserted: bool,
    attachment_prepared: bool,
) -> PreparationOutcome {
    PreparationOutcome::manual_fallback(target_url, stage, hint, text_inserted, attachment_prepared)
}

fn login_detection_expression(selectors: &[String]) -> Result<String> {
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

fn composer_insertion_expression(
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

const COMPOSER_POLL_INTERVAL: Duration = Duration::from_millis(100);
const COMPOSER_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(750);

fn poll_file_input_preparation<F>(
    cancelled: &AtomicBool,
    timeout: Duration,
    evaluate: F,
) -> Result<FileInputResult>
where
    F: FnMut(Duration) -> Result<FileInputResult>,
{
    poll_file_input_preparation_with_interval(cancelled, timeout, COMPOSER_POLL_INTERVAL, evaluate)
}

fn poll_file_input_preparation_with_interval<F>(
    cancelled: &AtomicBool,
    timeout: Duration,
    interval: Duration,
    mut evaluate: F,
) -> Result<FileInputResult>
where
    F: FnMut(Duration) -> Result<FileInputResult>,
{
    let deadline = polling_deadline(timeout, "file input")?;
    let mut observed_not_found = false;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let now = Instant::now();
        if now >= deadline {
            return if observed_not_found {
                Ok(FileInputResult::NotFound)
            } else {
                Err(AppError::TargetTimeout)
            };
        }
        let result = evaluate((deadline - now).min(COMPOSER_ATTEMPT_TIMEOUT))?;
        if !matches!(result, FileInputResult::NotFound) {
            return Ok(result);
        }
        observed_not_found = true;
        wait_for_next_probe(cancelled, deadline, interval)?;
    }
}

fn poll_composer_preparation<F>(
    cancelled: &AtomicBool,
    timeout: Duration,
    evaluate: F,
) -> Result<Value>
where
    F: FnMut(Duration) -> Result<Value>,
{
    poll_composer_preparation_with_interval(cancelled, timeout, COMPOSER_POLL_INTERVAL, evaluate)
}

fn poll_composer_preparation_with_interval<F>(
    cancelled: &AtomicBool,
    timeout: Duration,
    interval: Duration,
    mut evaluate: F,
) -> Result<Value>
where
    F: FnMut(Duration) -> Result<Value>,
{
    let deadline = polling_deadline(timeout, "composer")?;
    let mut last_missing = None;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let now = Instant::now();
        if now >= deadline {
            return last_missing.ok_or(AppError::TargetTimeout);
        }
        let result = evaluate((deadline - now).min(COMPOSER_ATTEMPT_TIMEOUT))?;
        let status = result
            .pointer("/result/value/status")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::BrowserProtocol(
                    "composer preparation returned an invalid status".to_owned(),
                )
            })?;
        if status != "missing" {
            return Ok(result);
        }
        last_missing = Some(result);

        wait_for_next_probe(cancelled, deadline, interval)?;
    }
}

fn polling_deadline(timeout: Duration, stage: &str) -> Result<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        AppError::InvalidPreparation(format!("{stage} polling timeout is too large"))
    })
}

fn wait_for_next_probe(
    cancelled: &AtomicBool,
    deadline: Instant,
    interval: Duration,
) -> Result<()> {
    let now = Instant::now();
    if now >= deadline {
        return Ok(());
    }
    let wait_until = now + interval.min(deadline - now);
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let remaining = wait_until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25).min(remaining));
    }
}

struct TempImage {
    path: PathBuf,
}

impl TempImage {
    fn create(temp_root: &Path, request_id: &str, bytes: &[u8]) -> Result<Self> {
        Self::create_with_writer(temp_root, request_id, bytes, |file, bytes| {
            file.write_all(bytes).and_then(|_| file.sync_all())
        })
    }

    fn create_with_writer<F>(
        temp_root: &Path,
        request_id: &str,
        bytes: &[u8],
        write: F,
    ) -> Result<Self>
    where
        F: FnOnce(&mut fs::File, &[u8]) -> std::io::Result<()>,
    {
        fs::create_dir_all(temp_root).map_err(|error| {
            AppError::io("creating temporary image directory", temp_root, error)
        })?;
        let safe_id: String = request_id
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
            .take(48)
            .collect();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = temp_root.join(format!(
            "askbridge-{}-{nonce}.png",
            if safe_id.is_empty() {
                "request"
            } else {
                &safe_id
            }
        ));
        let mut cleanup = TempImageCleanup::new(path.clone());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| AppError::io("creating temporary image", &path, error))?;
        cleanup.mark_created();
        write(&mut file, bytes)
            .map_err(|error| AppError::io("writing temporary image", &path, error))?;
        cleanup.disarm();
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

struct TempImageCleanup {
    path: PathBuf,
    armed: bool,
}

impl TempImageCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: false }
    }

    fn mark_created(&mut self) {
        self.armed = true;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempImageCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Drop for TempImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_composer_program_never_contains_submit_actions() {
        let expression =
            composer_insertion_expression("hello\nworld", &[], &[], "https://example.test/chat")
                .expect("expression");
        assert!(expression.contains("InputEvent('input'"));
        assert!(!expression.contains(".click("));
        assert!(!expression.contains("dispatchKeyEvent"));
    }

    #[test]
    fn composer_navigation_guard_precedes_dom_queries_and_mutation() {
        let expression =
            composer_insertion_expression("hello", &[], &[], "https://example.test/chat")
                .expect("expression");
        let guard = expression
            .find("if (location.href !== expectedUrl)")
            .expect("navigation guard");
        assert!(
            guard
                < expression
                    .find("document.querySelectorAll")
                    .expect("DOM query")
        );
        assert!(guard < expression.find("el.focus()").expect("focus"));
        assert!(guard < expression.find("InputEvent('input'").expect("input event"));
        assert!(expression.contains("status: 'navigation_changed'"));
    }

    #[test]
    fn login_detection_program_only_checks_structural_selectors() {
        let expression =
            login_detection_expression(&["a[href*=\"//accounts.google.com/\"]".to_owned()])
                .expect("expression");
        assert!(expression.contains("//accounts.google.com/"));
        assert!(expression.contains("loginDetected"));
        assert!(!expression.contains(".click("));
        assert!(!expression.contains("textContent"));
        assert!(!expression.contains("innerText"));
    }

    #[test]
    fn built_in_adapter_embeds_preferred_selectors_but_keeps_generic_fallback() {
        let adapter = GenericProviderAdapter::for_provider(
            "chatgpt",
            Some("chatgpt"),
            vec!["https://chatgpt.com/".to_owned()],
        )
        .expect("adapter");
        let selectors = adapter.rule.as_ref().expect("rule").composer_selectors();
        let expression =
            composer_insertion_expression("hello", selectors, &[], "https://chatgpt.com/chat")
                .expect("expression");
        assert!(expression.contains("#prompt-textarea"));
        assert!(expression.contains("textarea,[contenteditable=\"true\"]"));
        assert!(!expression.contains(".click("));
    }

    #[test]
    fn composer_poll_retries_missing_result_until_inserted() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_composer_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(if attempts < 3 {
                    serde_json::json!({"result": {"value": {"status": "missing"}}})
                } else {
                    serde_json::json!({"result": {"value": {"status": "inserted"}}})
                })
            },
        )
        .expect("poll result");

        assert_eq!(attempts, 3);
        assert_eq!(
            result
                .pointer("/result/value/status")
                .and_then(Value::as_str),
            Some("inserted")
        );
    }

    #[test]
    fn file_input_poll_retries_not_found_until_prepared() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_file_input_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(if attempts < 3 {
                    FileInputResult::NotFound
                } else {
                    FileInputResult::Prepared
                })
            },
        )
        .expect("poll result");

        assert_eq!(attempts, 3);
        assert!(matches!(result, FileInputResult::Prepared));
    }

    #[test]
    fn file_input_poll_stops_on_ambiguity() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_file_input_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(FileInputResult::Ambiguous)
            },
        )
        .expect("poll result");

        assert_eq!(attempts, 1);
        assert!(matches!(result, FileInputResult::Ambiguous));
    }

    #[test]
    fn composer_poll_stops_for_visible_login_structure() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_composer_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(serde_json::json!({
                    "result": {"value": {"status": "login_detected"}}
                }))
            },
        )
        .expect("poll result");

        assert_eq!(attempts, 1);
        assert_eq!(
            result
                .pointer("/result/value/status")
                .and_then(Value::as_str),
            Some("login_detected")
        );
    }

    #[test]
    fn composer_poll_honours_cancellation_before_evaluation() {
        let cancelled = AtomicBool::new(true);
        let result = poll_composer_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| panic!("cancelled polling must not evaluate the page"),
        );

        assert!(matches!(result, Err(AppError::BrowserCancelled)));
    }

    #[test]
    fn startup_cleanup_removes_only_owned_temp_images() {
        let directory = tempfile::tempdir().expect("temporary data directory");
        let temp_root = directory.path().join("Temp");
        fs::create_dir_all(&temp_root).expect("create temp root");
        let owned = temp_root.join("askbridge-stale.png");
        let unrelated = temp_root.join("keep.png");
        fs::write(&owned, b"owned").expect("write owned temp image");
        fs::write(&unrelated, b"unrelated").expect("write unrelated image");

        cleanup_temp_images_older_than(directory.path(), Duration::ZERO)
            .expect("cleanup owned images");

        assert!(!owned.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn generic_adapter_matches_only_provider_url_boundaries() {
        let adapter = GenericProviderAdapter::for_provider(
            "example",
            None,
            vec!["https://example.test/chat".to_owned()],
        )
        .expect("adapter");
        assert_eq!(adapter.id(), "example");
        assert!(adapter.matches_url("https://example.test/chat/1"));
        assert!(!adapter.matches_url("https://example.test/chatter"));
    }

    #[test]
    fn desktop_surface_stops_at_safe_manual_fallback() {
        let adapter = GenericProviderAdapter::for_provider(
            "chatgpt",
            None,
            vec!["https://chatgpt.com/".to_owned()],
        )
        .expect("adapter");
        let request = DispatchRequest::new(
            "text-1".to_owned(),
            askbridge_core::DispatchMode::TextOnlyPrompt,
            "chatgpt".to_owned(),
            "Explain".to_owned(),
            None,
            1,
        )
        .expect("request");
        let policy = PreparationPolicy::new(1_000).expect("policy");
        let mut page = PageSession::DesktopPwa {
            target_url: "desktop-pwa://chatgpt",
        };

        let outcome = adapter
            .prepare(&mut page, &request, &policy)
            .expect("fallback");
        assert!(outcome.manual_fallback_required);
        assert_eq!(
            outcome.recovery_hint,
            Some(RecoveryHint::FocusComposerAndPaste)
        );
    }

    #[test]
    fn manual_fallback_preserves_navigation_recovery_hint() {
        let outcome = manual_fallback(
            "https://example.test/changed",
            PreparationFailureStage::NavigationChanged,
            RecoveryHint::ReopenProviderPage,
            false,
            false,
        );

        assert_eq!(
            outcome.recovery_hint,
            Some(RecoveryHint::ReopenProviderPage)
        );
    }
}
