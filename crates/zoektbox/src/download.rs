use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tar::Archive;
use tracing::info;

use crate::release::{expected_sha256, ZoektPlatform, ZOEKT_BINARIES};

/// Download the Zoekt release tarball for `platform`, verify its SHA256,
/// extract the binaries into `dest_dir`, and return paths to the extracted binaries.
pub async fn download_release(platform: ZoektPlatform, dest_dir: &Path) -> Result<Vec<PathBuf>> {
    let url = crate::release::release_url(platform);
    info!(url = %url, "zoektbox: downloading release");

    let bytes = reqwest::get(&url)
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("HTTP error for {url}"))?
        .bytes()
        .await
        .context("reading response body")?;

    verify_sha256(&bytes, expected_sha256(platform))?;
    info!(bytes = bytes.len(), "zoektbox: tarball verified");

    extract_binaries(&bytes, dest_dir)
}

/// Verify `data` matches `expected` SHA256 hex digest. Returns error with both digests on
/// mismatch.
pub fn verify_sha256(data: &[u8], expected: &str) -> Result<()> {
    let actual = hex::encode(Sha256::digest(data));
    if actual != expected {
        bail!("SHA256 mismatch: expected={expected} actual={actual}");
    }
    Ok(())
}

fn extract_binaries(tarball: &[u8], dest_dir: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dest_dir).context("create dest_dir")?;

    let gz = GzDecoder::new(tarball);
    let mut archive = Archive::new(gz);
    let mut extracted = Vec::new();

    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry.context("tar entry")?;
        let path = entry.path().context("entry path")?.into_owned();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        if ZOEKT_BINARIES.contains(&name) {
            let dest = dest_dir.join(name);
            entry
                .unpack(&dest)
                .with_context(|| format!("unpack {name}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                    .with_context(|| format!("chmod {name}"))?;
            }
            info!(binary = name, dest = %dest.display(), "zoektbox: extracted");
            extracted.push(dest);
        }
    }

    if extracted.is_empty() {
        bail!("no zoekt binaries found in tarball — check ZOEKT_BINARIES list");
    }
    Ok(extracted)
}
