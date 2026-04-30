//! `mbx upgrade` — self-update the mbx binary from GitHub releases.
//!
//! This command does NOT talk to the miniboxd daemon.  It contacts the GitHub
//! Releases API, compares the latest tag against the running binary version,
//! downloads and extracts the tarball for the current platform, and atomically
//! replaces the current executable.

use anyhow::{Context as _, Result, bail};
use std::env;

// ── Domain types ──────────────────────────────────────────────────────────────

/// Information about a GitHub release asset.
#[derive(Debug)]
pub struct ReleaseInfo {
    /// Version tag, e.g. `"v0.21.0"`.
    pub tag: String,
    /// Download URL for the tarball matching the current platform.
    pub asset_url: String,
}

/// Port: fetch release information from a remote source.
pub trait ReleaseProvider: Send + Sync {
    /// Return release info for `version` (e.g. `"v0.21.0"`), or the latest
    /// release when `version` is `None`.
    fn fetch_release(
        &self,
        version: Option<&str>,
        target_triple: &str,
    ) -> impl std::future::Future<Output = Result<ReleaseInfo>> + Send;
}

/// Port: download bytes from a URL.
pub trait AssetDownloader: Send + Sync {
    fn download(&self, url: &str) -> impl std::future::Future<Output = Result<Vec<u8>>> + Send;
}

// ── Domain logic ──────────────────────────────────────────────────────────────

/// Map (`std::env::consts::OS`, `std::env::consts::ARCH`) to a Rust target
/// triple used in release asset names.
pub fn target_triple() -> Result<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl".into()),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl".into()),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin".into()),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin".into()),
        other => bail!("unsupported platform: {}/{}", other.0, other.1),
    }
}

/// Core upgrade logic, generic over the two ports.  Extracted for unit-testing
/// without network access.
pub async fn run_upgrade<R, D>(
    dry_run: bool,
    version: Option<String>,
    current_version: &str,
    provider: &R,
    downloader: &D,
) -> Result<()>
where
    R: ReleaseProvider,
    D: AssetDownloader,
{
    let triple = target_triple().context("determine target triple")?;
    let release = provider
        .fetch_release(version.as_deref(), &triple)
        .await
        .context("fetch release info")?;

    // Normalise: strip leading 'v' for comparison
    let current = current_version.trim_start_matches('v');
    let latest = release.tag.trim_start_matches('v');

    if current == latest {
        eprintln!("mbx is already up to date ({})", current_version);
        return Ok(());
    }

    if dry_run {
        eprintln!(
            "would upgrade {} -> {} ({})",
            current_version, release.tag, release.asset_url
        );
        return Ok(());
    }

    tracing::info!(
        current = current_version,
        latest = %release.tag,
        url = %release.asset_url,
        "upgrade: downloading release"
    );
    eprintln!("upgrading {} -> {}", current_version, release.tag);

    let bytes = downloader
        .download(&release.asset_url)
        .await
        .context("download asset")?;

    let (bin_path, _tmp_guard) =
        extract_binary(&bytes, &triple).context("extract binary from tarball")?;

    let current_exe = env::current_exe().context("locate current executable")?;
    replace_binary(&current_exe, &bin_path).context("replace binary")?;

    eprintln!("upgrade complete: {} -> {}", current_version, release.tag);
    Ok(())
}

/// Extract `mbx` binary from a gzipped tarball (bytes in memory).
/// Returns `(path_to_binary, _guard)` — the `TempDir` guard must be kept alive
/// until the caller is done with the extracted file.
fn extract_binary(bytes: &[u8], _triple: &str) -> Result<(std::path::PathBuf, tempfile::TempDir)> {
    use flate2::read::GzDecoder;

    let gz = GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);
    let tmp_dir = tempfile::tempdir().context("create temp dir for extraction")?;

    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        let entry_path = entry.path().context("tar entry path")?.into_owned();

        // Accept any file whose name component is `mbx` (or `mbx.exe`)
        let name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if name == "mbx" || name == "mbx.exe" {
            let dest = tmp_dir.path().join(name);
            let mut dest_file = std::fs::File::create(&dest).context("create extraction target")?;
            std::io::copy(&mut entry, &mut dest_file).context("extract binary from tar")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = dest_file
                    .metadata()
                    .context("stat extracted binary")?
                    .permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&dest, perms).context("set executable permissions")?;
            }

            return Ok((dest, tmp_dir));
        }
    }
    bail!("mbx binary not found in release tarball")
}

/// Atomically replace `current_exe` with the new binary at `new_bin`.
fn replace_binary(current_exe: &std::path::Path, new_bin: &std::path::Path) -> Result<()> {
    // Write to a sibling temp file, then rename (atomic on POSIX).
    let parent = current_exe
        .parent()
        .context("current exe has no parent directory")?;
    let tmp_dest = parent.join(".mbx.new");
    std::fs::copy(new_bin, &tmp_dest).context("copy new binary to temp location")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_dest, std::fs::Permissions::from_mode(0o755))
            .context("set permissions on new binary")?;
    }

    std::fs::rename(&tmp_dest, current_exe).context("rename new binary over current")?;
    Ok(())
}

// ── Infrastructure adapter ────────────────────────────────────────────────────

fn build_client() -> Result<reqwest::Client> {
    let version = env!("CARGO_PKG_VERSION");
    reqwest::Client::builder()
        .user_agent(format!("mbx/{version}"))
        .build()
        .context("build HTTP client")
}

/// HTTP-backed release provider using the GitHub API.
pub struct GitHubReleaseProvider {
    client: reqwest::Client,
}

impl GitHubReleaseProvider {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: build_client()?,
        })
    }
}

impl ReleaseProvider for GitHubReleaseProvider {
    async fn fetch_release(
        &self,
        version: Option<&str>,
        target_triple: &str,
    ) -> Result<ReleaseInfo> {
        let url = match version {
            Some(v) => format!(
                "https://api.github.com/repos/89jobrien/minibox/releases/tags/{}",
                v
            ),
            None => "https://api.github.com/repos/89jobrien/minibox/releases/latest".to_string(),
        };

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("send GitHub API request")?
            .error_for_status()
            .context("GitHub API returned error status")?;

        let body: serde_json::Value = resp.json().await.context("parse GitHub API response")?;

        let tag = body["tag_name"]
            .as_str()
            .context("missing tag_name in GitHub response")?
            .to_string();

        let expected_name = format!("minibox-{}.tar.gz", target_triple);
        let asset_url = body["assets"]
            .as_array()
            .context("missing assets array")?
            .iter()
            .find(|a| a["name"].as_str() == Some(&expected_name))
            .and_then(|a| a["browser_download_url"].as_str())
            .with_context(|| format!("no asset named {} in release {}", expected_name, tag))?
            .to_string();

        Ok(ReleaseInfo { tag, asset_url })
    }
}

/// HTTP-backed asset downloader.
pub struct HttpAssetDownloader {
    client: reqwest::Client,
}

impl HttpAssetDownloader {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: build_client()?,
        })
    }
}

impl AssetDownloader for HttpAssetDownloader {
    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let bytes = self
            .client
            .get(url)
            .send()
            .await
            .context("send download request")?
            .error_for_status()
            .context("download returned error status")?
            .bytes()
            .await
            .context("read download response body")?;
        Ok(bytes.to_vec())
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Entry point called from `main.rs`.
pub async fn execute(dry_run: bool, version: Option<String>) -> Result<()> {
    let provider = GitHubReleaseProvider::new().context("create release provider")?;
    let downloader = HttpAssetDownloader::new().context("create asset downloader")?;
    let current = env!("CARGO_PKG_VERSION");
    run_upgrade(dry_run, version, current, &provider, &downloader).await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // ── CLI parse tests (exercised from main.rs test module) ──

    // ── target_triple unit tests ──────────────────────────────────────────────

    /// Override OS/ARCH via constants and verify round-trip.  We cannot set
    /// `env::consts::OS` at runtime so we test the function on the current
    /// host and verify it returns a known non-empty string.
    #[test]
    fn target_triple_returns_nonempty_string() {
        // On the CI host (macOS aarch64 or Linux x86_64) this must succeed.
        let t = target_triple();
        assert!(t.is_ok(), "target_triple failed: {:?}", t.err());
        let triple = t.unwrap();
        assert!(!triple.is_empty());
        // Must contain a known OS substring
        assert!(
            triple.contains("linux") || triple.contains("darwin"),
            "unexpected triple: {}",
            triple
        );
    }

    #[test]
    fn target_triple_known_values() {
        // We test the mapping logic by inspecting the current host.
        let os = env::consts::OS;
        let arch = env::consts::ARCH;
        let triple = target_triple().unwrap();
        match (os, arch) {
            ("linux", "x86_64") => assert_eq!(triple, "x86_64-unknown-linux-musl"),
            ("linux", "aarch64") => assert_eq!(triple, "aarch64-unknown-linux-musl"),
            ("macos", "aarch64") => assert_eq!(triple, "aarch64-apple-darwin"),
            ("macos", "x86_64") => assert_eq!(triple, "x86_64-apple-darwin"),
            _ => {} // unsupported platform — function would have errored above
        }
    }

    // ── In-memory test doubles ────────────────────────────────────────────────

    struct MockProvider {
        tag: String,
        asset_url: String,
    }

    impl ReleaseProvider for MockProvider {
        async fn fetch_release(
            &self,
            _version: Option<&str>,
            _triple: &str,
        ) -> Result<ReleaseInfo> {
            Ok(ReleaseInfo {
                tag: self.tag.clone(),
                asset_url: self.asset_url.clone(),
            })
        }
    }

    struct MockDownloader {
        bytes: Vec<u8>,
    }

    impl AssetDownloader for MockDownloader {
        async fn download(&self, _url: &str) -> Result<Vec<u8>> {
            Ok(self.bytes.clone())
        }
    }

    // ── Version comparison tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn already_up_to_date_returns_ok() {
        let provider = MockProvider {
            tag: "v1.2.3".into(),
            asset_url: "https://example.com/asset.tar.gz".into(),
        };
        let downloader = MockDownloader { bytes: vec![] };
        // Use the same version as the "latest" release
        let result = run_upgrade(false, None, "1.2.3", &provider, &downloader).await;
        assert!(result.is_ok(), "expected Ok but got: {:?}", result.err());
    }

    #[tokio::test]
    async fn dry_run_does_not_download() {
        static DOWNLOADED: Mutex<bool> = Mutex::new(false);

        struct TrackingDownloader;
        impl AssetDownloader for TrackingDownloader {
            async fn download(&self, _url: &str) -> Result<Vec<u8>> {
                *DOWNLOADED.lock().unwrap() = true;
                Ok(vec![])
            }
        }

        let provider = MockProvider {
            tag: "v9.9.9".into(),
            asset_url: "https://example.com/asset.tar.gz".into(),
        };
        let result = run_upgrade(true, None, "0.0.1", &provider, &TrackingDownloader).await;
        assert!(result.is_ok());
        assert!(!*DOWNLOADED.lock().unwrap(), "dry_run should not download");
    }
}
