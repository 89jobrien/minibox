# searchbox + zoektbox Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `zoektbox` (Zoekt binary lifecycle) and `searchbox` (search port + MCP stdio
server) as two new minibox workspace crates.

**Architecture:** `zoektbox` owns download/verify/deploy of pinned Zoekt binaries to the VPS
via SSH; `searchbox` owns the `SearchProvider` port, adapters (Zoekt HTTP, merged fan-out, git
mirror, filesystem rsync, local sidecar), and a `searchboxd` binary that serves as a local MCP
stdio server. `searchbox` depends on `zoektbox`; `zoektbox` has zero knowledge of search.

**Tech Stack:** Rust 2024 edition, `async-trait`, `reqwest` (HTTP), `tokio` (async), `sha2` +
`hex` + `flate2` + `tar` (binary verification/extraction), `clap` (CLI), `serde`/`serde_json`
(config + MCP wire), `futures::future::join_all` (fan-out). All deps are existing workspace deps.

**Pre-flight:** Run from inside the minibox workspace root. Confirm `cargo check --workspace`
is clean before starting.

---

## File Map

### `crates/zoektbox/`

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest; workspace deps only |
| `src/lib.rs` | Re-exports: `release`, `download`, `deploy`, `service` |
| `src/release.rs` | `ZOEKT_VERSION`, `ZoektPlatform`, `release_url()`, `expected_sha256()` |
| `src/download.rs` | `download_release()` — fetch tarball, verify SHA256, extract to staging dir |
| `src/deploy.rs` | `deploy_binaries()` — rsync staging dir to VPS via SSH |
| `src/service.rs` | `ZoektServiceAdapter` — impl `searchbox::domain::ServiceManager` |
| `tests/unit.rs` | SHA256 verify, URL construction, version parse, staging extraction |

### `crates/searchbox/`

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Crate manifest; depends on `zoektbox` |
| `src/lib.rs` | Re-exports: `domain`, `config`, `adapters` |
| `src/domain.rs` | `SearchProvider`, `IndexSource`, `ServiceManager` traits; all domain types + errors |
| `src/config.rs` | `SearchboxConfig`, `RepoConfig`, `LocalConfig` — serde TOML |
| `src/mcp.rs` | JSON-RPC 2.0 stdio loop; tool dispatch (`search`, `list_repos`, `reindex`, `service_status`) |
| `src/adapters/zoekt.rs` | `ZoektAdapter` — reqwest client to `zoekt-webserver` |
| `src/adapters/merged.rs` | `MergedAdapter` — fan-out + dedup |
| `src/adapters/git_source.rs` | `GitRepoSource` — git mirror + `zoekt-git-index` |
| `src/adapters/fs_source.rs` | `FilesystemSource` — glob expand + rsync + `zoekt-index` |
| `src/adapters/local.rs` | `LocalZoektSource` — localhost sidecar |
| `src/adapters/mock.rs` | `MockSearchProvider` — test double |
| `bin/searchboxd.rs` | Composition root; clap subcommands: `mcp`, `reindex`, `status`, `provision` |
| `tests/unit.rs` | `MergedAdapter` dedup/merge, config parse, domain invariants |
| `tests/integration.rs` | `ZoektAdapter` against live `zoekt-webserver` (feature `integration-tests`) |

---

## Task 1: `zoektbox` crate scaffold + release manifest

**Files:**
- Create: `crates/zoektbox/Cargo.toml`
- Create: `crates/zoektbox/src/lib.rs`
- Create: `crates/zoektbox/src/release.rs`
- Create: `crates/zoektbox/tests/unit.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] Add `crates/zoektbox` to workspace `members` in root `Cargo.toml`:

```toml
# in [workspace] members = [ ... ] add:
"crates/zoektbox",
```

- [ ] Write `crates/zoektbox/Cargo.toml`:

```toml
[package]
name = "zoektbox"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
anyhow = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
tokio = { workspace = true }
sha2 = { workspace = true }
hex = { workspace = true }
flate2 = { workspace = true }
tar = { workspace = true }
reqwest = { workspace = true }
tempfile = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["full"] }
```

- [ ] Write `crates/zoektbox/src/release.rs`:

```rust
//! Pinned Zoekt release manifest. Update `ZOEKT_VERSION` and `CHECKSUMS` on each upgrade.

pub const ZOEKT_VERSION: &str = "3.7.2-89.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoektPlatform {
    LinuxAmd64,
    LinuxArm64,
    DarwinArm64,
}

impl ZoektPlatform {
    pub fn detect() -> Self {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Self::LinuxAmd64,
            ("linux", "aarch64") => Self::LinuxArm64,
            ("macos", "aarch64") => Self::DarwinArm64,
            (os, arch) => panic!("unsupported platform: {os}/{arch}"),
        }
    }

    fn triple(&self) -> &'static str {
        match self {
            Self::LinuxAmd64 => "linux_amd64",
            Self::LinuxArm64 => "linux_arm64",
            Self::DarwinArm64 => "darwin_arm64",
        }
    }
}

/// GitHub release tarball URL for the given platform.
pub fn release_url(platform: ZoektPlatform) -> String {
    format!(
        "https://github.com/sourcegraph/zoekt/releases/download/v{version}/zoekt_{version}_{triple}.tar.gz",
        version = ZOEKT_VERSION,
        triple = platform.triple(),
    )
}

/// Expected SHA256 hex digest for each platform's tarball.
/// Run `sha256sum <tarball>` after downloading to verify and update these.
pub fn expected_sha256(platform: ZoektPlatform) -> &'static str {
    match platform {
        // TODO: fill in after first download — run `sha256sum` on each tarball
        ZoektPlatform::LinuxAmd64  => "0000000000000000000000000000000000000000000000000000000000000000",
        ZoektPlatform::LinuxArm64  => "0000000000000000000000000000000000000000000000000000000000000000",
        ZoektPlatform::DarwinArm64 => "0000000000000000000000000000000000000000000000000000000000000000",
    }
}

/// Names of binaries extracted from the tarball.
pub const ZOEKT_BINARIES: &[&str] = &[
    "zoekt-webserver",
    "zoekt-indexserver",
    "zoekt-git-index",
    "zoekt-index",
];
```

- [ ] Write `crates/zoektbox/src/lib.rs`:

```rust
pub mod deploy;
pub mod download;
pub mod release;
pub mod service;

pub use release::{ZoektPlatform, ZOEKT_BINARIES, ZOEKT_VERSION};
pub use service::ZoektServiceAdapter;
```

- [ ] Write the failing test in `crates/zoektbox/tests/unit.rs`:

```rust
use zoektbox::release::{ZoektPlatform, expected_sha256, release_url};

#[test]
fn release_url_linux_amd64_contains_version_and_triple() {
    let url = release_url(ZoektPlatform::LinuxAmd64);
    assert!(url.contains("linux_amd64"), "url: {url}");
    assert!(url.contains(zoektbox::ZOEKT_VERSION), "url: {url}");
    assert!(url.starts_with("https://github.com/sourcegraph/zoekt/releases/"), "url: {url}");
}

#[test]
fn release_url_differs_per_platform() {
    let a = release_url(ZoektPlatform::LinuxAmd64);
    let b = release_url(ZoektPlatform::LinuxArm64);
    let c = release_url(ZoektPlatform::DarwinArm64);
    assert_ne!(a, b);
    assert_ne!(b, c);
}

#[test]
fn expected_sha256_is_64_hex_chars() {
    for platform in [ZoektPlatform::LinuxAmd64, ZoektPlatform::LinuxArm64, ZoektPlatform::DarwinArm64] {
        let digest = expected_sha256(platform);
        assert_eq!(digest.len(), 64, "platform {platform:?}: len={}", digest.len());
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()), "not hex: {digest}");
    }
}
```

- [ ] Run: `cargo test -p zoektbox 2>&1 | tail -20`
  Expected: `release_url_*` tests pass; `expected_sha256_is_64_hex_chars` passes (zeros are valid hex).

- [ ] Commit:

```bash
git add crates/zoektbox/ Cargo.toml
git commit -m "feat(zoektbox): scaffold crate with release manifest"
```

---

## Task 2: `zoektbox` — download + SHA256 verify

**Files:**
- Create: `crates/zoektbox/src/download.rs`
- Modify: `crates/zoektbox/tests/unit.rs`

- [ ] Write `crates/zoektbox/src/download.rs`:

```rust
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

/// Verify `data` matches `expected` SHA256 hex digest. Returns error with both digests on mismatch.
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
            entry.unpack(&dest).with_context(|| format!("unpack {name}"))?;
            // make executable
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
```

- [ ] Add tests to `crates/zoektbox/tests/unit.rs`:

```rust
use zoektbox::download::verify_sha256;

#[test]
fn verify_sha256_passes_on_correct_digest() {
    let data = b"hello zoekt";
    let digest = hex::encode(sha2::Sha256::digest(data));
    verify_sha256(data, &digest).expect("should pass");
}

#[test]
fn verify_sha256_fails_on_wrong_digest() {
    let data = b"hello zoekt";
    let err = verify_sha256(data, "deadbeef00000000000000000000000000000000000000000000000000000000");
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("SHA256 mismatch"), "msg: {msg}");
}
```

Add to `[dev-dependencies]` in `crates/zoektbox/Cargo.toml`:
```toml
sha2 = { workspace = true }
hex = { workspace = true }
```

- [ ] Run: `cargo test -p zoektbox 2>&1 | tail -20`
  Expected: all 5 tests pass.

- [ ] Commit:

```bash
git add crates/zoektbox/src/download.rs crates/zoektbox/tests/unit.rs crates/zoektbox/Cargo.toml
git commit -m "feat(zoektbox): download + SHA256 verify"
```

---

## Task 3: `zoektbox` — deploy + service stubs

**Files:**
- Create: `crates/zoektbox/src/deploy.rs`
- Create: `crates/zoektbox/src/service.rs`

- [ ] Write `crates/zoektbox/src/deploy.rs`:

```rust
use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

/// Rsync binaries from `src_dir` to `dest_host:dest_path` via SSH.
/// `ssh_host` is a Tailscale alias (e.g. "minibox").
pub async fn deploy_binaries(ssh_host: &str, src_dir: &Path, dest_path: &str) -> Result<()> {
    info!(host = ssh_host, src = %src_dir.display(), dest = dest_path, "zoektbox: deploying binaries");

    let status = tokio::process::Command::new("rsync")
        .args([
            "-avz",
            "--chmod=755",
            &format!("{}/", src_dir.display()),
            &format!("{ssh_host}:{dest_path}/"),
        ])
        .status()
        .await
        .context("rsync")?;

    if !status.success() {
        anyhow::bail!("rsync exited with {status}");
    }
    info!(host = ssh_host, "zoektbox: deploy complete");
    Ok(())
}

/// Run a command on the remote host via SSH, returning stdout as a String.
pub async fn ssh_run(ssh_host: &str, cmd: &str) -> Result<String> {
    let out = tokio::process::Command::new("ssh")
        .args([ssh_host, cmd])
        .output()
        .await
        .with_context(|| format!("ssh {ssh_host} {cmd}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("ssh command failed: {stderr}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
```

- [ ] Write `crates/zoektbox/src/service.rs`:

```rust
//! ZoektServiceAdapter — implements searchbox::domain::ServiceManager.
//! Manages zoekt-indexserver + zoekt-webserver on a remote VPS via SSH.

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::info;

use crate::deploy::ssh_run;

/// Configuration for the remote Zoekt service.
#[derive(Debug, Clone)]
pub struct ZoektServiceConfig {
    /// Tailscale SSH host alias.
    pub ssh_host: String,
    /// Port for zoekt-webserver (default 6070).
    pub port: u16,
    /// Remote path where binaries and index live.
    pub remote_base: String,
}

impl Default for ZoektServiceConfig {
    fn default() -> Self {
        Self {
            ssh_host: "minibox".into(),
            port: 6070,
            remote_base: "/opt/zoekt".into(),
        }
    }
}

pub struct ZoektServiceAdapter {
    pub config: ZoektServiceConfig,
}

impl ZoektServiceAdapter {
    pub fn new(config: ZoektServiceConfig) -> Self {
        Self { config }
    }

    /// Download, verify, and rsync Zoekt binaries to the VPS.
    /// Must be called once before `start()`.
    pub async fn provision(&self) -> Result<()> {
        let tmp = tempfile::tempdir().context("tempdir")?;
        crate::download::download_release(
            crate::release::ZoektPlatform::LinuxAmd64,
            tmp.path(),
        )
        .await?;
        crate::deploy::deploy_binaries(
            &self.config.ssh_host,
            tmp.path(),
            &format!("{}/bin", self.config.remote_base),
        )
        .await?;
        // Create index dir on VPS
        ssh_run(
            &self.config.ssh_host,
            &format!("mkdir -p {}/index", self.config.remote_base),
        )
        .await?;
        info!(host = %self.config.ssh_host, "zoektbox: provision complete");
        Ok(())
    }

    fn index_dir(&self) -> String {
        format!("{}/index", self.config.remote_base)
    }

    fn bin(&self, name: &str) -> String {
        format!("{}/bin/{name}", self.config.remote_base)
    }
}

// ServiceManager is defined in searchbox::domain and imported here.
// The trait impl lives in searchbox's dependency graph, so we use a re-export shim
// in searchbox/src/adapters/service_bridge.rs. See Task 8.
//
// For now, expose the async methods directly so searchboxd can call them.
impl ZoektServiceAdapter {
    pub async fn start(&self) -> Result<()> {
        let index = self.index_dir();
        let webserver = self.bin("zoekt-webserver");
        let indexserver = self.bin("zoekt-indexserver");

        // Start indexserver (daemonised via nohup)
        ssh_run(
            &self.config.ssh_host,
            &format!("nohup {indexserver} -index {index} </dev/null >/opt/zoekt/indexserver.log 2>&1 &"),
        ).await.context("start indexserver")?;

        // Start webserver bound to Tailscale IP only (not 0.0.0.0)
        let ts_ip = self.tailscale_ip().await?;
        ssh_run(
            &self.config.ssh_host,
            &format!("nohup {webserver} -index {index} -listen {ts_ip}:{} </dev/null >/opt/zoekt/webserver.log 2>&1 &",
                self.config.port),
        ).await.context("start webserver")?;

        info!(host = %self.config.ssh_host, port = self.config.port, "zoektbox: started");
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        ssh_run(&self.config.ssh_host, "pkill -f zoekt-webserver; pkill -f zoekt-indexserver; true")
            .await
            .context("stop zoekt")?;
        info!(host = %self.config.ssh_host, "zoektbox: stopped");
        Ok(())
    }

    pub async fn status(&self) -> Result<bool> {
        let url = format!("http://{}:{}/healthz", self.config.ssh_host, self.config.port);
        match reqwest::get(&url).await {
            Ok(r) => Ok(r.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    pub async fn reindex(&self, repo: Option<&str>) -> Result<()> {
        let index = self.index_dir();
        let git_index = self.bin("zoekt-git-index");
        let cmd = match repo {
            Some(r) => format!("{git_index} -index {index} {index}/{r}.git"),
            None => format!(
                "for d in {index}/*.git; do {git_index} -index {index} \"$d\"; done"
            ),
        };
        ssh_run(&self.config.ssh_host, &cmd)
            .await
            .context("reindex")?;
        Ok(())
    }

    async fn tailscale_ip(&self) -> Result<String> {
        let out = ssh_run(&self.config.ssh_host, "tailscale ip -4")
            .await
            .context("get tailscale IP")?;
        Ok(out.trim().to_string())
    }
}
```

- [ ] Run: `cargo check -p zoektbox 2>&1 | tail -20`
  Expected: clean (no errors).

- [ ] Commit:

```bash
git add crates/zoektbox/src/deploy.rs crates/zoektbox/src/service.rs
git commit -m "feat(zoektbox): deploy + service adapter"
```

---

## Task 4: `searchbox` crate scaffold + domain types

**Files:**
- Create: `crates/searchbox/Cargo.toml`
- Create: `crates/searchbox/src/lib.rs`
- Create: `crates/searchbox/src/domain.rs`
- Create: `crates/searchbox/tests/unit.rs`
- Modify: `Cargo.toml` (workspace members + internal dep)

- [ ] Add to workspace `Cargo.toml`:

In `[workspace] members`:
```toml
"crates/searchbox",
```

In `[workspace.dependencies]`:
```toml
zoektbox = { path = "crates/zoektbox" }
searchbox = { path = "crates/searchbox" }
```

- [ ] Write `crates/searchbox/Cargo.toml`:

```toml
[package]
name = "searchbox"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
zoektbox    = { workspace = true }
anyhow      = { workspace = true }
thiserror   = { workspace = true }
tracing     = { workspace = true }
tokio       = { workspace = true }
async-trait = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
reqwest     = { workspace = true }
futures     = { workspace = true }
clap        = { workspace = true }
chrono      = { workspace = true }
toml        = "0.8"
glob        = "0.3"

[[bin]]
name = "searchboxd"
path = "bin/searchboxd.rs"

[features]
integration-tests = []

[dev-dependencies]
tokio    = { workspace = true, features = ["full"] }
tempfile = { workspace = true }
```

- [ ] Write `crates/searchbox/src/domain.rs`:

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub repos: Option<Vec<String>>, // None = all
    pub lang: Option<String>,
    pub case_sensitive: bool,
    pub context_lines: u8, // default 2
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            repos: None,
            lang: None,
            case_sensitive: false,
            context_lines: 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub repo: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub snippet: String,
    pub score: f32,
    pub commit: Option<String>, // SHA if from git history ref
}

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub name: String,
    pub source_type: SourceType,
    pub last_indexed: Option<DateTime<Utc>>,
    pub doc_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Git,
    Filesystem,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Indexing,
}

pub struct SyncStats {
    pub files_synced: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("zoekt unavailable: {0}")]
    Unavailable(String),
    #[error("query failed: {0}")]
    QueryFailed(String),
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("sync failed for {repo}: {reason}")]
    SyncFailed { repo: String, reason: String },
    #[error("index command failed: {0}")]
    IndexCmd(String),
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("ssh: {0}")]
    Ssh(String),
    #[error("process: {0}")]
    Process(String),
}

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError>;
}

#[async_trait]
pub trait IndexSource: Send + Sync {
    fn name(&self) -> &str;
    fn source_type(&self) -> SourceType;
    async fn sync(&self, dest: &Path) -> Result<SyncStats, IndexError>;
}

#[async_trait]
pub trait ServiceManager: Send + Sync {
    async fn start(&self) -> Result<(), ServiceError>;
    async fn stop(&self) -> Result<(), ServiceError>;
    async fn status(&self) -> Result<ServiceStatus, ServiceError>;
    async fn reindex(&self, repo: Option<&str>) -> Result<(), ServiceError>;
}
```

- [ ] Write `crates/searchbox/src/lib.rs`:

```rust
pub mod adapters;
pub mod config;
pub mod domain;
pub mod mcp;

pub use domain::{
    IndexError, IndexSource, RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult,
    ServiceError, ServiceManager, ServiceStatus, SourceType, SyncStats,
};
```

- [ ] Write the failing tests in `crates/searchbox/tests/unit.rs`:

```rust
use searchbox::domain::{SearchQuery, SearchResult, SourceType};

#[test]
fn search_query_defaults() {
    let q = SearchQuery::new("foo");
    assert_eq!(q.text, "foo");
    assert_eq!(q.context_lines, 2);
    assert!(!q.case_sensitive);
    assert!(q.repos.is_none());
}

#[test]
fn source_type_deserializes() {
    let t: SourceType = toml::from_str("\"git\"").unwrap();
    assert_eq!(t, SourceType::Git);
    let t: SourceType = toml::from_str("\"fs\"").unwrap();
    assert_eq!(t, SourceType::Filesystem);
}
```

- [ ] Run: `cargo test -p searchbox 2>&1 | tail -20`
  Expected: both tests pass.

- [ ] Commit:

```bash
git add crates/searchbox/ Cargo.toml
git commit -m "feat(searchbox): scaffold crate with domain types and ports"
```

---

## Task 5: `SearchboxConfig` + TOML parsing

**Files:**
- Create: `crates/searchbox/src/config.rs`
- Modify: `crates/searchbox/tests/unit.rs`

- [ ] Write `crates/searchbox/src/config.rs`:

```rust
use crate::domain::SourceType;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct SearchboxConfig {
    pub service: ServiceConfig,
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
    #[serde(default)]
    pub local: LocalConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServiceConfig {
    pub vps_host: String,
    #[serde(default = "default_zoekt_port")]
    pub zoekt_port: u16,
    /// Cron expression for scheduled reindex. Empty = manual only.
    #[serde(default)]
    pub index_schedule: String,
}

fn default_zoekt_port() -> u16 { 6070 }

#[derive(Debug, Deserialize)]
pub struct RepoConfig {
    pub name: String,
    /// Remote git URL (for source = "git") or local path (for source = "fs"/"local").
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    pub source: SourceType,
}

#[derive(Debug, Default, Deserialize)]
pub struct LocalConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_local_port")]
    pub port: u16,
    #[serde(default)]
    pub repos: Vec<String>,
}

fn default_local_port() -> u16 { 6071 }

impl SearchboxConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let cfg: Self = toml::from_str(&text).context("parse config TOML")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn load_default() -> Result<Self> {
        let path = config_path();
        Self::load(&path)
    }

    fn validate(&self) -> Result<()> {
        for repo in &self.repos {
            match repo.source {
                SourceType::Git => {
                    if repo.url.is_none() {
                        bail!(
                            "repo `{}`: source = \"git\" requires `url` field",
                            repo.name
                        );
                    }
                }
                SourceType::Filesystem | SourceType::Local => {
                    if repo.path.is_none() {
                        bail!(
                            "repo `{}`: source = \"{}\" requires `path` field",
                            repo.name,
                            match repo.source {
                                SourceType::Filesystem => "fs",
                                _ => "local",
                            }
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("SEARCHBOX_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("searchbox")
        .join("config.toml")
}
```

Add `dirs = "5"` and `toml = "0.8"` to `crates/searchbox/Cargo.toml` (both already in workspace or can be added as direct deps):

```toml
dirs = { workspace = true }
toml = "0.8"
```

(`dirs` is already a workspace dep. `toml` is not — add it as a plain dep.)

- [ ] Add tests to `crates/searchbox/tests/unit.rs`:

```rust
use searchbox::config::SearchboxConfig;

#[test]
fn config_parses_valid_toml() {
    let toml = r#"
[service]
vps_host = "minibox"

[[repos]]
name = "myrepo"
url  = "git@github.com:user/myrepo.git"
source = "git"
"#;
    let cfg: SearchboxConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.service.vps_host, "minibox");
    assert_eq!(cfg.service.zoekt_port, 6070); // default
    assert_eq!(cfg.repos[0].name, "myrepo");
}

#[test]
fn config_rejects_git_source_without_url() {
    let toml = r#"
[service]
vps_host = "minibox"

[[repos]]
name   = "bad"
source = "git"
"#;
    let cfg: SearchboxConfig = toml::from_str(toml).unwrap();
    assert!(cfg.validate_pub().is_err());
}
```

Add `pub fn validate_pub(&self) -> anyhow::Result<()> { self.validate() }` to `SearchboxConfig` in `config.rs` for test access.

- [ ] Run: `cargo test -p searchbox 2>&1 | tail -20`
  Expected: all tests pass.

- [ ] Commit:

```bash
git add crates/searchbox/src/config.rs crates/searchbox/tests/unit.rs
git commit -m "feat(searchbox): SearchboxConfig with TOML parsing and validation"
```

---

## Task 6: `MergedAdapter` + `MockSearchProvider`

**Files:**
- Create: `crates/searchbox/src/adapters/mod.rs`
- Create: `crates/searchbox/src/adapters/mock.rs`
- Create: `crates/searchbox/src/adapters/merged.rs`
- Modify: `crates/searchbox/tests/unit.rs`

- [ ] Write `crates/searchbox/src/adapters/mod.rs`:

```rust
pub mod fs_source;
pub mod git_source;
pub mod local;
pub mod merged;
pub mod mock;
pub mod zoekt;
```

- [ ] Write `crates/searchbox/src/adapters/mock.rs`:

```rust
use async_trait::async_trait;
use crate::domain::{RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult, SourceType};

pub struct MockSearchProvider {
    pub results: Vec<SearchResult>,
    pub repos: Vec<RepoInfo>,
    pub fail: bool,
}

impl MockSearchProvider {
    pub fn with_results(results: Vec<SearchResult>) -> Self {
        Self { results, repos: vec![], fail: false }
    }
    pub fn failing() -> Self {
        Self { results: vec![], repos: vec![], fail: true }
    }
}

#[async_trait]
impl SearchProvider for MockSearchProvider {
    async fn search(&self, _query: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        if self.fail {
            return Err(SearchError::Unavailable("mock failure".into()));
        }
        Ok(self.results.clone())
    }
    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
        Ok(self.repos.clone())
    }
}
```

- [ ] Write `crates/searchbox/src/adapters/merged.rs`:

```rust
use async_trait::async_trait;
use futures::future::join_all;
use std::collections::HashSet;
use tracing::warn;

use crate::domain::{RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult};

pub struct MergedAdapter {
    providers: Vec<Box<dyn SearchProvider>>,
}

impl MergedAdapter {
    pub fn new(providers: Vec<Box<dyn SearchProvider>>) -> Self {
        Self { providers }
    }
}

#[async_trait]
impl SearchProvider for MergedAdapter {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        let futs = self.providers.iter().map(|p| p.search(query.clone()));
        let results_per_provider = join_all(futs).await;

        let mut merged: Vec<SearchResult> = Vec::new();
        let mut seen: HashSet<(String, String, u32)> = HashSet::new();

        for result in results_per_provider {
            match result {
                Ok(hits) => {
                    for hit in hits {
                        let key = (hit.repo.clone(), hit.file.clone(), hit.line);
                        if seen.insert(key) {
                            merged.push(hit);
                        }
                    }
                }
                Err(e) => warn!(error = %e, "MergedAdapter: provider error (continuing)"),
            }
        }

        // Sort by score descending
        merged.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(merged)
    }

    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
        let futs = self.providers.iter().map(|p| p.list_repos());
        let all = join_all(futs).await;
        let mut repos: Vec<RepoInfo> = all.into_iter().filter_map(|r| r.ok()).flatten().collect();
        repos.dedup_by(|a, b| a.name == b.name);
        Ok(repos)
    }
}
```

- [ ] Add tests to `crates/searchbox/tests/unit.rs`:

```rust
use searchbox::adapters::{merged::MergedAdapter, mock::MockSearchProvider};
use searchbox::domain::{SearchQuery, SearchResult, SourceType};

fn make_result(repo: &str, file: &str, line: u32, score: f32) -> SearchResult {
    SearchResult {
        repo: repo.into(),
        file: file.into(),
        line,
        col: 0,
        snippet: "snippet".into(),
        score,
        commit: None,
    }
}

#[tokio::test]
async fn merged_deduplicates_same_repo_file_line() {
    let r1 = make_result("repo", "src/lib.rs", 42, 1.0);
    let r2 = make_result("repo", "src/lib.rs", 42, 0.9); // duplicate
    let r3 = make_result("repo", "src/lib.rs", 99, 0.5); // distinct

    let p1 = MockSearchProvider::with_results(vec![r1]);
    let p2 = MockSearchProvider::with_results(vec![r2, r3]);

    let merged = MergedAdapter::new(vec![Box::new(p1), Box::new(p2)]);
    let results = merged.search(SearchQuery::new("foo")).await.unwrap();

    assert_eq!(results.len(), 2, "expected 2 unique results, got {}", results.len());
}

#[tokio::test]
async fn merged_sorts_by_score_descending() {
    let results = vec![
        make_result("r", "a", 1, 0.3),
        make_result("r", "b", 2, 0.9),
        make_result("r", "c", 3, 0.6),
    ];
    let p = MockSearchProvider::with_results(results);
    let merged = MergedAdapter::new(vec![Box::new(p)]);
    let out = merged.search(SearchQuery::new("x")).await.unwrap();
    assert_eq!(out[0].score, 0.9);
    assert_eq!(out[1].score, 0.6);
    assert_eq!(out[2].score, 0.3);
}

#[tokio::test]
async fn merged_continues_on_provider_failure() {
    let good = MockSearchProvider::with_results(vec![make_result("r", "f", 1, 1.0)]);
    let bad  = MockSearchProvider::failing();
    let merged = MergedAdapter::new(vec![Box::new(bad), Box::new(good)]);
    let out = merged.search(SearchQuery::new("x")).await.unwrap();
    assert_eq!(out.len(), 1); // bad provider skipped, good provider returned result
}
```

- [ ] Run: `cargo test -p searchbox 2>&1 | tail -20`
  Expected: all tests pass.

- [ ] Commit:

```bash
git add crates/searchbox/src/adapters/
git commit -m "feat(searchbox): MergedAdapter with dedup, score sort, and failure tolerance"
```

---

## Task 7: `ZoektAdapter`

**Files:**
- Create: `crates/searchbox/src/adapters/zoekt.rs`

- [ ] Write `crates/searchbox/src/adapters/zoekt.rs`:

```rust
use async_trait::async_trait;
use serde::Deserialize;
use tracing::debug;

use crate::domain::{RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult, SourceType};

pub struct ZoektAdapter {
    /// Base URL of zoekt-webserver, e.g. "http://minibox:6070"
    base_url: String,
    client: reqwest::Client,
}

impl ZoektAdapter {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

// Zoekt search API response shapes
#[derive(Deserialize)]
struct ZoektSearchResponse {
    #[serde(rename = "Result")]
    result: Option<ZoektResult>,
}

#[derive(Deserialize)]
struct ZoektResult {
    #[serde(rename = "Files")]
    files: Option<Vec<ZoektFile>>,
}

#[derive(Deserialize)]
struct ZoektFile {
    #[serde(rename = "Repository")]
    repository: String,
    #[serde(rename = "FileName")]
    file_name: String,
    #[serde(rename = "Branches")]
    branches: Option<Vec<String>>,
    #[serde(rename = "LineMatches")]
    line_matches: Option<Vec<ZoektLineMatch>>,
    #[serde(rename = "Score")]
    score: f64,
}

#[derive(Deserialize)]
struct ZoektLineMatch {
    #[serde(rename = "Line")]
    line: String,
    #[serde(rename = "LineNumber")]
    line_number: u32,
    #[serde(rename = "LineFragments")]
    line_fragments: Option<Vec<ZoektFragment>>,
}

#[derive(Deserialize)]
struct ZoektFragment {
    #[serde(rename = "LineOffset")]
    line_offset: u32,
}

#[derive(Deserialize)]
struct ZoektListResponse {
    #[serde(rename = "Repos")]
    repos: Option<Vec<ZoektRepoEntry>>,
}

#[derive(Deserialize)]
struct ZoektRepoEntry {
    #[serde(rename = "Repository")]
    repository: ZoektRepoInfo,
    #[serde(rename = "Stats")]
    stats: Option<ZoektRepoStats>,
}

#[derive(Deserialize)]
struct ZoektRepoInfo {
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Deserialize)]
struct ZoektRepoStats {
    #[serde(rename = "Documents")]
    documents: Option<u64>,
}

#[async_trait]
impl SearchProvider for ZoektAdapter {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        let url = format!("{}/search", self.base_url);

        // Build Zoekt JSON query
        let mut zoekt_query = query.text.clone();
        if query.case_sensitive {
            zoekt_query = format!("case:yes {zoekt_query}");
        }
        if let Some(lang) = &query.lang {
            zoekt_query = format!("lang:{lang} {zoekt_query}");
        }
        if let Some(repos) = &query.repos {
            let repo_filter = repos.iter().map(|r| format!("repo:{r}")).collect::<Vec<_>>().join(" ");
            zoekt_query = format!("{repo_filter} {zoekt_query}");
        }

        let body = serde_json::json!({
            "Q": zoekt_query,
            "Opts": {
                "NumContextLines": query.context_lines,
                "MaxDocDisplayCount": 100,
            }
        });

        debug!(query = %zoekt_query, "ZoektAdapter: searching");

        let resp: ZoektSearchResponse = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Unavailable(e.to_string()))?
            .error_for_status()
            .map_err(|e| SearchError::QueryFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| SearchError::QueryFailed(format!("decode: {e}")))?;

        let files = resp.result.and_then(|r| r.files).unwrap_or_default();
        let mut results = Vec::new();

        for file in files {
            // If result is from a non-main branch, populate commit field
            let commit = file.branches.as_ref()
                .and_then(|b| b.iter().find(|br| *br != "HEAD" && *br != "main" && *br != "master"))
                .cloned();

            for lm in file.line_matches.unwrap_or_default() {
                let col = lm.line_fragments.as_ref()
                    .and_then(|f| f.first())
                    .map(|f| f.line_offset)
                    .unwrap_or(0);

                results.push(SearchResult {
                    repo: file.repository.clone(),
                    file: file.file_name.clone(),
                    line: lm.line_number,
                    col,
                    snippet: lm.line.clone(),
                    score: file.score as f32,
                    commit: commit.clone(),
                });
            }
        }

        Ok(results)
    }

    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
        let url = format!("{}/list", self.base_url);
        let body = serde_json::json!({ "Q": { "Repo": ".*" } });

        let resp: ZoektListResponse = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Unavailable(e.to_string()))?
            .error_for_status()
            .map_err(|e| SearchError::QueryFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| SearchError::QueryFailed(format!("decode: {e}")))?;

        let repos = resp.repos.unwrap_or_default().into_iter().map(|e| RepoInfo {
            name: e.repository.name,
            source_type: SourceType::Git,
            last_indexed: None,
            doc_count: e.stats.and_then(|s| s.documents).unwrap_or(0),
        }).collect();

        Ok(repos)
    }
}
```

- [ ] Run: `cargo check -p searchbox 2>&1 | tail -20`
  Expected: clean.

- [ ] Commit:

```bash
git add crates/searchbox/src/adapters/zoekt.rs
git commit -m "feat(searchbox): ZoektAdapter — HTTP client to zoekt-webserver"
```

---

## Task 8: `GitRepoSource` + `FilesystemSource` + `LocalZoektSource`

**Files:**
- Create: `crates/searchbox/src/adapters/git_source.rs`
- Create: `crates/searchbox/src/adapters/fs_source.rs`
- Create: `crates/searchbox/src/adapters/local.rs`

- [ ] Write `crates/searchbox/src/adapters/git_source.rs`:

```rust
use anyhow::Context;
use async_trait::async_trait;
use std::path::Path;
use tracing::info;

use crate::domain::{IndexError, IndexSource, SourceType, SyncStats};

/// Mirrors a remote git repo into `dest` (bare clone) for zoekt-git-index.
pub struct GitRepoSource {
    pub name: String,
    pub url: String,
    /// SSH host to run `zoekt-git-index` on after mirroring.
    pub ssh_host: String,
    pub remote_index_dir: String,
}

#[async_trait]
impl IndexSource for GitRepoSource {
    fn name(&self) -> &str { &self.name }
    fn source_type(&self) -> SourceType { SourceType::Git }

    async fn sync(&self, dest: &Path) -> Result<SyncStats, IndexError> {
        let bare = dest.join(format!("{}.git", self.name));
        let map_err = |e: anyhow::Error| IndexError::SyncFailed {
            repo: self.name.clone(),
            reason: e.to_string(),
        };

        if bare.exists() {
            // Update existing mirror
            info!(repo = %self.name, "GitRepoSource: updating mirror");
            tokio::process::Command::new("git")
                .args(["-C", bare.to_str().unwrap(), "remote", "update"])
                .status()
                .await
                .context("git remote update")
                .map_err(map_err)?;
        } else {
            // Initial clone
            info!(repo = %self.name, url = %self.url, "GitRepoSource: cloning mirror");
            tokio::process::Command::new("git")
                .args(["clone", "--mirror", &self.url, bare.to_str().unwrap()])
                .status()
                .await
                .context("git clone --mirror")
                .map_err(map_err)?;
        }

        // Rsync bare repo to VPS
        let status = tokio::process::Command::new("rsync")
            .args([
                "-avz",
                &format!("{}/", bare.display()),
                &format!("{}:{}/{}.git/", self.ssh_host, self.remote_index_dir, self.name),
            ])
            .status()
            .await
            .context("rsync to VPS")
            .map_err(map_err)?;

        if !status.success() {
            return Err(IndexError::SyncFailed {
                repo: self.name.clone(),
                reason: format!("rsync exited {status}"),
            });
        }

        // Trigger zoekt-git-index on VPS
        let index_cmd = format!(
            "{}/bin/zoekt-git-index -index {dir} {dir}/{}.git",
            self.remote_index_dir,
            self.name,
            dir = self.remote_index_dir,
        );
        zoektbox::deploy::ssh_run(&self.ssh_host, &index_cmd)
            .await
            .context("zoekt-git-index")
            .map_err(|e| IndexError::IndexCmd(e.to_string()))?;

        info!(repo = %self.name, "GitRepoSource: sync complete");
        Ok(SyncStats { files_synced: 1 })
    }
}
```

- [ ] Write `crates/searchbox/src/adapters/fs_source.rs`:

```rust
use async_trait::async_trait;
use glob::glob;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::domain::{IndexError, IndexSource, SourceType, SyncStats};

/// Rsyncs a glob-expanded local path to the VPS, then runs zoekt-index.
pub struct FilesystemSource {
    pub name: String,
    /// Glob pattern, e.g. "~/dev/*/docs"
    pub glob_pattern: String,
    pub ssh_host: String,
    pub remote_index_dir: String,
    pub remote_base: String,
}

#[async_trait]
impl IndexSource for FilesystemSource {
    fn name(&self) -> &str { &self.name }
    fn source_type(&self) -> SourceType { SourceType::Filesystem }

    async fn sync(&self, _dest: &Path) -> Result<SyncStats, IndexError> {
        let pattern = shellexpand::tilde(&self.glob_pattern).into_owned();
        let paths: Vec<PathBuf> = glob(&pattern)
            .map_err(|e| IndexError::SyncFailed {
                repo: self.name.clone(),
                reason: format!("glob error: {e}"),
            })?
            .filter_map(|e| e.ok())
            .collect();

        if paths.is_empty() {
            return Err(IndexError::SyncFailed {
                repo: self.name.clone(),
                reason: format!("glob `{pattern}` matched no paths"),
            });
        }

        let remote_path = format!("{}/{}", self.remote_base, self.name);
        zoektbox::deploy::ssh_run(
            &self.ssh_host,
            &format!("mkdir -p {remote_path}"),
        )
        .await
        .map_err(|e| IndexError::SyncFailed { repo: self.name.clone(), reason: e.to_string() })?;

        let mut total = 0u64;
        for local_path in &paths {
            info!(src = %local_path.display(), dest = %remote_path, "FilesystemSource: rsyncing");
            let status = tokio::process::Command::new("rsync")
                .args(["-avz", "--delete", &format!("{}/", local_path.display()),
                       &format!("{}:{remote_path}/", self.ssh_host)])
                .status()
                .await
                .map_err(|e| IndexError::SyncFailed {
                    repo: self.name.clone(),
                    reason: e.to_string(),
                })?;

            if !status.success() {
                return Err(IndexError::SyncFailed {
                    repo: self.name.clone(),
                    reason: format!("rsync exited {status}"),
                });
            }
            total += 1;
        }

        // Trigger zoekt-index on VPS
        let index_cmd = format!(
            "{}/bin/zoekt-index -index {} {}",
            self.remote_base, self.remote_index_dir, remote_path
        );
        zoektbox::deploy::ssh_run(&self.ssh_host, &index_cmd)
            .await
            .map_err(|e| IndexError::IndexCmd(e.to_string()))?;

        Ok(SyncStats { files_synced: total })
    }
}
```

Add `shellexpand = "3"` and `glob = "0.3"` to `crates/searchbox/Cargo.toml`.

- [ ] Write `crates/searchbox/src/adapters/local.rs`:

```rust
use async_trait::async_trait;
use std::path::Path;

use crate::domain::{
    IndexError, IndexSource, RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult,
    SourceType, SyncStats,
};
use super::zoekt::ZoektAdapter;

/// Local Mac-side Zoekt sidecar. Indexes paths that must not leave the Mac.
/// Acts as both IndexSource (manages local zoekt process) and SearchProvider (queries it).
pub struct LocalZoektSource {
    pub name: String,
    pub local_path: String,
    port: u16,
    zoekt: ZoektAdapter,
}

impl LocalZoektSource {
    pub fn new(name: impl Into<String>, local_path: impl Into<String>, port: u16) -> Self {
        let port = port;
        Self {
            name: name.into(),
            local_path: local_path.into(),
            port,
            zoekt: ZoektAdapter::new(format!("http://localhost:{port}")),
        }
    }
}

#[async_trait]
impl IndexSource for LocalZoektSource {
    fn name(&self) -> &str { &self.name }
    fn source_type(&self) -> SourceType { SourceType::Local }

    async fn sync(&self, _dest: &Path) -> Result<SyncStats, IndexError> {
        // zoekt-git-index or zoekt-index run locally (binaries must be on PATH)
        let path = shellexpand::tilde(&self.local_path).into_owned();
        let status = tokio::process::Command::new("zoekt-git-index")
            .args(["-index", &format!("/tmp/zoekt-local-{}", self.name), &path])
            .status()
            .await
            .map_err(|e| IndexError::IndexCmd(e.to_string()))?;

        if !status.success() {
            return Err(IndexError::IndexCmd(format!("zoekt-git-index exited {status}")));
        }
        Ok(SyncStats { files_synced: 1 })
    }
}

#[async_trait]
impl SearchProvider for LocalZoektSource {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        self.zoekt.search(query).await
    }

    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
        self.zoekt.list_repos().await
    }
}
```

- [ ] Run: `cargo check -p searchbox 2>&1 | tail -20`
  Expected: clean.

- [ ] Commit:

```bash
git add crates/searchbox/src/adapters/git_source.rs \
        crates/searchbox/src/adapters/fs_source.rs \
        crates/searchbox/src/adapters/local.rs \
        crates/searchbox/Cargo.toml
git commit -m "feat(searchbox): GitRepoSource, FilesystemSource, LocalZoektSource"
```

---

## Task 9: MCP stdio server (`mcp.rs`)

**Files:**
- Create: `crates/searchbox/src/mcp.rs`

- [ ] Write `crates/searchbox/src/mcp.rs`:

```rust
//! JSON-RPC 2.0 stdio MCP server (MCP spec 2025-03-26).
//! Reads newline-delimited JSON from stdin, writes responses to stdout.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use tracing::{debug, error};

use crate::domain::{SearchProvider, SearchQuery, ServiceManager};

// ---------------------------------------------------------------------------
// JSON-RPC wire types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Request {
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

impl Response {
    fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }
    fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self { jsonrpc: "2.0", id, result: None, error: Some(RpcError { code, message: message.into() }) }
    }
}

// ---------------------------------------------------------------------------
// MCP tool manifest
// ---------------------------------------------------------------------------

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "search",
                "description": "Full-text search across indexed repos",
                "inputSchema": {
                    "type": "object",
                    "required": ["q"],
                    "properties": {
                        "q":              { "type": "string" },
                        "repos":          { "type": "array", "items": { "type": "string" } },
                        "lang":           { "type": "string" },
                        "case_sensitive": { "type": "boolean" },
                        "context_lines":  { "type": "integer", "minimum": 0, "maximum": 10 }
                    }
                }
            },
            {
                "name": "list_repos",
                "description": "List indexed repos and last-indexed timestamps",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "reindex",
                "description": "Trigger reindex. Omit `repo` to reindex all.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repo": { "type": "string" }
                    }
                }
            },
            {
                "name": "service_status",
                "description": "Check zoekt-webserver health on VPS",
                "inputSchema": { "type": "object", "properties": {} }
            }
        ]
    })
}

// ---------------------------------------------------------------------------
// Stdio loop
// ---------------------------------------------------------------------------

pub async fn run_stdio_loop(
    search: &dyn SearchProvider,
    service: &dyn ServiceManager,
) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }

        debug!(line = %line, "mcp: recv");

        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::err(Value::Null, -32700, format!("parse error: {e}"));
                writeln!(out, "{}", serde_json::to_string(&resp)?)?;
                out.flush()?;
                continue;
            }
        };

        let resp = dispatch(&req, search, service).await;
        writeln!(out, "{}", serde_json::to_string(&resp)?)?;
        out.flush()?;
    }
    Ok(())
}

async fn dispatch(req: &Request, search: &dyn SearchProvider, service: &dyn ServiceManager) -> Response {
    let id = req.id.clone();
    let params = req.params.clone().unwrap_or(json!({}));

    match req.method.as_str() {
        "initialize" => Response::ok(id, json!({
            "protocolVersion": "2025-03-26",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "searchbox", "version": env!("CARGO_PKG_VERSION") }
        })),

        "tools/list" => Response::ok(id, tools_list()),

        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("").to_string();
            let args = params["arguments"].clone();
            handle_tool_call(id, &name, &args, search, service).await
        }

        other => Response::err(id, -32601, format!("method not found: {other}")),
    }
}

async fn handle_tool_call(
    id: Value,
    name: &str,
    args: &Value,
    search: &dyn SearchProvider,
    service: &dyn ServiceManager,
) -> Response {
    match name {
        "search" => {
            let q = match args["q"].as_str() {
                Some(s) => s.to_string(),
                None => return Response::err(id, -32602, "missing `q`"),
            };
            let mut query = SearchQuery::new(q);
            if let Some(repos) = args["repos"].as_array() {
                query.repos = Some(repos.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            }
            if let Some(lang) = args["lang"].as_str() {
                query.lang = Some(lang.to_string());
            }
            if let Some(cs) = args["case_sensitive"].as_bool() {
                query.case_sensitive = cs;
            }
            if let Some(ctx) = args["context_lines"].as_u64() {
                query.context_lines = ctx.min(10) as u8;
            }
            match search.search(query).await {
                Ok(results) => {
                    let content: Vec<Value> = results.iter().map(|r| json!({
                        "repo": r.repo, "file": r.file,
                        "line": r.line, "col": r.col,
                        "snippet": r.snippet, "score": r.score,
                        "commit": r.commit,
                    })).collect();
                    Response::ok(id, json!({ "content": [{ "type": "text", "text": serde_json::to_string(&content).unwrap() }] }))
                }
                Err(e) => Response::err(id, -32000, e.to_string()),
            }
        }

        "list_repos" => {
            match search.list_repos().await {
                Ok(repos) => {
                    let content: Vec<Value> = repos.iter().map(|r| json!({
                        "name": r.name,
                        "source_type": format!("{:?}", r.source_type),
                        "last_indexed": r.last_indexed.map(|t| t.to_rfc3339()),
                        "doc_count": r.doc_count,
                    })).collect();
                    Response::ok(id, json!({ "content": [{ "type": "text", "text": serde_json::to_string(&content).unwrap() }] }))
                }
                Err(e) => Response::err(id, -32000, e.to_string()),
            }
        }

        "reindex" => {
            let repo = args["repo"].as_str();
            match service.reindex(repo).await {
                Ok(()) => Response::ok(id, json!({ "content": [{ "type": "text", "text": "reindex triggered" }] })),
                Err(e) => Response::err(id, -32000, e.to_string()),
            }
        }

        "service_status" => {
            match service.status().await {
                Ok(status) => Response::ok(id, json!({ "content": [{ "type": "text", "text": format!("{status:?}") }] })),
                Err(e) => Response::err(id, -32000, e.to_string()),
            }
        }

        other => Response::err(id, -32601, format!("unknown tool: {other}")),
    }
}
```

- [ ] Run: `cargo check -p searchbox 2>&1 | tail -20`
  Expected: clean.

- [ ] Commit:

```bash
git add crates/searchbox/src/mcp.rs
git commit -m "feat(searchbox): MCP stdio JSON-RPC server"
```

---

## Task 10: `searchboxd` binary (composition root)

**Files:**
- Create: `crates/searchbox/bin/searchboxd.rs`

- [ ] Create `crates/searchbox/bin/` directory and write `searchboxd.rs`:

```rust
//! searchboxd — composition root for searchbox.
//!
//! Subcommands:
//!   mcp        — run MCP stdio server (for Claude Code)
//!   status     — check zoekt-webserver health
//!   reindex    — trigger reindex (optionally for one repo)
//!   provision  — download + deploy Zoekt binaries to VPS, then start service

use anyhow::Result;
use clap::{Parser, Subcommand};
use searchbox::{
    adapters::{merged::MergedAdapter, zoekt::ZoektAdapter},
    config::SearchboxConfig,
    domain::{ServiceManager, ServiceStatus},
    mcp,
};
use zoektbox::service::{ZoektServiceAdapter, ZoektServiceConfig};

#[derive(Parser)]
#[command(name = "searchboxd", version, about = "Zoekt-backed code search MCP server")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run as MCP stdio server
    Mcp,
    /// Check zoekt-webserver health
    Status,
    /// Trigger reindex (--repo NAME for single repo, omit for all)
    Reindex {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Provision Zoekt on the VPS (first-time setup)
    Provision,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cfg = SearchboxConfig::load_default()?;

    let svc_cfg = ZoektServiceConfig {
        ssh_host: cfg.service.vps_host.clone(),
        port: cfg.service.zoekt_port,
        remote_base: "/opt/zoekt".into(),
    };
    let service = ZoektServiceAdapter::new(svc_cfg);

    match cli.cmd {
        Cmd::Provision => {
            service.provision().await?;
            service.start().await?;
            println!("Provisioning complete. zoekt-webserver running on {}:{}",
                cfg.service.vps_host, cfg.service.zoekt_port);
        }

        Cmd::Status => {
            let running = service.status().await?;
            println!("{}", if running { "running" } else { "stopped" });
        }

        Cmd::Reindex { repo } => {
            service.reindex(repo.as_deref()).await?;
            println!("Reindex triggered");
        }

        Cmd::Mcp => {
            let base_url = format!("http://{}:{}", cfg.service.vps_host, cfg.service.zoekt_port);
            let zoekt = ZoektAdapter::new(&base_url);

            // Build merged adapter (local sidecar added here if cfg.local.enabled)
            let mut providers: Vec<Box<dyn searchbox::domain::SearchProvider>> =
                vec![Box::new(zoekt)];

            if cfg.local.enabled {
                providers.push(Box::new(
                    searchbox::adapters::local::LocalZoektSource::new(
                        "local",
                        "",
                        cfg.local.port,
                    ),
                ));
            }

            let merged = MergedAdapter::new(providers);

            // ServiceManager bridge: wrap ZoektServiceAdapter methods
            let svc_bridge = ServiceBridge { inner: service };
            mcp::run_stdio_loop(&merged, &svc_bridge).await?;
        }
    }

    Ok(())
}

/// Bridge ZoektServiceAdapter's direct async methods into the ServiceManager trait.
/// ZoektServiceAdapter methods are not yet behind the trait (see spec: trait defined in
/// searchbox::domain, implemented in zoektbox). This bridge handles that until Task 11
/// wires the full trait impl.
struct ServiceBridge {
    inner: ZoektServiceAdapter,
}

#[async_trait::async_trait]
impl searchbox::domain::ServiceManager for ServiceBridge {
    async fn start(&self) -> Result<(), searchbox::domain::ServiceError> {
        self.inner.start().await.map_err(|e| searchbox::domain::ServiceError::Process(e.to_string()))
    }
    async fn stop(&self) -> Result<(), searchbox::domain::ServiceError> {
        self.inner.stop().await.map_err(|e| searchbox::domain::ServiceError::Process(e.to_string()))
    }
    async fn status(&self) -> Result<searchbox::domain::ServiceStatus, searchbox::domain::ServiceError> {
        let running = self.inner.status().await.map_err(|e| searchbox::domain::ServiceError::Ssh(e.to_string()))?;
        Ok(if running { ServiceStatus::Running } else { ServiceStatus::Stopped })
    }
    async fn reindex(&self, repo: Option<&str>) -> Result<(), searchbox::domain::ServiceError> {
        self.inner.reindex(repo).await.map_err(|e| searchbox::domain::ServiceError::Process(e.to_string()))
    }
}
```

- [ ] Run: `cargo build -p searchbox 2>&1 | tail -30`
  Expected: binary `target/debug/searchboxd` produced, no errors.

- [ ] Smoke test:

```bash
./target/debug/searchboxd --help
# Expected: usage with mcp | status | reindex | provision subcommands
```

- [ ] Commit:

```bash
git add crates/searchbox/bin/searchboxd.rs
git commit -m "feat(searchbox): searchboxd binary — mcp, status, reindex, provision"
```

---

## Task 11: Wire MCP into `~/.claude/settings.json`

**Files:**
- Modify: `~/.claude/settings.json`

- [ ] Install binary:

```bash
cargo install --path crates/searchbox --bin searchboxd
# confirms: installed at ~/.cargo/bin/searchboxd
```

- [ ] Add MCP server entry to `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "searchbox": {
      "type": "stdio",
      "command": "searchboxd",
      "args": ["mcp"]
    }
  }
}
```

- [ ] Uninstall the old sourcegraph plugin (it points at a broken endpoint):

```bash
claude plugin uninstall sourcegraph
```

- [ ] Verify MCP registration:

```bash
claude mcp list
# Expected: searchbox entry present, sourcegraph entry gone
```

- [ ] Commit:

```bash
git -C /Users/joe/dev/minibox add -A
git -C /Users/joe/dev/minibox commit -m "feat(searchbox): wire MCP into Claude settings"
```

---

## Task 12: Update `~/.config/searchbox/config.toml` + initial provision

**Files:**
- Create: `~/.config/searchbox/config.toml`

- [ ] Create config:

```toml
[service]
vps_host       = "minibox"
zoekt_port     = 6070
index_schedule = "0 * * * *"

[[repos]]
name   = "minibox"
url    = "git@github.com:89jobrien/minibox.git"
source = "git"

[[repos]]
name   = "obsidian-vault"
path   = "~/Documents/Obsidian Vault"
source = "git"

[[repos]]
name   = "dev-docs"
path   = "~/dev/*/docs"
source = "fs"
```

- [ ] Verify config parses:

```bash
searchboxd status
# Expected: "stopped" (not a config error)
```

- [ ] Provision Zoekt on VPS:

```bash
searchboxd provision
# Expected: "Provisioning complete. zoekt-webserver running on minibox:6070"
```

- [ ] Verify running:

```bash
searchboxd status
# Expected: "running"
```

- [ ] Trigger initial index of minibox repo:

```bash
searchboxd reindex --repo minibox
# Expected: "Reindex triggered"
```

---

## Task 13: Integration test

**Files:**
- Modify: `crates/searchbox/tests/integration.rs`

- [ ] Write `crates/searchbox/tests/integration.rs`:

```rust
//! Integration tests: ZoektAdapter against a live zoekt-webserver.
//! Run with: cargo test -p searchbox --features integration-tests -- --ignored
//!
//! Requires: zoekt-webserver and zoekt-git-index on PATH (or in /opt/zoekt/bin/).
//! Set ZOEKT_BASE_URL to override default http://localhost:6070.

#![cfg(feature = "integration-tests")]

use searchbox::adapters::zoekt::ZoektAdapter;
use searchbox::domain::{SearchProvider, SearchQuery};
use std::process::Command;
use tempfile::TempDir;

fn zoekt_base_url() -> String {
    std::env::var("ZOEKT_BASE_URL").unwrap_or_else(|_| "http://localhost:6070".into())
}

#[tokio::test]
#[ignore = "requires live zoekt-webserver"]
async fn zoekt_adapter_search_returns_results() {
    let adapter = ZoektAdapter::new(zoekt_base_url());
    let results = adapter.search(SearchQuery::new("fn main")).await.unwrap();
    // Just verify we got a response — result count varies by what's indexed
    println!("got {} results", results.len());
}

#[tokio::test]
#[ignore = "requires live zoekt-webserver"]
async fn zoekt_adapter_list_repos_returns_at_least_one() {
    let adapter = ZoektAdapter::new(zoekt_base_url());
    let repos = adapter.list_repos().await.unwrap();
    assert!(!repos.is_empty(), "expected at least one indexed repo");
    println!("repos: {:?}", repos.iter().map(|r| &r.name).collect::<Vec<_>>());
}
```

- [ ] Run unit tests to confirm nothing regressed:

```bash
cargo test -p searchbox 2>&1 | tail -20
cargo test -p zoektbox  2>&1 | tail -20
```

Expected: all pass.

- [ ] Commit:

```bash
git add crates/searchbox/tests/integration.rs
git commit -m "test(searchbox): integration test skeleton for ZoektAdapter"
```

---

## Task 14: Update workspace xtask clippy gates

**Files:**
- Modify: `crates/xtask/src/main.rs` (or wherever clippy crate list is defined)

- [ ] Find the clippy crate list:

```bash
grep -n "zoektbox\|searchbox\|clippy" crates/xtask/src/main.rs | head -20
```

- [ ] Add `zoektbox` and `searchbox` to the clippy invocation in `pre_commit()`:

The existing clippy command in xtask looks like:
```
cargo clippy -p minibox -p minibox-macros ... -- -D warnings
```

Add `-p zoektbox -p searchbox` to that list.

- [ ] Run the full pre-commit gate:

```bash
cargo xtask pre-commit
```

Expected: clean, all crates pass clippy.

- [ ] Commit:

```bash
git add crates/xtask/src/main.rs
git commit -m "chore: add zoektbox and searchbox to xtask clippy gates"
```

---

## Self-Review

**Spec coverage check:**
- ✅ `zoektbox`: release manifest, download+verify, deploy, service adapter
- ✅ `searchbox`: domain types + ports, config + validation, MergedAdapter, ZoektAdapter,
  GitRepoSource, FilesystemSource, LocalZoektSource, MCP stdio server, searchboxd binary
- ✅ MCP tool schemas: `search`, `list_repos`, `reindex`, `service_status`
- ✅ Config validation: `source = "git"` without `url` rejected at parse
- ✅ Security: Tailscale-IP-only binding wired in `service.rs`
- ✅ Phase B hook: `ServiceBridge` pattern makes swap to `GiteaAdapter` a composition-root change only
- ✅ Integration tests (feature-gated)
- ✅ Xtask clippy gate updated

**Type consistency check:**
- `SearchQuery`, `SearchResult`, `RepoInfo`, `SyncStats` defined in Task 4, used consistently throughout
- `ZoektAdapter::new(base_url)` — string arg used in Tasks 7 and 10 ✅
- `MergedAdapter::new(providers)` — `Vec<Box<dyn SearchProvider>>` used in Tasks 6 and 10 ✅
- `ServiceBridge` wraps `ZoektServiceAdapter` directly in Task 10, bridging to `ServiceManager` trait ✅
- `mcp::run_stdio_loop(&search, &service)` — `&dyn SearchProvider` + `&dyn ServiceManager` ✅

**Gaps fixed:** `toml` dep added as direct (not workspace) dep in Task 5; `shellexpand` and `glob`
added in Task 8; `dirs` workspace dep used in config path resolution.
