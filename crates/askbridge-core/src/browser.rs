#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserTarget {
    pub id: String,
    pub url: String,
}

impl BrowserTarget {
    pub fn new(id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusEvidence {
    Confirmed(String),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetDecision {
    UseExisting(String),
    CreateNew,
}

pub struct TargetResolver;

impl TargetResolver {
    pub fn resolve(
        targets: &[BrowserTarget],
        url_patterns: &[String],
        focus: &FocusEvidence,
    ) -> TargetDecision {
        let matches: Vec<&BrowserTarget> = targets
            .iter()
            .filter(|target| matches_any_pattern(&target.url, url_patterns))
            .collect();

        if let FocusEvidence::Confirmed(focused_id) = focus
            && matches.iter().any(|target| target.id == *focused_id)
        {
            return TargetDecision::UseExisting(focused_id.clone());
        }

        match matches.as_slice() {
            [only] => TargetDecision::UseExisting(only.id.clone()),
            [] | [_, _, ..] => TargetDecision::CreateNew,
        }
    }
}

pub fn matches_any_pattern(url: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = normalize_scheme_and_host(pattern.trim_end_matches('/'));
        let url = normalize_scheme_and_host(url);
        url == pattern
            || url
                .strip_prefix(&pattern)
                .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with('?'))
    })
}

/// Lowercases the `scheme://host` prefix so matching does not depend on the
/// case a browser or the configuration happens to use; path and query keep
/// their casing because some sites treat them as case-sensitive.
fn normalize_scheme_and_host(value: &str) -> String {
    let Some((scheme, rest)) = value.split_once("://") else {
        return value.to_owned();
    };
    let (host, suffix) = match rest.find(['/', '?', '#']) {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    format!(
        "{}://{}{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        suffix
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str, url: &str) -> BrowserTarget {
        BrowserTarget::new(id, url)
    }

    fn patterns() -> Vec<String> {
        vec!["https://example.test/chat".to_owned()]
    }

    #[test]
    fn confirmed_matching_focus_wins_among_multiple_matches() {
        let targets = vec![
            target("one", "https://example.test/chat/1"),
            target("two", "https://example.test/chat/2"),
        ];

        assert_eq!(
            TargetResolver::resolve(
                &targets,
                &patterns(),
                &FocusEvidence::Confirmed("two".to_owned())
            ),
            TargetDecision::UseExisting("two".to_owned())
        );
    }

    #[test]
    fn unique_match_is_used_when_focus_is_unknown_or_unrelated() {
        let targets = vec![
            target("matching", "https://example.test/chat/1"),
            target("other", "https://unrelated.test/"),
        ];

        assert_eq!(
            TargetResolver::resolve(&targets, &patterns(), &FocusEvidence::Unknown),
            TargetDecision::UseExisting("matching".to_owned())
        );
        assert_eq!(
            TargetResolver::resolve(
                &targets,
                &patterns(),
                &FocusEvidence::Confirmed("other".to_owned())
            ),
            TargetDecision::UseExisting("matching".to_owned())
        );
    }

    #[test]
    fn no_match_creates_a_new_target() {
        let targets = vec![target("other", "https://unrelated.test/")];

        assert_eq!(
            TargetResolver::resolve(&targets, &patterns(), &FocusEvidence::Unknown),
            TargetDecision::CreateNew
        );
    }

    #[test]
    fn scheme_and_host_case_differences_still_match() {
        let uppercase_patterns = vec!["https://ChatGPT.com/".to_owned()];
        assert!(matches_any_pattern(
            "https://chatgpt.com/c/abc",
            &uppercase_patterns
        ));

        let lowercase_patterns = vec!["https://chatgpt.com/".to_owned()];
        assert!(matches_any_pattern(
            "https://ChatGPT.com/c/ABC",
            &lowercase_patterns
        ));
        // A case difference inside the pattern's own path stays significant.
        let path_sensitive = vec!["https://example.test/Chat".to_owned()];
        assert!(matches_any_pattern(
            "https://EXAMPLE.test/Chat/x",
            &path_sensitive
        ));
        assert!(!matches_any_pattern(
            "https://example.test/chat",
            &path_sensitive
        ));
    }

    #[test]
    fn ambiguous_matches_without_confirmed_focus_create_a_new_target() {
        let targets = vec![
            target("one", "https://example.test/chat/1"),
            target("two", "https://example.test/chat/2"),
        ];

        assert_eq!(
            TargetResolver::resolve(&targets, &patterns(), &FocusEvidence::Unknown),
            TargetDecision::CreateNew
        );
        assert_eq!(
            TargetResolver::resolve(
                &targets,
                &patterns(),
                &FocusEvidence::Confirmed("missing".to_owned())
            ),
            TargetDecision::CreateNew
        );
    }

    #[test]
    fn pattern_match_respects_path_boundary() {
        let targets = vec![
            target("valid", "https://example.test/chat?new=1"),
            target("invalid", "https://example.test/chatter"),
        ];

        assert_eq!(
            TargetResolver::resolve(&targets, &patterns(), &FocusEvidence::Unknown),
            TargetDecision::UseExisting("valid".to_owned())
        );
    }
}
