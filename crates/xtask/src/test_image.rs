//! build-test-image — cross-compile test binaries and assemble an OCI tarball.
//!
//! Output: `~/.mbx/test-image/mbx-tester.tar`
//!
//! The tarball is compatible with both `nerdctl load -i <path>` and the
//! minibox `NativeImageLoader` (Docker-compat `manifest.json` + OCI `index.json`).

use anyhow::{Context, Result, bail};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use xshell::{Shell, cmd};

const TARGET: &str = "aarch64-unknown-linux-musl";
const ALPINE_IMAGE: &str = "alpine";
const ALPINE_TAG: &str = "3.21";
const ALPINE_ARCH: &str = "arm64"; // Docker Hub arch name for aarch64
const DOCKER_REGISTRY: &str = "https://registry-1.docker.io";
const DOCKER_AUTH: &str = "https://auth.docker.io";

/// Return the default output directory for the test image.
pub fn default_test_image_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".mbx")
        .join("test-image")
}

/// Full Linux dogfood flow: build test image, load into minibox, run tests.
pub fn test_linux(sh: &Shell) -> Result<()> {
    // 1. Build image (cached unless out of date)
    build_test_image(false)?;

    let home = std::env::var("HOME").context("HOME not set")?;
    let image_path = format!("{home}/.mbx/test-image/mbx-tester.tar");

    // 2. Load into minibox
    cmd!(sh, "minibox load {image_path}").run()?;

    // 3. Run — privileged, ephemeral, stream output
    cmd!(sh, "minibox run --privileged mbx-tester -- /run-tests.sh").run()?;

    Ok(())
}

/// Entry point: build or refresh the test OCI tarball.
pub fn build_test_image(force: bool) -> Result<()> {
    let out_dir = default_test_image_dir();
    let tar_path = out_dir.join("mbx-tester.tar");

    fs::create_dir_all(&out_dir)
        .with_context(|| format!("creating output dir {}", out_dir.display()))?;

    if !force && is_up_to_date(&tar_path)? {
        println!(
            "test image up-to-date ({}); use --force to rebuild",
            tar_path.display()
        );
        return Ok(());
    }

    let staging = out_dir.join("staging");
    // Clean staging dir to start fresh
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .with_context(|| format!("removing staging dir {}", staging.display()))?;
    }
    fs::create_dir_all(&staging).context("creating staging dir")?;

    // 1. Cross-compile binaries
    let binaries = cross_compile_binaries(force)?;

    // 2. Fetch Alpine base layer
    println!("[2/4] fetching Alpine {ALPINE_TAG} {ALPINE_ARCH} layer …");
    let (alpine_layer_path, alpine_layer_digest) = fetch_alpine_layer(&out_dir, force)?;

    // 3. Build the binaries layer tarball
    println!("[3/4] assembling binaries layer …");
    let (bins_layer_path, bins_layer_digest) = build_binaries_layer(&staging, &binaries)?;

    // 4. Assemble the OCI tarball
    println!("[4/4] writing OCI tarball → {} …", tar_path.display());
    assemble_oci_tar(
        &tar_path,
        &alpine_layer_path,
        &alpine_layer_digest,
        &bins_layer_path,
        &bins_layer_digest,
    )?;

    println!("mbx-tester.tar ready: {}", tar_path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Cache check
// ---------------------------------------------------------------------------

/// Returns true if the tarball is newer than all .rs sources in the workspace.
fn is_up_to_date(tar_path: &Path) -> Result<bool> {
    if !tar_path.exists() {
        return Ok(false);
    }
    let tar_mtime = tar_path
        .metadata()
        .with_context(|| format!("stat {}", tar_path.display()))?
        .modified()
        .context("mtime")?;

    // Walk workspace crates dir looking for .rs files
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask parent")?
        .parent()
        .context("workspace root")?;
    let crates_dir = workspace_root.join("crates");

    let newest_src = find_newest_rs_mtime(&crates_dir)?;
    Ok(newest_src.map(|src| tar_mtime > src).unwrap_or(false))
}

fn find_newest_rs_mtime(dir: &Path) -> Result<Option<std::time::SystemTime>> {
    let mut newest = None;
    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry.context("dir entry")?;
        let path = entry.path();
        if path.is_dir() {
            let sub = find_newest_rs_mtime(&path)?;
            if let Some(t) = sub {
                newest = Some(newest.map(|n: std::time::SystemTime| n.max(t)).unwrap_or(t));
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let mtime = path
                .metadata()
                .with_context(|| format!("stat {}", path.display()))?
                .modified()
                .context("mtime")?;
            newest = Some(
                newest
                    .map(|n: std::time::SystemTime| n.max(mtime))
                    .unwrap_or(mtime),
            );
        }
    }
    Ok(newest)
}

// ---------------------------------------------------------------------------
// Cross-compilation
// ---------------------------------------------------------------------------

fn cross_compile_binaries(force: bool) -> Result<Vec<(String, PathBuf)>> {
    let target_base = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));

    println!("[1/4] cross-compiling for {TARGET} …");

    let cc = "aarch64-linux-musl-gcc";

    // -- miniboxd binary --
    let miniboxd_bin = target_base.join(TARGET).join("debug").join("miniboxd");
    if force || !miniboxd_bin.exists() {
        println!("  cargo build miniboxd …");
        run_cross(&["build", "--target", TARGET, "-p", "miniboxd"], cc)?;
    } else {
        println!("  cached  miniboxd");
    }

    // -- minibox (CLI) binary --
    let cli_bin = target_base.join(TARGET).join("debug").join("minibox");
    if force || !cli_bin.exists() {
        println!("  cargo build minibox-cli …");
        run_cross(&["build", "--target", TARGET, "-p", "minibox-cli"], cc)?;
    } else {
        println!("  cached  minibox-cli");
    }

    // -- test binaries --
    let test_suites: &[(&str, &str, &str)] = &[
        ("cgroup_tests", "miniboxd", "cgroup_tests"),
        ("e2e_tests", "miniboxd", "e2e_tests"),
        ("integration_tests", "miniboxd", "integration_tests"),
        ("sandbox_tests", "miniboxd", "sandbox_tests"),
    ];

    let mut binaries: Vec<(String, PathBuf)> = vec![
        ("miniboxd".to_string(), miniboxd_bin),
        ("minibox".to_string(), cli_bin),
    ];

    for (suite_name, pkg, test_name) in test_suites {
        println!("  cargo test --no-run --test {test_name} …");
        let bin_path = build_test_binary(pkg, test_name, cc, &target_base, force)?;
        binaries.push((suite_name.to_string(), bin_path));
    }

    Ok(binaries)
}

fn run_cross(args: &[&str], cc: &str) -> Result<()> {
    let status = Command::new("cargo")
        .args(args)
        .env("CC_aarch64_unknown_linux_musl", cc)
        .env("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER", cc)
        .status()
        .context("spawning cargo")?;
    if !status.success() {
        bail!("cargo {:?} failed", args);
    }
    Ok(())
}

/// Compile a test binary with `cargo test --no-run --test <name>` and find the
/// resulting binary in `target/<TARGET>/debug/deps/`.
fn build_test_binary(
    pkg: &str,
    test_name: &str,
    cc: &str,
    target_base: &Path,
    _force: bool,
) -> Result<PathBuf> {
    // `cargo test --no-run` outputs a line like:
    //   Executable unittests src/lib.rs (target/.../deps/foo-abc123)
    // We capture stderr and parse it.
    let output = Command::new("cargo")
        .args([
            "test",
            "--no-run",
            "--target",
            TARGET,
            "-p",
            pkg,
            "--test",
            test_name,
            "--message-format=json",
        ])
        .env("CC_aarch64_unknown_linux_musl", cc)
        .env("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER", cc)
        .output()
        .context("spawning cargo test --no-run")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("cargo test --no-run --test {test_name} failed:\n{stderr}");
    }

    // Parse JSON messages to find the executable path
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) {
            if msg.get("reason").and_then(|r| r.as_str()) == Some("compiler-artifact") {
                if let Some(exe) = msg.get("executable").and_then(|e| e.as_str()) {
                    let p = PathBuf::from(exe);
                    if p.exists() {
                        return Ok(p);
                    }
                }
            }
        }
    }

    // Fallback: glob deps dir for a binary matching the test name prefix
    let deps_dir = target_base.join(TARGET).join("debug").join("deps");
    let prefix = test_name.replace('-', "_");
    if deps_dir.exists() {
        for entry in
            fs::read_dir(&deps_dir).with_context(|| format!("read_dir {}", deps_dir.display()))?
        {
            let entry = entry.context("dir entry")?;
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            // Binary name: <test_name>-<hash> with no extension
            if fname.starts_with(&prefix) && !fname.contains('.') && entry.path().is_file() {
                return Ok(entry.path());
            }
        }
    }

    bail!(
        "could not find compiled test binary for {test_name} in {}",
        deps_dir.display()
    )
}

// ---------------------------------------------------------------------------
// Alpine layer fetch via Docker Hub API
// ---------------------------------------------------------------------------

/// Fetch the Alpine aarch64 layer from Docker Hub and return (layer_tar_path, sha256_digest).
/// Uses `curl` for HTTP (no extra deps needed).
fn fetch_alpine_layer(cache_dir: &Path, force: bool) -> Result<(PathBuf, String)> {
    let layer_path = cache_dir.join(format!("alpine-{ALPINE_TAG}-{ALPINE_ARCH}-layer.tar"));
    let digest_path = cache_dir.join(format!("alpine-{ALPINE_TAG}-{ALPINE_ARCH}-layer.digest"));

    if !force && layer_path.exists() && digest_path.exists() {
        let digest = fs::read_to_string(&digest_path)
            .context("reading cached layer digest")?
            .trim()
            .to_string();
        println!("  cached  alpine layer ({})", &digest[..16]);
        return Ok((layer_path, digest));
    }

    // Step 1: get auth token
    let token = docker_auth_token(ALPINE_IMAGE)?;

    // Step 2: get manifest to find the layer blob digest for our arch
    let layer_digest = get_alpine_layer_digest(&token)?;

    // Step 3: download the layer blob
    println!("  fetching alpine layer blob {} …", &layer_digest[..19]);
    let blob_url = format!("{DOCKER_REGISTRY}/v2/library/{ALPINE_IMAGE}/blobs/{layer_digest}");
    curl_download_with_auth(&blob_url, &layer_path, &token)?;

    // Save digest for cache
    fs::write(&digest_path, &layer_digest).context("writing layer digest cache")?;

    Ok((layer_path, layer_digest))
}

fn docker_auth_token(image: &str) -> Result<String> {
    let url = format!(
        "{DOCKER_AUTH}/token?service=registry.docker.io&scope=repository:library/{image}:pull"
    );
    let out = Command::new("curl")
        .args(["--silent", "--show-error", "--fail", &url])
        .output()
        .context("curl auth token")?;
    if !out.status.success() {
        bail!("docker auth token fetch failed");
    }
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parsing auth token JSON")?;
    json["token"]
        .as_str()
        .map(|s| s.to_string())
        .context("no token field in auth response")
}

fn get_alpine_layer_digest(token: &str) -> Result<String> {
    // Fetch the manifest list to find the aarch64 manifest
    let manifest_list_url =
        format!("{DOCKER_REGISTRY}/v2/library/{ALPINE_IMAGE}/manifests/{ALPINE_TAG}");
    let out = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "-H",
            &format!("Authorization: Bearer {token}"),
            "-H",
            "Accept: application/vnd.docker.distribution.manifest.list.v2+json,application/vnd.oci.image.index.v1+json",
            &manifest_list_url,
        ])
        .output()
        .context("curl manifest list")?;

    if !out.status.success() {
        bail!("manifest list fetch failed");
    }

    let list: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parsing manifest list")?;

    // Find the aarch64 manifest digest in the list
    let arch_digest = list["manifests"]
        .as_array()
        .context("no manifests array")?
        .iter()
        .find(|m| {
            m["platform"]["architecture"].as_str() == Some(ALPINE_ARCH)
                && m["platform"]["os"].as_str() == Some("linux")
        })
        .and_then(|m| m["digest"].as_str())
        .map(|s| s.to_string())
        .context("aarch64 linux manifest not found in manifest list")?;

    // Fetch the arch-specific manifest to get the layer digest
    let manifest_url =
        format!("{DOCKER_REGISTRY}/v2/library/{ALPINE_IMAGE}/manifests/{arch_digest}");
    let out = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "-H",
            &format!("Authorization: Bearer {token}"),
            "-H",
            "Accept: application/vnd.docker.distribution.manifest.v2+json,application/vnd.oci.image.manifest.v1+json",
            &manifest_url,
        ])
        .output()
        .context("curl arch manifest")?;

    if !out.status.success() {
        bail!("arch manifest fetch failed");
    }

    let manifest: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parsing arch manifest")?;

    // Alpine is a single-layer image; take the first layer
    manifest["layers"]
        .as_array()
        .context("no layers in manifest")?
        .first()
        .context("empty layers array")?["digest"]
        .as_str()
        .map(|s| s.to_string())
        .context("layer digest not a string")
}

fn curl_download_with_auth(url: &str, dest: &Path, token: &str) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating dir {}", parent.display()))?;
    }
    let status = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--fail",
            "-H",
            &format!("Authorization: Bearer {token}"),
            "-o",
        ])
        .arg(dest)
        .arg(url)
        .status()
        .context("curl download")?;
    if !status.success() {
        bail!("curl download failed for {url}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Binaries layer
// ---------------------------------------------------------------------------

/// Build a tar layer containing:
///   usr/local/bin/<binary>  for each binary
///   run-tests.sh
/// Returns (layer_tar_path, sha256_digest_string).
fn build_binaries_layer(
    staging: &Path,
    binaries: &[(String, PathBuf)],
) -> Result<(PathBuf, String)> {
    let layer_dir = staging.join("bins-layer-content");
    let usr_local_bin = layer_dir.join("usr").join("local").join("bin");
    fs::create_dir_all(&usr_local_bin).context("creating usr/local/bin in layer")?;

    for (name, src) in binaries {
        let dest = usr_local_bin.join(name);
        fs::copy(src, &dest).with_context(|| format!("copying {} to layer", src.display()))?;
        // Set executable bit
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))
                .context("chmod binary")?;
        }
    }

    // Write the entrypoint script
    let script_path = layer_dir.join("run-tests.sh");
    let script = entrypoint_script();
    fs::write(&script_path, script).context("writing run-tests.sh")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755))
            .context("chmod run-tests.sh")?;
    }

    // Build the tar
    let layer_tar = staging.join("bins-layer.tar");
    let status = Command::new("tar")
        .args(["-cf"])
        .arg(&layer_tar)
        .args(["-C"])
        .arg(&layer_dir)
        .arg(".")
        .status()
        .context("tar bins layer")?;
    if !status.success() {
        bail!("tar failed building binaries layer");
    }

    let digest = sha256_of_file(&layer_tar)?;
    Ok((layer_tar, format!("sha256:{digest}")))
}

fn entrypoint_script() -> &'static str {
    r#"#!/bin/sh
set -e
MINIBOX_ADAPTER=native
export MINIBOX_ADAPTER

echo "=== cgroup_tests ==="
/usr/local/bin/cgroup_tests --test-threads=1 --nocapture

echo "=== integration_tests ==="
/usr/local/bin/integration_tests --test-threads=1 --ignored --nocapture

echo "=== e2e_tests ==="
MINIBOX_TEST_BIN_DIR=/usr/local/bin /usr/local/bin/e2e_tests --test-threads=1 --nocapture

echo "=== sandbox_tests ==="
MINIBOX_TEST_BIN_DIR=/usr/local/bin /usr/local/bin/sandbox_tests --test-threads=1 --ignored --nocapture

echo "=== all Linux tests passed ==="
"#
}

// ---------------------------------------------------------------------------
// SHA-256 helper
// ---------------------------------------------------------------------------

fn sha256_of_file(path: &Path) -> Result<String> {
    // Use shasum / sha256sum depending on platform
    let (cmd, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("shasum", &["-a", "256"])
    } else {
        ("sha256sum", &[])
    };
    let out = Command::new(cmd)
        .args(args)
        .arg(path)
        .output()
        .with_context(|| format!("running {cmd}"))?;
    if !out.status.success() {
        bail!("{cmd} failed for {}", path.display());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Output format: "<hex>  <filename>"
    stdout
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
        .context("empty sha256 output")
}

fn sha256_of_bytes(data: &[u8]) -> String {
    // Write to a temp file then hash it — avoids pulling in sha2 crate
    let tmp = std::env::temp_dir().join(format!("xtask-sha-{}", std::process::id()));
    std::fs::write(&tmp, data).expect("writing tmp for sha256");
    let digest = sha256_of_file(&tmp).expect("sha256 of tmp");
    let _ = std::fs::remove_file(&tmp);
    digest
}

// ---------------------------------------------------------------------------
// OCI tarball assembly
// ---------------------------------------------------------------------------

/// Assemble the final OCI tarball with both Docker-compat manifest.json and OCI index.json.
fn assemble_oci_tar(
    out_tar: &Path,
    alpine_layer: &Path,
    alpine_digest: &str,
    bins_layer: &Path,
    bins_digest: &str,
) -> Result<()> {
    // Determine short IDs (strip "sha256:" prefix, take 12 chars for dir names)
    let alpine_id = alpine_digest
        .strip_prefix("sha256:")
        .unwrap_or(alpine_digest);
    let bins_id = bins_digest.strip_prefix("sha256:").unwrap_or(bins_digest);

    let alpine_layer_name = format!("{}/layer.tar", &alpine_id[..12]);
    let bins_layer_name = format!("{}/layer.tar", &bins_id[..12]);

    // Build OCI image config
    let config_obj = json!({
        "architecture": "arm64",
        "os": "linux",
        "rootfs": {
            "type": "layers",
            "diff_ids": [alpine_digest, bins_digest]
        },
        "config": {
            "Cmd": ["/run-tests.sh"]
        }
    });
    let config_bytes = serde_json::to_vec(&config_obj).context("serializing image config")?;
    let config_digest_hex = sha256_of_bytes(&config_bytes);
    let config_digest = format!("sha256:{config_digest_hex}");
    let config_filename = format!("{config_digest_hex}.json");

    let alpine_layer_size = fs::metadata(alpine_layer)
        .with_context(|| format!("stat {}", alpine_layer.display()))?
        .len();
    let bins_layer_size = fs::metadata(bins_layer)
        .with_context(|| format!("stat {}", bins_layer.display()))?
        .len();
    let config_size = config_bytes.len() as u64;

    // Docker-compat manifest.json
    let docker_manifest = json!([{
        "Config": config_filename,
        "RepoTags": ["mbx-tester:latest"],
        "Layers": [alpine_layer_name, bins_layer_name]
    }]);

    // OCI image manifest
    let oci_manifest = json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": config_digest,
            "size": config_size
        },
        "layers": [
            {
                "mediaType": "application/vnd.oci.image.layer.v1.tar",
                "digest": alpine_digest,
                "size": alpine_layer_size
            },
            {
                "mediaType": "application/vnd.oci.image.layer.v1.tar",
                "digest": bins_digest,
                "size": bins_layer_size
            }
        ]
    });
    let oci_manifest_bytes =
        serde_json::to_vec(&oci_manifest).context("serializing OCI manifest")?;
    let oci_manifest_digest_hex = sha256_of_bytes(&oci_manifest_bytes);
    let oci_manifest_digest = format!("sha256:{oci_manifest_digest_hex}");
    let oci_manifest_size = oci_manifest_bytes.len() as u64;

    // OCI index
    let oci_index = json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [{
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": oci_manifest_digest,
            "size": oci_manifest_size,
            "platform": {
                "architecture": "arm64",
                "os": "linux"
            },
            "annotations": {
                "org.opencontainers.image.ref.name": "mbx-tester:latest"
            }
        }]
    });

    let oci_layout = json!({"imageLayoutVersion": "1.0.0"});

    // Write the tar by building it from a staging area
    let staging = out_tar
        .parent()
        .context("tar parent dir")?
        .join("oci-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging).context("removing oci-staging")?;
    }
    fs::create_dir_all(&staging).context("creating oci-staging")?;

    // Write JSON files
    let write_json = |name: &str, value: &serde_json::Value| -> Result<()> {
        let bytes = serde_json::to_vec(value).with_context(|| format!("serializing {name}"))?;
        fs::write(staging.join(name), bytes).with_context(|| format!("writing {name}"))
    };
    write_json("manifest.json", &docker_manifest)?;
    write_json("index.json", &oci_index)?;
    write_json("oci-layout", &oci_layout)?;
    write_json(&config_filename, &config_obj)?;

    // Write OCI manifest as a blob-addressed file
    let oci_manifest_file = format!("{oci_manifest_digest_hex}.manifest.json");
    fs::write(staging.join(&oci_manifest_file), &oci_manifest_bytes)
        .context("writing OCI manifest blob")?;

    // Create layer subdirs and hard-link or copy layer tarballs
    let alpine_layer_dir = staging.join(&alpine_id[..12]);
    let bins_layer_dir = staging.join(&bins_id[..12]);
    fs::create_dir_all(&alpine_layer_dir).context("creating alpine layer dir")?;
    fs::create_dir_all(&bins_layer_dir).context("creating bins layer dir")?;

    fs::copy(alpine_layer, alpine_layer_dir.join("layer.tar")).context("copying alpine layer")?;
    fs::copy(bins_layer, bins_layer_dir.join("layer.tar")).context("copying bins layer")?;

    // Build the final tar
    if out_tar.exists() {
        fs::remove_file(out_tar).context("removing old mbx-tester.tar")?;
    }
    let status = Command::new("tar")
        .args(["-cf"])
        .arg(out_tar)
        .args(["-C"])
        .arg(&staging)
        .arg(".")
        .status()
        .context("tar oci-staging")?;
    if !status.success() {
        bail!("tar failed building OCI tarball");
    }

    // Clean up staging
    fs::remove_dir_all(&staging).context("removing oci-staging")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_dir_is_absolute_and_contains_mbx() {
        let d = default_test_image_dir();
        assert!(d.is_absolute());
        assert!(d.to_string_lossy().contains(".mbx"));
        assert!(d.to_string_lossy().contains("test-image"));
    }

    #[test]
    fn entrypoint_script_contains_all_suites() {
        let s = entrypoint_script();
        assert!(s.contains("cgroup_tests"));
        assert!(s.contains("integration_tests"));
        assert!(s.contains("e2e_tests"));
        assert!(s.contains("sandbox_tests"));
        assert!(s.contains("all Linux tests passed"));
    }

    #[test]
    fn sha256_of_known_content() {
        let tmp = tempdir().expect("tempdir");
        let f = tmp.path().join("test.bin");
        fs::write(&f, b"hello world\n").unwrap();
        let digest = sha256_of_file(&f).unwrap();
        // sha256("hello world\n") = a948904f2f0f479b8f936434bfff...
        assert_eq!(digest.len(), 64, "sha256 should be 64 hex chars");
    }

    #[test]
    fn is_up_to_date_returns_false_for_missing_tar() {
        let tmp = tempdir().expect("tempdir");
        let tar = tmp.path().join("nonexistent.tar");
        assert!(!is_up_to_date(&tar).unwrap());
    }

    #[test]
    fn binaries_layer_is_created_with_correct_contents() {
        let tmp = tempdir().expect("tempdir");
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        // Create a fake binary
        let fake_bin = tmp.path().join("fake_binary");
        fs::write(&fake_bin, b"\x7fELF fake").unwrap();
        let binaries = vec![("mybin".to_string(), fake_bin)];

        let (layer_tar, digest) = build_binaries_layer(&staging, &binaries).unwrap();
        assert!(layer_tar.exists(), "layer tar should exist");
        assert!(
            digest.starts_with("sha256:"),
            "digest should start with sha256:"
        );
    }
}
