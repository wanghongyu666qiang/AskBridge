use std::{collections::HashSet, sync::atomic::AtomicBool};

use askbridge_core::{AppError, Result};
use serde_json::{Value, json};

use super::connection::TargetSession;

pub(super) fn first_preferred_file_input_candidates<F>(
    selectors: &[String],
    mut query: F,
) -> Result<Vec<i64>>
where
    F: FnMut(&str) -> Result<Vec<i64>>,
{
    for selector in selectors {
        let candidates = query(selector)?;
        if !candidates.is_empty() {
            return Ok(candidates);
        }
    }
    Ok(Vec::new())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AttachmentReceipt {
    pub(super) named_count: u64,
    pub(super) preview_count: u64,
    pub(super) busy_count: u64,
}

pub(super) fn has_new_attachment_receipt(
    baseline: AttachmentReceipt,
    current: AttachmentReceipt,
) -> bool {
    current.busy_count == 0
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
        .as_chunks::<2>()
        .0
        .iter()
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

#[cfg(test)]
mod preferred_selector_tests {
    use super::*;

    #[test]
    fn first_matching_exact_selector_wins_without_unioning_layout_variants() {
        let selectors = vec!["#desktop-upload".to_owned(), "#mobile-upload".to_owned()];
        let mut queried = Vec::new();
        let candidates = first_preferred_file_input_candidates(&selectors, |selector| {
            queried.push(selector.to_owned());
            Ok(match selector {
                "#desktop-upload" => Vec::new(),
                "#mobile-upload" => vec![42],
                _ => vec![90, 91, 92],
            })
        })
        .expect("candidate");

        assert_eq!(candidates, [42]);
        assert_eq!(queried, selectors);
    }

    #[test]
    fn ambiguity_within_one_exact_selector_is_preserved_for_fail_closed_handling() {
        let selectors = vec!["#upload".to_owned(), "input[type=file]".to_owned()];
        let candidates = first_preferred_file_input_candidates(&selectors, |selector| {
            Ok(if selector == "#upload" {
                vec![7, 8]
            } else {
                vec![9]
            })
        })
        .expect("candidates");

        assert_eq!(candidates, [7, 8]);
    }
}
