//! Fetches the latest GitHub release metadata and validates that every asset
//! belongs to the official AskBridge repository.

use askbridge_core::{AppError, Result};
use serde::Deserialize;

use super::MAX_RELEASE_BYTES;
use super::http::{get_https, probe_redirect_target};
use super::version::ReleaseVersion;
use super::{AvailableUpdate, update_error};

const RELEASE_API_URL: &str =
    "https://api.github.com/repos/wanghongyu666qiang/AskBridge/releases/latest";
const RELEASE_LATEST_PAGE_URL: &str =
    "https://github.com/wanghongyu666qiang/AskBridge/releases/latest";
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/wanghongyu666qiang/AskBridge/releases/download/";
const RELEASE_PAGE_PREFIX: &str = "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/";
const RELEASE_TAG_PATH_PREFIX: &str = "/wanghongyu666qiang/AskBridge/releases/tag/";
const MAX_RELEASE_NOTES_CHARS: usize = 2_000;

pub(super) fn check_latest(current_version: &ReleaseVersion) -> Result<Option<AvailableUpdate>> {
    match fetch_latest_from_api(current_version) {
        Ok(outcome) => Ok(outcome),
        Err(api_error) => {
            fetch_latest_from_release_page(current_version).map_err(|fallback_error| {
                update_error(format!(
                    "{}；备用发布页探测也失败：{}",
                    error_text(&api_error),
                    error_text(&fallback_error)
                ))
            })
        }
    }
}

fn fetch_latest_from_api(current_version: &ReleaseVersion) -> Result<Option<AvailableUpdate>> {
    let source = get_https(RELEASE_API_URL, MAX_RELEASE_BYTES)?;
    let release: GithubRelease = serde_json::from_slice(&source)
        .map_err(|_| update_error("GitHub 返回的更新信息不是有效 JSON"))?;
    parse_release(release, current_version)
}

/// Resolves the latest release without the GitHub API: the release page
/// redirects `/releases/latest` to `/releases/tag/vX.Y.Z`, and every official
/// asset name is derived from the version alone, so the tag is sufficient to
/// construct all download URLs. The fallback notes are intentionally brief
/// because the API-provided notes body is unavailable on this path.
fn fetch_latest_from_release_page(
    current_version: &ReleaseVersion,
) -> Result<Option<AvailableUpdate>> {
    let location = probe_redirect_target(RELEASE_LATEST_PAGE_URL)?;
    let tag = parse_release_tag(&location)?;
    let version_text = tag
        .strip_prefix('v')
        .ok_or_else(|| update_error("发布标签必须以 v 开头"))?;
    let version = ReleaseVersion::parse(version_text)?;
    if version <= *current_version {
        return Ok(None);
    }
    let names = official_asset_names(version_text);
    let setup_url = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{}", names.setup);
    Ok(Some(AvailableUpdate {
        version: version_text.to_owned(),
        notes: "详细更新内容请查看发布页面。".to_owned(),
        release_url: format!("{RELEASE_PAGE_PREFIX}{tag}"),
        setup_name: names.setup,
        setup_url,
        setup_size: 0,
        checksum_url: format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{}", names.checksums),
        signature_url: format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{}", names.signature),
    }))
}

/// Accepts only the official repository's own redirect targets, either the
/// absolute `https://github.com/...` form or a site-relative path, and
/// returns the trailing tag after validating it carries no extra segments.
fn parse_release_tag(location: &str) -> Result<String> {
    let path = if let Some(rest) = location.strip_prefix("https://github.com/") {
        format!("/{rest}")
    } else if location.starts_with('/') {
        location.to_owned()
    } else {
        return Err(update_error("发布页跳转地址无法识别"));
    };
    let tag = path
        .strip_prefix(RELEASE_TAG_PATH_PREFIX)
        .ok_or_else(|| update_error("发布页跳转地址不属于 AskBridge 官方仓库"))?;
    if tag.is_empty() || tag.contains(['/', '?', '#']) {
        return Err(update_error("发布页跳转地址中的标签无效"));
    }
    Ok(tag.to_owned())
}

fn error_text(error: &AppError) -> String {
    match error {
        AppError::UpdateFailed(message) => message.clone(),
        other => other.to_string(),
    }
}

struct OfficialAssetNames {
    setup: String,
    checksums: String,
    signature: String,
}

fn official_asset_names(version_text: &str) -> OfficialAssetNames {
    let setup = format!("AskBridge-{version_text}-Setup.exe");
    let checksums = format!("AskBridge-{version_text}-SHA256SUMS.txt");
    let signature = format!("{checksums}.sig");
    OfficialAssetNames {
        setup,
        checksums,
        signature,
    }
}

fn parse_release(
    release: GithubRelease,
    current_version: &ReleaseVersion,
) -> Result<Option<AvailableUpdate>> {
    let version_text = release
        .tag_name
        .strip_prefix('v')
        .ok_or_else(|| update_error("发布标签必须以 v 开头"))?;
    let version = ReleaseVersion::parse(version_text)?;
    if version <= *current_version {
        return Ok(None);
    }
    let names = official_asset_names(version_text);
    let setup = single_asset(&release.assets, &names.setup)?;
    let checksums = single_asset(&release.assets, &names.checksums)?;
    let signature = single_asset(&release.assets, &names.signature)?;
    validate_release_asset(&setup.browser_download_url, &release.tag_name, &names.setup)?;
    validate_release_asset(
        &checksums.browser_download_url,
        &release.tag_name,
        &names.checksums,
    )?;
    validate_release_asset(
        &signature.browser_download_url,
        &release.tag_name,
        &names.signature,
    )?;
    let expected_page = format!("{RELEASE_PAGE_PREFIX}{}", release.tag_name);
    if release.html_url != expected_page {
        return Err(update_error("发布页面地址不属于 AskBridge 官方仓库"));
    }
    if setup.size == 0 || setup.size > super::MAX_SETUP_BYTES as u64 {
        return Err(update_error("更新安装包大小超出安全限制"));
    }
    Ok(Some(AvailableUpdate {
        version: version_text.to_owned(),
        notes: truncate_chars(release.body.unwrap_or_default(), MAX_RELEASE_NOTES_CHARS),
        release_url: release.html_url,
        setup_name: names.setup,
        setup_url: setup.browser_download_url.clone(),
        setup_size: setup.size,
        checksum_url: checksums.browser_download_url.clone(),
        signature_url: signature.browser_download_url.clone(),
    }))
}

fn single_asset<'a>(assets: &'a [GithubAsset], expected_name: &str) -> Result<&'a GithubAsset> {
    let mut matches = assets.iter().filter(|asset| asset.name == expected_name);
    let asset = matches
        .next()
        .ok_or_else(|| update_error(format!("发布缺少 {expected_name}")))?;
    if matches.next().is_some() {
        return Err(update_error(format!("发布包含重复的 {expected_name}")));
    }
    Ok(asset)
}

fn validate_release_asset(url: &str, tag: &str, name: &str) -> Result<()> {
    let expected = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{name}");
    if url != expected {
        return Err(update_error("更新资产地址不属于 AskBridge 官方仓库"));
    }
    Ok(())
}

fn truncate_chars(mut text: String, max_chars: usize) -> String {
    if let Some((index, _)) = text.char_indices().nth(max_chars) {
        text.truncate(index);
    }
    text
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_official_release_assets() {
        assert!(
            validate_release_asset(
                "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-Setup.exe",
                "v1.2.3",
                "AskBridge-1.2.3-Setup.exe"
            )
            .is_ok()
        );
        assert!(
            validate_release_asset(
                "https://example.test/AskBridge-1.2.3-Setup.exe",
                "v1.2.3",
                "AskBridge-1.2.3-Setup.exe"
            )
            .is_err()
        );
    }

    #[test]
    fn newer_release_requires_the_complete_official_asset_pair() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_owned(),
            html_url:
                "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3"
                    .to_owned(),
            body: Some("notes".to_owned()),
            assets: vec![
                GithubAsset {
                    name: "AskBridge-1.2.3-Setup.exe".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-Setup.exe".to_owned(),
                    size: 4096,
                },
                GithubAsset {
                    name: "AskBridge-1.2.3-SHA256SUMS.txt".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-SHA256SUMS.txt".to_owned(),
                    size: 256,
                },
                GithubAsset {
                    name: "AskBridge-1.2.3-SHA256SUMS.txt.sig".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-SHA256SUMS.txt.sig".to_owned(),
                    size: 128,
                },
            ],
        };
        let current = ReleaseVersion::parse("1.2.2").expect("current");
        let available = parse_release(release, &current)
            .expect("release")
            .expect("newer release");
        assert_eq!(available.version(), "1.2.3");
        assert_eq!(available.notes(), "notes");
        assert!(
            available
                .signature_url
                .ends_with("/AskBridge-1.2.3-SHA256SUMS.txt.sig")
        );
    }

    #[test]
    fn release_without_the_signature_asset_is_rejected() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_owned(),
            html_url:
                "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3"
                    .to_owned(),
            body: None,
            assets: vec![
                GithubAsset {
                    name: "AskBridge-1.2.3-Setup.exe".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-Setup.exe".to_owned(),
                    size: 4096,
                },
                GithubAsset {
                    name: "AskBridge-1.2.3-SHA256SUMS.txt".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-SHA256SUMS.txt".to_owned(),
                    size: 256,
                },
            ],
        };
        let current = ReleaseVersion::parse("1.2.2").expect("current");
        let error = parse_release(release, &current).expect_err("missing signature asset");
        assert!(
            error.to_string().contains("SHA256SUMS.txt.sig"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn truncates_release_notes_on_character_boundaries() {
        assert_eq!(truncate_chars("更新说明abc".to_owned(), 4), "更新说明");
    }

    #[test]
    fn release_page_fallback_constructs_official_assets_from_the_tag() {
        let location = "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/v2.0.6";
        let tag = parse_release_tag(location).expect("tag");
        assert_eq!(tag, "v2.0.6");

        let current = ReleaseVersion::parse("2.0.5").expect("current");
        let version_text = tag.strip_prefix('v').expect("version text");
        let version = ReleaseVersion::parse(version_text).expect("version");
        assert!(version > current);
        let names = official_asset_names(version_text);
        assert_eq!(names.setup, "AskBridge-2.0.6-Setup.exe");
        assert_eq!(names.checksums, "AskBridge-2.0.6-SHA256SUMS.txt");
        assert_eq!(names.signature, "AskBridge-2.0.6-SHA256SUMS.txt.sig");
        assert_eq!(
            format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{}", names.setup),
            "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v2.0.6/AskBridge-2.0.6-Setup.exe"
        );
    }

    #[test]
    fn release_page_fallback_accepts_relative_redirect_only_for_official_repo() {
        assert_eq!(
            parse_release_tag("/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3")
                .expect("relative redirect"),
            "v1.2.3"
        );
        for location in [
            "https://example.com/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3",
            "/someone-else/AskBridge/releases/tag/v1.2.3",
            "/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-Setup.exe",
            "/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3/extra",
            "/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3?query=1",
            "/wanghongyu666qiang/AskBridge/releases/tag/",
            "ftp://github.com/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3",
        ] {
            assert!(parse_release_tag(location).is_err(), "{location}");
        }
    }

    #[test]
    fn release_page_fallback_ignores_releases_not_newer_than_current() {
        let tag = parse_release_tag(
            "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/v2.0.5",
        )
        .expect("tag");
        let version =
            ReleaseVersion::parse(tag.strip_prefix('v').expect("version text")).expect("version");
        let current = ReleaseVersion::parse("2.0.5").expect("current");
        assert!(version <= current);
    }

    #[test]
    #[ignore = "requires live GitHub access"]
    fn live_github_release_matches_the_update_contract() {
        let baseline = ReleaseVersion::parse("0.0.0").expect("baseline");
        let release = check_latest(&baseline)
            .expect("live release check")
            .expect("published release");
        assert!(release.version().split('.').count() == 3);
        assert!(release.release_url().starts_with(RELEASE_PAGE_PREFIX));
    }
}
