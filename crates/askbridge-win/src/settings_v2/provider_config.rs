use askbridge_core::{AppError, ProviderConfig, Result};

pub(super) fn parse_custom_providers(text: &str) -> Result<Vec<ProviderConfig>> {
    let mut providers = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields = line.split('|').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(AppError::InvalidProvider(format!(
                "custom provider line {} must contain id | name | start URL | match prefixes",
                index + 1
            )));
        }
        let url_patterns = fields[3]
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let provider = ProviderConfig {
            id: fields[0].to_owned(),
            display_name: fields[1].to_owned(),
            enabled: true,
            start_url: fields[2].to_owned(),
            url_patterns,
            is_custom: true,
            adapter_override: None,
        };
        provider.validate()?;
        providers.push(provider);
    }
    Ok(providers)
}

pub(super) fn origin_pattern(url: &str) -> Result<String> {
    let Some(remainder) = url.strip_prefix("https://") else {
        return Err(AppError::InvalidProviderUrl(url.to_owned()));
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(AppError::InvalidProviderUrl(url.to_owned()));
    }
    Ok(format!("https://{authority}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_rows_require_all_safe_fields() {
        assert!(
            parse_custom_providers("id | Name | https://example.test/ | https://example.test/")
                .is_ok()
        );
        assert!(parse_custom_providers("id | Name | javascript:alert(1) | javascript:").is_err());
        assert!(parse_custom_providers("missing fields").is_err());
    }
}
