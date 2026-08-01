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

fn matches_any_pattern(url: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim_end_matches('/');
        url == pattern
            || url
                .strip_prefix(pattern)
                .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with('?'))
    })
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
