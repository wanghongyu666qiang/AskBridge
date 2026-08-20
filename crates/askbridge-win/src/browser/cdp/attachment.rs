use std::{collections::HashSet, sync::atomic::AtomicBool};

use askbridge_core::{AppError, Result};
use serde_json::{Value, json};

use super::connection::TargetSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AttachmentReceipt {
    pub(super) file_count: u64,
    pub(super) named_count: u64,
    pub(super) preview_count: u64,
    pub(super) busy_count: u64,
}

pub(super) fn has_new_attachment_receipt(
    baseline: AttachmentReceipt,
    current: AttachmentReceipt,
) -> bool {
    current.file_count > 0
        && current.busy_count == 0
        && (current.named_count > baseline.named_count
            || current.preview_count > baseline.preview_count)
}

pub(super) fn attachment_receipt(
    socket: &mut TargetSession<'_>,
    object_id: &str,
    file_name: &str,
    cancelled: &AtomicBool,
) -> Result<AttachmentReceipt> {
    let value = socket.command(
        "Runtime.callFunctionOn",
        Some(json!({
            "objectId": object_id,
            "functionDeclaration": r#"function(expectedName) {
                const visible = (element) => {
                    const style = getComputedStyle(element);
                    const rect = element.getBoundingClientRect();
                    return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0;
                };
                const root = this.closest('form') || this.parentElement || document.body;
                const expected = String(expectedName || '').toLowerCase();
                let named = 0;
                let previews = 0;
                let busy = 0;
                for (const element of root.querySelectorAll('*')) {
                    if (!visible(element)) continue;
                    const attributes = [
                        element.getAttribute('aria-label'),
                        element.getAttribute('title'),
                        element.getAttribute('data-testid')
                    ].filter(Boolean).join(' ').toLowerCase();
                    const text = (element.textContent || '').trim().toLowerCase();
                    if (expected && (attributes.includes(expected) || text.includes(expected))) named++;
                    if (element.tagName === 'IMG') {
                        const source = String(element.getAttribute('src') || '');
                        if (source.startsWith('blob:') || source.startsWith('data:image/')) previews++;
                    }
                    const state = String(element.getAttribute('data-state') || '').toLowerCase();
                    if (element.getAttribute('aria-busy') === 'true' || element.getAttribute('role') === 'progressbar' || state === 'uploading' || state === 'pending') busy++;
                }
                return {
                    fileCount: this.files ? this.files.length : 0,
                    namedCount: named,
                    previewCount: previews,
                    busyCount: busy
                };
            }"#,
            "arguments": [{"value": file_name}],
            "returnByValue": true
        })),
        cancelled,
    )?;
    let result = value
        .pointer("/result/value")
        .ok_or_else(|| AppError::BrowserProtocol("attachment receipt is missing".to_owned()))?;
    let count = |name: &str| {
        result
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| AppError::BrowserProtocol(format!("attachment receipt has no {name}")))
    };
    Ok(AttachmentReceipt {
        file_count: count("fileCount")?,
        named_count: count("namedCount")?,
        preview_count: count("previewCount")?,
        busy_count: count("busyCount")?,
    })
}

pub(super) fn query_acceptable_file_inputs(
    socket: &mut TargetSession<'_>,
    root_id: i64,
    selectors: &[String],
    cancelled: &AtomicBool,
) -> Result<Vec<i64>> {
    let mut node_ids = HashSet::new();
    for selector in selectors {
        let query = socket.command(
            "DOM.querySelectorAll",
            Some(json!({"nodeId": root_id, "selector": selector})),
            cancelled,
        )?;
        let queried = query
            .get("nodeIds")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::BrowserProtocol("file input query failed".to_owned()))?;
        for node_id in queried.iter().filter_map(Value::as_i64) {
            node_ids.insert(node_id);
        }
    }
    let mut candidates = Vec::new();
    for node_id in node_ids {
        let attributes = socket.command(
            "DOM.getAttributes",
            Some(json!({"nodeId": node_id})),
            cancelled,
        )?;
        if file_input_accepts_png(&attributes) {
            candidates.push(node_id);
        }
    }
    candidates.sort_unstable();
    Ok(candidates)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileInputResult {
    Prepared,
    NotFound,
    Ambiguous,
    VerificationFailed,
    NavigationChanged,
}

pub(super) fn file_input_accepts_png(result: &Value) -> bool {
    let Some(attributes) = result.get("attributes").and_then(Value::as_array) else {
        return false;
    };
    let pairs: Vec<(&str, &str)> = attributes
        .chunks_exact(2)
        .filter_map(|pair| Some((pair[0].as_str()?, pair[1].as_str()?)))
        .collect();
    if pairs
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("disabled"))
    {
        return false;
    }
    pairs
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("accept"))
        .is_none_or(|(_, accept)| {
            accept.trim().is_empty()
                || accept.split(',').any(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "image/*" | "image/png" | ".png"
                    )
                })
        })
}
