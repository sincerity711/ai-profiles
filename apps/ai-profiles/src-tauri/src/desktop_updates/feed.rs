use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app_kind::AppKind;

pub const CLAUDE_FEED: &str =
    "https://releases.claude.com/api/desktop/darwin/universal/squirrel/update";
pub const CODEX_FEED: &str = "https://persistent.oaistatic.com/codex-app-prod/appcast.xml";
const CODEX_KEY: &str = "mNfr1v9t63BfgDtlw4C8lRvSY6uMggIXABDOCi3tS6k=";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub build: String,
    pub url: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub signature: Option<String>,
}

pub fn version_parts(version: &str) -> Result<Vec<u64>, String> {
    version
        .split('.')
        .map(|p| {
            p.parse()
                .map_err(|_| format!("Unsupported version: {version}"))
        })
        .collect()
}

pub fn newer(candidate: &str, installed: &str) -> Result<bool, String> {
    let mut a = version_parts(candidate)?;
    let mut b = version_parts(installed)?;
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    Ok(a > b)
}

pub fn allowed_url(app: AppKind, url: &reqwest::Url) -> bool {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
    {
        return false;
    }
    match app {
        AppKind::Claude => matches!(
            url.host_str(),
            Some("releases.claude.com" | "downloads.claude.ai")
        ),
        AppKind::Codex => {
            url.host_str() == Some("persistent.oaistatic.com")
                && url.path().starts_with("/codex-app-prod/")
        }
    }
}

fn validate(app: AppKind, release: Release) -> Result<Release, String> {
    version_parts(&release.version)?;
    version_parts(&release.build)?;
    let url = reqwest::Url::parse(&release.url).map_err(|e| e.to_string())?;
    if !allowed_url(app, &url) || !url.path().ends_with(".zip") {
        return Err("The update feed returned an unexpected download URL".into());
    }
    Ok(release)
}

pub fn claude(text: &str) -> Result<Release, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let current = value["currentRelease"]
        .as_str()
        .ok_or("Claude feed has no current release")?;
    let entry = value["releases"]
        .as_array()
        .and_then(|a| a.iter().find(|r| r["version"].as_str() == Some(current)))
        .ok_or("Claude feed has no full release")?;
    let update = &entry["updateTo"];
    let hash = update["sha256"]
        .as_str()
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or("Claude feed has no SHA-256 checksum")?;
    validate(
        AppKind::Claude,
        Release {
            version: current.into(),
            build: current.into(),
            url: update["url"]
                .as_str()
                .ok_or("Claude feed has no URL")?
                .into(),
            size: update["size"].as_u64(),
            sha256: Some(hash.to_lowercase()),
            signature: None,
        },
    )
}

pub fn codex(text: &str, arch: &str, os: &str) -> Result<Release, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let mut releases = Vec::new();
    for item in doc.descendants().filter(|n| n.has_tag_name("item")) {
        let field = |name: &str| {
            item.children()
                .find(|n| n.is_element() && n.tag_name().name() == name)
                .and_then(|n| n.text())
        };
        if field("hardwareRequirements")
            .is_some_and(|required| !required.split(',').any(|a| a.trim() == arch))
        {
            continue;
        }
        if let Some(min) = field("minimumSystemVersion") {
            if newer(min, os)? {
                continue;
            }
        }
        // Direct children only: Sparkle delta enclosures are not full archives.
        let Some(enclosure) = item.children().find(|n| n.has_tag_name("enclosure")) else {
            continue;
        };
        let signature = enclosure
            .attributes()
            .find(|a| a.name() == "edSignature")
            .map(|a| a.value());
        let release = Release {
            version: field("shortVersionString")
                .ok_or("Missing ChatGPT version")?
                .into(),
            build: field("version").ok_or("Missing ChatGPT build")?.into(),
            url: enclosure
                .attribute("url")
                .ok_or("Missing ChatGPT URL")?
                .into(),
            size: enclosure.attribute("length").and_then(|s| s.parse().ok()),
            sha256: None,
            signature: Some(signature.ok_or("Missing ChatGPT archive signature")?.into()),
        };
        releases.push(validate(AppKind::Codex, release)?);
    }
    let mut latest: Option<Release> = None;
    for release in releases {
        if latest
            .as_ref()
            .map(|r| newer(&release.build, &r.build))
            .transpose()?
            .unwrap_or(true)
        {
            latest = Some(release);
        }
    }
    latest.ok_or_else(|| "No compatible ChatGPT release for this Mac".into())
}

pub fn verify_archive(app: AppKind, release: &Release, bytes: &[u8]) -> Result<(), String> {
    if release.size.is_some_and(|s| s != bytes.len() as u64) {
        return Err("Download size mismatch; download again".into());
    }
    match app {
        AppKind::Claude => {
            let actual = format!("{:x}", Sha256::digest(bytes));
            if release.sha256.as_deref() != Some(&actual) {
                return Err("Claude archive checksum mismatch".into());
            }
        }
        AppKind::Codex => {
            let engine = base64::engine::general_purpose::STANDARD;
            let key: [u8; 32] = engine
                .decode(CODEX_KEY)
                .map_err(|e| e.to_string())?
                .try_into()
                .map_err(|_| "Invalid update key")?;
            let signature = engine
                .decode(
                    release
                        .signature
                        .as_deref()
                        .ok_or("Missing archive signature")?,
                )
                .map_err(|e| e.to_string())?;
            let signature =
                ed25519_dalek::Signature::from_slice(&signature).map_err(|e| e.to_string())?;
            ed25519_dalek::VerifyingKey::from_bytes(&key)
                .map_err(|e| e.to_string())?
                .verify_strict(bytes, &signature)
                .map_err(|_| "ChatGPT archive signature verification failed".to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_numeric_versions_and_rejects_unexpected_sources() {
        assert!(newer("4.4212.0", "2.42304.0").unwrap());
        assert!(!newer("1.0", "1.0.0").unwrap());
        assert!(version_parts("../../evil").is_err());
        for url in [
            "http://persistent.oaistatic.com/codex-app-prod/x.zip",
            "https://persistent.oaistatic.com.evil.org/codex-app-prod/x.zip",
            "https://persistent.oaistatic.com/sidekick/x.zip",
        ] {
            assert!(!allowed_url(
                AppKind::Codex,
                &reqwest::Url::parse(url).unwrap()
            ));
        }
    }
    #[test]
    fn selects_compatible_full_archive_not_delta_or_wrong_arch() {
        let xml = r#"<rss xmlns:s="urn:s"><channel>
        <item><s:version>20</s:version><s:shortVersionString>2.0</s:shortVersionString><s:hardwareRequirements>x86_64</s:hardwareRequirements><enclosure url="https://persistent.oaistatic.com/codex-app-prod/wrong.zip" s:edSignature="sig"/></item>
        <item><s:version>10</s:version><s:shortVersionString>1.0</s:shortVersionString><s:hardwareRequirements>arm64</s:hardwareRequirements><s:minimumSystemVersion>13.0</s:minimumSystemVersion><enclosure url="https://persistent.oaistatic.com/codex-app-prod/full.zip" s:edSignature="sig"/><s:deltas><enclosure url="delta"/></s:deltas></item>
        </channel></rss>"#;
        assert!(codex(xml, "arm64", "26.0")
            .unwrap()
            .url
            .ends_with("full.zip"));
        assert!(codex(xml, "arm64", "12.0").is_err());
    }
    #[test]
    fn corrupt_download_is_rejected() {
        let release = Release {
            version: "1".into(),
            build: "1".into(),
            url: String::new(),
            size: None,
            sha256: Some("0".repeat(64)),
            signature: Some("bad".into()),
        };
        assert!(verify_archive(AppKind::Claude, &release, b"bad").is_err());
        assert!(verify_archive(AppKind::Codex, &release, b"bad").is_err());
    }
}
