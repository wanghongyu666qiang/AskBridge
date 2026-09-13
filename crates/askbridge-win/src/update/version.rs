//! Semantic version parsing and ordering for release tags, including
//! prerelease identifiers following semver precedence rules.

use askbridge_core::Result;

use super::update_error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<Vec<PrereleaseIdentifier>>,
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(left), Some(right)) => left.cmp(right),
            })
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrereleaseIdentifier {
    Numeric(u64),
    Text(String),
}

impl Ord for PrereleaseIdentifier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Numeric(left), Self::Numeric(right)) => left.cmp(right),
            (Self::Numeric(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Numeric(_)) => std::cmp::Ordering::Greater,
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
        }
    }
}

impl PartialOrd for PrereleaseIdentifier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl ReleaseVersion {
    pub(super) fn parse(value: &str) -> Result<Self> {
        let value = value.split_once('+').map_or(value, |(core, _)| core);
        let (core, prerelease) = value
            .split_once('-')
            .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
        let mut parts = core.split('.');
        let major = parse_version_number(parts.next(), value)?;
        let minor = parse_version_number(parts.next(), value)?;
        let patch = parse_version_number(parts.next(), value)?;
        if parts.next().is_some() {
            return Err(update_error(format!("版本号 {value} 格式无效")));
        }
        let prerelease = prerelease
            .map(parse_prerelease)
            .transpose()?
            .filter(|parts| !parts.is_empty());
        Ok(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }
}

fn parse_version_number(value: Option<&str>, complete: &str) -> Result<u64> {
    let value = value.ok_or_else(|| update_error(format!("版本号 {complete} 格式无效")))?;
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(update_error(format!("版本号 {complete} 格式无效")));
    }
    value
        .parse()
        .map_err(|_| update_error(format!("版本号 {complete} 超出范围")))
}

fn parse_prerelease(value: &str) -> Result<Vec<PrereleaseIdentifier>> {
    value
        .split('.')
        .map(|part| {
            if part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err(update_error("预发布版本号格式无效"));
            }
            if part.bytes().all(|byte| byte.is_ascii_digit()) {
                if part.len() > 1 && part.starts_with('0') {
                    return Err(update_error("预发布数字版本不能有前导零"));
                }
                part.parse::<u64>()
                    .map(PrereleaseIdentifier::Numeric)
                    .map_err(|_| update_error("预发布版本号超出范围"))
            } else {
                Ok(PrereleaseIdentifier::Text(part.to_owned()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_versions_order_stable_after_prerelease() {
        let prerelease = ReleaseVersion::parse("1.2.3-rc.1").expect("prerelease");
        let stable = ReleaseVersion::parse("1.2.3").expect("stable");
        let next = ReleaseVersion::parse("1.2.4").expect("next");
        assert!(prerelease < stable);
        assert!(stable < next);
    }

    #[test]
    fn rejects_ambiguous_or_invalid_versions() {
        for value in ["v1.2.3", "1.2", "01.2.3", "1.2.3-01", "1.2.x"] {
            assert!(ReleaseVersion::parse(value).is_err(), "{value}");
        }
    }
}
