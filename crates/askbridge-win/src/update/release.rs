//! Fetches the latest GitHub release metadata and validates that every asset
//! belongs to the official AskBridge repository.

use askbridge_core::Result;
use serde::Deserialize;

use super::MAX_RELEASE_BYTES;
use super::http::get_https;
use super::version::ReleaseVersion;
use super::{AvailableUpdate, update_error};

const RELEASE_API_URL: &str =
    "https://api.github.com/repos/wanghongyu666qiang/AskBridge/releases/latest";
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/wanghongyu666qiang/AskBridge/releases/download/";
const RELEASE_PAGE_PREFIX: &str = "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/";
const MAX_RELEASE_NOTES_CHARS: usize = 2_000;

pub(super) fn check_latest(current_version: &ReleaseVersion) -> Result<Option<AvailableUpdate>> {
    let source = get_https(RELEASE_API_URL, MAX_RELEASE_BYTES)?;
    let release: GithubRelease = serde_json::from_slice(&source)
        .map_err(|_| update_error("GitHub 返回的更新信息不是有效 JSON"))?;
    parse_release(release, current_version)
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
    let expected_setup = format!("AskBridge-{version_text}-Setup.exe");
    let expected_checksums = format!("AskBridge-{version_text}-SHA256SUMS.txt");
    let expected_signature = format!("{expected_checksums}.sig");
    let setup = single_asset(&release.assets, &expected_setup)?;
    let checksums = single_asset(&release.assets, &expected_checksums)?;
    let signature = single_asset(&release.assets, &expected_signature)?;
    validate_release_asset(
        &setup.browser_download_url,
        &release.tag_name,
        &expected_setup,
    )?;
    validate_release_asset(
        &checksums.browser_download_url,
        &release.tag_name,
        &expected_checksums,
    )?;
    validate_release_asset(
        &signature.browser_download_url,
        &release.tag_name,
        &expected_signature,
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
        setup_name: expected_setup,
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
