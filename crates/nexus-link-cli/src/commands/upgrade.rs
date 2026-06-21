use anyhow::Context;
use serde::Deserialize;
use tracing::info;

const REPO: &str = "gwnexus/nexus-link";
const GITHUB_API: &str = "https://api.github.com";

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// Check for a newer version on GitHub Releases (used for background update notifications)
#[allow(dead_code)]
pub async fn check_update() -> anyhow::Result<Option<String>> {
    let current = env!("CARGO_PKG_VERSION");
    let client = reqwest::Client::builder()
        .user_agent(format!("nexus-link/{}", current))
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    let url = format!("{}/repos/{}/releases/latest", GITHUB_API, REPO);
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await;

    let resp = match resp {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            info!("Update check: GitHub API returned {}", r.status());
            return Ok(None);
        }
        Err(e) => {
            info!("Update check: network error ({})", e);
            return Ok(None);
        }
    };

    let release: GithubRelease = resp.json().await?;
    let latest = release.tag_name.trim_start_matches('v');

    if version_newer(latest, current) {
        Ok(Some(latest.to_string()))
    } else {
        Ok(None)
    }
}

/// Perform the self-update: download new binary and replace current
pub async fn execute(force: bool) -> anyhow::Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("Nexus Link v{}", current);
    println!();

    println!("Checking for updates...");

    let client = reqwest::Client::builder()
        .user_agent(format!("nexus-link/{}", current))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url = format!("{}/repos/{}/releases/latest", GITHUB_API, REPO);
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("Failed to reach GitHub API")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "GitHub API returned {} -- check network connectivity",
            resp.status()
        );
    }

    let release: GithubRelease = resp.json().await?;
    let latest = release.tag_name.trim_start_matches('v').to_string();

    if !force && !version_newer(&latest, current) {
        println!("Already up to date (v{}).", current);
        return Ok(());
    }

    println!("Upgrading: v{} -> v{} ...", current, latest);

    // Determine target triple
    let target = detect_target()?;
    let asset_name = format!("nexus-link-{}.tar.gz", target);

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No release asset '{}' found for your platform. \
                 Try building from source: cargo install --git https://github.com/{}.git nexus-link-cli",
                asset_name,
                REPO
            )
        })?;

    info!("Downloading {}", asset.browser_download_url);
    println!("Downloading {}...", asset_name);

    let archive_bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await?
        .bytes()
        .await
        .context("Failed to download release archive")?;

    // Extract and replace binary
    let current_exe = std::env::current_exe().context("Cannot determine current binary path")?;
    let bin_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine binary directory"))?;

    let tmp_dir = tempfile::tempdir().context("Cannot create temp directory")?;
    let archive_path = tmp_dir.path().join(&asset_name);
    std::fs::write(&archive_path, &archive_bytes)?;

    // Extract tar.gz
    let output = std::process::Command::new("tar")
        .args(["-xzf", archive_path.to_str().unwrap()])
        .current_dir(tmp_dir.path())
        .output()
        .context("Failed to extract archive (is 'tar' available?)")?;

    if !output.status.success() {
        anyhow::bail!(
            "tar extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Find and install binaries
    let binaries = ["nexus-link", "nexus-link-agent", "nexus-link-service"];
    let mut installed = 0;

    for bin_name in &binaries {
        // Try multiple extraction paths
        let candidates = [
            tmp_dir.path().join(bin_name),
            tmp_dir
                .path()
                .join(format!("nexus-link-{}", target))
                .join(bin_name),
            tmp_dir.path().join("dist").join(bin_name),
        ];

        for candidate in &candidates {
            if candidate.exists() {
                let dest = bin_dir.join(bin_name);
                // Atomic replace: rename over existing
                if dest.exists() {
                    let backup = dest.with_extension("old");
                    let _ = std::fs::remove_file(&backup);
                    std::fs::rename(&dest, &backup).ok();
                }
                std::fs::copy(candidate, &dest)
                    .with_context(|| format!("Failed to install {}", bin_name))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
                }
                // Clean up backup
                let _ = std::fs::remove_file(dest.with_extension("old"));
                installed += 1;
                break;
            }
        }
    }

    if installed == 0 {
        anyhow::bail!("No binaries found in the release archive");
    }

    println!();
    println!(
        "Upgraded to v{} ({} binary/binaries installed)",
        latest, installed
    );
    println!();

    Ok(())
}

/// Compare semver versions (returns true if `latest` > `current`)
fn version_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> (u32, u32, u32) {
        let parts: Vec<u32> = v
            .trim_start_matches('v')
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect();
        (
            parts.first().copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };

    let l = parse(latest);
    let c = parse(current);
    l > c
}

/// Detect the current platform target triple
fn detect_target() -> anyhow::Result<&'static str> {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;

    match (os, arch) {
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        _ => anyhow::bail!("Unsupported platform: {}/{}", os, arch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_newer() {
        assert!(version_newer("0.7.1", "0.7.0"));
        assert!(version_newer("0.8.0", "0.7.1"));
        assert!(version_newer("1.0.0", "0.99.99"));
        assert!(!version_newer("0.7.0", "0.7.0"));
        assert!(!version_newer("0.6.9", "0.7.0"));
        assert!(!version_newer("0.7.0", "0.7.1"));
    }

    #[test]
    fn test_version_newer_with_prefix() {
        assert!(version_newer("v0.7.1", "0.7.0"));
        assert!(version_newer("0.7.1", "v0.7.0"));
    }

    #[test]
    fn test_detect_target() {
        // Should not panic on the current platform
        let result = detect_target();
        assert!(result.is_ok());
        let target = result.unwrap();
        assert!(target.contains("linux") || target.contains("darwin") || target.contains("apple"));
    }
}
