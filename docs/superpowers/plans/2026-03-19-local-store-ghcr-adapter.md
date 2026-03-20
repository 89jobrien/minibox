---
status: done
completed: "2026-03-19"
branch: feat/local-store-ghcr
note: ImageRef, GhcrRegistry, streaming protocol all shipped
---
# Local Store + GHCR Adapter + Protocol Streaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `ImageRef` parsing, `GhcrRegistry` adapter, user-local image store (`~/.mbx/cache/`), and protocol streaming (`ContainerOutput`/`ContainerStopped`) so `minibox pull ghcr.io/org/image:tag` works and container stdout/stderr pipes to the terminal.

**Architecture:** 7 sequential tasks. Tasks 1–5 are macOS-safe (pure Rust, no Linux syscalls). Tasks 6–7 touch container process pipes (Linux only) and require integration testing on Linux. Each task ends with a commit.

**Tech Stack:** Rust 2024, tokio, reqwest, nix (pipe/dup2), serde_json, xshell (xtask)

---

## File Map

### New files
| File | Responsibility |
|------|---------------|
| `crates/minibox-lib/src/image/reference.rs` | `ImageRef` type — parse/validate/route image references |
| `crates/minibox-lib/src/adapters/ghcr.rs` | `GhcrRegistry` — ghcr.io OCI adapter |

### Modified files
| File | Change |
|------|--------|
| `crates/minibox-macros/src/lib.rs` | Add `normalize_name!`, `normalize_digest!`, `normalize!`, `denormalize_digest!` (mirrors `as_any!`/`default_new!`/`adapt!` pattern) |
| `crates/minibox-lib/src/image/mod.rs` | Add `pub mod reference;`; replace 3 inline `.replace` calls with `normalize_name!`/`normalize_digest!` |
| `crates/minibox-lib/src/image/registry.rs` | Replace 2 inline `.replace` calls with `normalize_name!`/`normalize_digest!` |
| `crates/minibox-lib/src/adapters/registry.rs` | Replace 1 inline `.replace('_', ":")` call with `denormalize_digest!` |
| `crates/minibox-lib/src/lib.rs` | Re-export `ImageRef` at crate root |
| `crates/minibox-lib/src/adapters/mod.rs` | Add `pub mod ghcr; pub use ghcr::GhcrRegistry;` |
| `crates/minibox-lib/src/protocol.rs` | Add `ContainerOutput`, `ContainerStopped`, `OutputStreamKind`; add `ephemeral: bool` to `Run` |
| `crates/minibox-lib/src/domain.rs` | Add `SpawnResult` type; add `capture_output: bool` and `stdout_fd: Option<OwnedFd>` to `ContainerSpawnConfig`; update `spawn_process` to return `SpawnResult` |
| `crates/minibox-lib/src/container/process.rs` | Add pipe setup before clone; child dup2s; parent returns read end |
| `crates/daemonbox/src/handler.rs` | Add `select_registry()`, update `handle_run`/`handle_pull` to use `ImageRef`, add streaming loop |
| `crates/daemonbox/src/server.rs` | Support multi-message streaming responses via channel |
| `crates/miniboxd/src/main.rs` | Add `GhcrRegistry` init, `resolve_data_dir()` with UID-aware default |
| `crates/minibox-cli/src/commands/run.rs` | Handle `ContainerOutput` / `ContainerStopped` streaming response |

### Disk layout note
`ImageStore.image_dir()` uses `normalize_name!(name)` to replace `/` with `_` in image names. `get_image_layers()` and `store_layer()` use `normalize_digest!(digest)` to replace `:` with `_` in digest keys. Both macros live in `minibox-macros` alongside `adapt!`. For backward compatibility, `ImageRef::cache_name()` returns the unqualified name for docker.io images (e.g., `library/alpine`) and a registry-prefixed name for others (e.g., `ghcr.io/org/image` → stored as `ghcr.io_org_image/`). Existing docker.io image caches are preserved.

---

## Task 1: `ImageRef` — Image Reference Parsing

**Files:**
- Create: `crates/minibox-lib/src/image/reference.rs`
- Modify: `crates/minibox-lib/src/image/mod.rs`
- Modify: `crates/minibox-lib/src/lib.rs`

- [ ] **Step 1.1: Write failing tests in `reference.rs`**

Create `crates/minibox-lib/src/image/reference.rs` with tests only first:

```rust
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    pub registry: String,
    pub namespace: String,
    pub name: String,
    pub tag: String,
}

#[derive(Debug, Error, PartialEq)]
pub enum ImageRefError {
    #[error("empty image reference")]
    Empty,
    #[error("invalid image reference: {0}")]
    Invalid(String),
}

impl ImageRef {
    pub fn parse(_s: &str) -> Result<Self, ImageRefError> {
        unimplemented!()
    }

    pub fn registry_host(&self) -> &str {
        unimplemented!()
    }

    pub fn repository(&self) -> String {
        unimplemented!()
    }

    /// Storage key for ImageStore. Backward compat: docker.io returns "namespace/name"
    /// (no registry prefix) to preserve existing caches. All others prefix with registry.
    pub fn cache_name(&self) -> String {
        unimplemented!()
    }

    pub fn cache_path(&self, images_dir: &Path) -> PathBuf {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parse_bare_name() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.namespace, "library");
        assert_eq!(r.name, "alpine");
        assert_eq!(r.tag, "latest");
    }

    #[test]
    fn parse_name_with_tag() {
        let r = ImageRef::parse("ubuntu:22.04").unwrap();
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.namespace, "library");
        assert_eq!(r.name, "ubuntu");
        assert_eq!(r.tag, "22.04");
    }

    #[test]
    fn parse_org_image() {
        let r = ImageRef::parse("myorg/myimage").unwrap();
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.namespace, "myorg");
        assert_eq!(r.name, "myimage");
        assert_eq!(r.tag, "latest");
    }

    #[test]
    fn parse_org_image_with_tag() {
        let r = ImageRef::parse("myorg/myimage:v2").unwrap();
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.namespace, "myorg");
        assert_eq!(r.name, "myimage");
        assert_eq!(r.tag, "v2");
    }

    #[test]
    fn parse_ghcr_full() {
        let r = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.namespace, "org");
        assert_eq!(r.name, "minibox-rust-ci");
        assert_eq!(r.tag, "stable");
    }

    #[test]
    fn parse_empty_fails() {
        assert_eq!(ImageRef::parse(""), Err(ImageRefError::Empty));
    }

    #[test]
    fn parse_ghcr_without_namespace_fails() {
        // ghcr.io/image:tag has no org — should fail (non-docker.io requires org/name)
        assert!(ImageRef::parse("ghcr.io/image:tag").is_err());
    }

    #[test]
    fn registry_host_docker() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.registry_host(), "registry-1.docker.io");
    }

    #[test]
    fn registry_host_ghcr() {
        let r = ImageRef::parse("ghcr.io/org/image:latest").unwrap();
        assert_eq!(r.registry_host(), "ghcr.io");
    }

    #[test]
    fn repository_docker() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.repository(), "library/alpine");
    }

    #[test]
    fn repository_ghcr() {
        let r = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();
        assert_eq!(r.repository(), "org/minibox-rust-ci");
    }

    #[test]
    fn cache_name_docker_no_prefix() {
        // Backward compat: docker.io images stored without registry prefix
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(r.cache_name(), "library/alpine");
    }

    #[test]
    fn cache_name_ghcr_with_prefix() {
        let r = ImageRef::parse("ghcr.io/org/image:stable").unwrap();
        assert_eq!(r.cache_name(), "ghcr.io/org/image");
    }

    #[test]
    fn cache_path_docker() {
        let r = ImageRef::parse("alpine").unwrap();
        // ImageStore replaces '/' with '_', so cache_name "library/alpine" becomes "library_alpine"
        // cache_path returns the raw path component; ImageStore does the sanitisation
        let p = r.cache_path(Path::new("/data/images"));
        assert_eq!(p, std::path::PathBuf::from("/data/images/library/alpine/latest"));
    }

    #[test]
    fn cache_path_ghcr() {
        let r = ImageRef::parse("ghcr.io/org/image:stable").unwrap();
        let p = r.cache_path(Path::new("/data/images"));
        assert_eq!(p, std::path::PathBuf::from("/data/images/ghcr.io/org/image/stable"));
    }
}
```

- [ ] **Step 1.2: Run tests — expect runtime panics (unimplemented)**

```bash
cargo test -p minibox-lib image::reference -- --nocapture
```

Expected: tests compile but panic with 'not yet implemented' (because `unimplemented!()` panics at runtime, not compile time).

- [ ] **Step 1.3: Implement `ImageRef`**

Replace `unimplemented!()` stubs with real implementations:

```rust
impl ImageRef {
    pub fn parse(s: &str) -> Result<Self, ImageRefError> {
        if s.is_empty() {
            return Err(ImageRefError::Empty);
        }

        // Split tag. The tag separator ':' must not appear in a path component.
        let (path_part, tag) = match s.rsplit_once(':') {
            Some((p, t)) if !t.is_empty() && !t.contains('/') => {
                (p, t.to_owned())
            }
            _ => (s, "latest".to_owned()),
        };

        // Detect registry hostname: first path component contains '.' or ':',
        // or equals "localhost".
        let (registry, rest) = match path_part.split_once('/') {
            Some((first, rest))
                if first.contains('.') || first.contains(':') || first == "localhost" =>
            {
                (first.to_owned(), rest)
            }
            _ => ("docker.io".to_owned(), path_part),
        };

        // Split remaining into namespace/name using the LAST '/'.
        let (namespace, name) = match rest.rsplit_once('/') {
            Some((ns, n)) => (ns.to_owned(), n.to_owned()),
            None => {
                if registry == "docker.io" {
                    ("library".to_owned(), rest.to_owned())
                } else {
                    return Err(ImageRefError::Invalid(format!(
                        "non-docker.io registry requires org/name format, got: {s}"
                    )));
                }
            }
        };

        if name.is_empty() {
            return Err(ImageRefError::Invalid(format!("empty image name in: {s}")));
        }

        Ok(ImageRef { registry, namespace, name, tag })
    }

    pub fn registry_host(&self) -> &str {
        match self.registry.as_str() {
            "docker.io" => "registry-1.docker.io",
            other => other,
        }
    }

    pub fn repository(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    pub fn cache_name(&self) -> String {
        if self.registry == "docker.io" {
            // Backward compat: existing docker.io caches stored without registry prefix
            format!("{}/{}", self.namespace, self.name)
        } else {
            format!("{}/{}/{}", self.registry, self.namespace, self.name)
        }
    }

    pub fn cache_path(&self, images_dir: &Path) -> PathBuf {
        // NOTE: This returns a logical path. ImageStore.image_dir() will replace '/'
        // with '_' internally, so the actual disk path differs from this.
        images_dir
            .join(&self.registry)
            .join(&self.namespace)
            .join(&self.name)
            .join(&self.tag)
    }
}
```

- [ ] **Step 1.4: Add normalization macros to `minibox-macros` and refactor `image/mod.rs`**

**Step 1.4a: Add macros to `crates/minibox-macros/src/lib.rs`**

Append after the existing `adapt!` macro, following the same pattern (`as_any!` → `default_new!` → `adapt!`):

```rust
/// Normalize an image name string for use as a filesystem path component.
///
/// Replaces `/` with `_` (e.g. `"library/alpine"` → `"library_alpine"`).
/// Use for image names. For digest strings use [`normalize_digest!`].
/// Use [`normalize!`] to replace both.
///
/// # Examples
///
/// ```rust
/// use minibox_macros::normalize_name;
/// assert_eq!(normalize_name!("library/alpine"), "library_alpine");
/// assert_eq!(normalize_name!("ghcr.io/org/image"), "ghcr.io_org_image");
/// ```
#[macro_export]
macro_rules! normalize_name {
    ($s:expr) => {
        $s.replace('/', "_")
    };
}

/// Normalize a digest string for use as a filesystem path component.
///
/// Replaces `:` with `_` (e.g. `"sha256:abc123"` → `"sha256_abc123"`).
/// Use for layer digest keys. For image names use [`normalize_name!`].
/// Use [`normalize!`] to replace both.
///
/// # Examples
///
/// ```rust
/// use minibox_macros::normalize_digest;
/// assert_eq!(normalize_digest!("sha256:abc123"), "sha256_abc123");
/// ```
#[macro_export]
macro_rules! normalize_digest {
    ($s:expr) => {
        $s.replace(':', "_")
    };
}

/// Normalize a string for use as a filesystem path component, replacing both
/// `/` and `:` with `_`.
///
/// Equivalent to applying [`normalize_name!`] then [`normalize_digest!`].
/// Use when the input may contain either character (e.g. full image refs).
///
/// # Examples
///
/// ```rust
/// use minibox_macros::normalize;
/// assert_eq!(normalize!("ghcr.io/org/image:stable"), "ghcr.io_org_image_stable");
/// ```
#[macro_export]
macro_rules! normalize {
    ($s:expr) => {
        $s.replace(['/', ':'], "_")
    };
}

/// Recover a digest string from a filesystem path component.
///
/// Reverses [`normalize_digest!`] by replacing `_` with `:`.
/// Used when reading stored layer directories back into digest form.
///
/// # Examples
///
/// ```rust
/// use minibox_macros::denormalize_digest;
/// assert_eq!(denormalize_digest!("sha256_abc123"), "sha256:abc123");
/// ```
#[macro_export]
macro_rules! denormalize_digest {
    ($s:expr) => {
        $s.replace('_', ":")
    };
}
```

**Step 1.4b: Refactor all call sites to use the new macros**

Five call sites across two files get the macros. One reverse call gets `denormalize_digest!`.

`crates/minibox-lib/src/image/mod.rs` — add at the top:
```rust
use minibox_macros::{normalize_digest, normalize_name};
```

Replace inline `.replace` calls (3 sites):
```rust
// image_dir() — image names:
let safe_name = normalize_name!(name);       // was: name.replace('/', "_")

// get_image_layers() — digest keys:
let digest_key = normalize_digest!(desc.digest);  // was: desc.digest.replace(':', "_")

// store_layer() — digest keys:
let digest_key = normalize_digest!(digest);       // was: digest.replace(':', "_")
```

`crates/minibox-lib/src/image/registry.rs` — add at the top:
```rust
use minibox_macros::{normalize_digest, normalize_name};
```

Replace inline `.replace` calls (2 sites):
```rust
// pull_image() layer loop — digest key and name:
let digest_key = normalize_digest!(layer_desc.digest);  // was: layer_desc.digest.replace(':', "_")
let layer_dir = store.base_dir
    .join(normalize_name!(name))   // was: name.replace('/', "_")
    .join(tag)
    .join("layers")
    .join(&digest_key);
```

`crates/minibox-lib/src/adapters/registry.rs` — add at the top:
```rust
use minibox_macros::denormalize_digest;
```

Replace the reverse call (1 site):
```rust
// get_image_layers() reading stored paths back to digest form:
let digest = path
    .file_name()
    .and_then(|s| s.to_str())
    .unwrap_or("unknown")
    .replace('_', ":");   // was inline
// After:
    denormalize_digest!(path.file_name().and_then(|s| s.to_str()).unwrap_or("unknown"))
```

**Not replaced** (intentional differences):
- `colima.rs:160` — `name.replace('/', "-")` uses `-`, not `_` — Lima-specific convention, keep as-is
- `handler.rs:151` — UUID hyphen stripping — unrelated to path normalization, keep as-is

Run tests to confirm no regressions:

```bash
cargo test -p minibox-macros -- --nocapture
cargo test -p minibox-lib image -- --nocapture
cargo test -p minibox-lib adapters -- --nocapture
```

- [ ] **Step 1.5: Wire into `image/mod.rs` and `lib.rs`**

In `crates/minibox-lib/src/image/mod.rs`, add:
```rust
pub mod reference;
```

In `crates/minibox-lib/src/lib.rs`, add a re-export (find the `pub use` block and add):
```rust
pub use image::reference::{ImageRef, ImageRefError};
```

- [ ] **Step 1.6: Run tests — expect all pass**

```bash
cargo test -p minibox-lib image::reference -- --nocapture
cargo test -p minibox-lib image -- --nocapture
```

Expected: all tests pass. Also run:
```bash
cargo clippy -p minibox-lib -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 1.7: Commit**

```bash
git add crates/minibox-macros/src/lib.rs \
        crates/minibox-lib/src/image/reference.rs \
        crates/minibox-lib/src/image/mod.rs \
        crates/minibox-lib/src/lib.rs
git commit -m "feat: add ImageRef parsing; add normalize_name!/normalize_digest!/normalize! macros"
```

---

## Task 2: Data Dir Resolution

**Files:**
- Modify: `crates/miniboxd/src/main.rs`

The daemon currently resolves `data_dir` inline. Extract it into a testable function that picks `~/.mbx/cache/` for non-root.

- [ ] **Step 2.1: Write failing tests at the bottom of `main.rs`**

Add to the end of `crates/miniboxd/src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env-mutating tests to avoid races (set_var is unsafe in Rust 2024)
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_root_uses_system_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("MINIBOX_DATA_DIR") };
        assert_eq!(
            resolve_data_dir_for_uid(0),
            std::path::PathBuf::from("/var/lib/minibox")
        );
    }

    #[test]
    fn resolve_nonroot_uses_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("MINIBOX_DATA_DIR");
            std::env::set_var("HOME", "/home/testuser");
        }
        let dir = resolve_data_dir_for_uid(1000);
        assert_eq!(dir, std::path::PathBuf::from("/home/testuser/.mbx/cache"));
    }

    #[test]
    fn resolve_explicit_override_beats_uid() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("MINIBOX_DATA_DIR", "/custom/path") };
        // Both root and non-root respect explicit override
        assert_eq!(
            resolve_data_dir_for_uid(0),
            std::path::PathBuf::from("/custom/path")
        );
        assert_eq!(
            resolve_data_dir_for_uid(1000),
            std::path::PathBuf::from("/custom/path")
        );
    }
}
```

- [ ] **Step 2.2: Run tests — expect compile error (function not found)**

```bash
cargo test -p miniboxd -- --nocapture
```

Expected: compile error, `resolve_data_dir_for_uid` not found.

- [ ] **Step 2.3: Extract `resolve_data_dir_for_uid` in `main.rs`**

Find the existing inline `data_dir` resolution (around line 196-204) and replace with:

```rust
/// Resolve the data directory for image/container storage.
///
/// Resolution order:
/// 1. `MINIBOX_DATA_DIR` env var (explicit override)
/// 2. `~/.mbx/cache/` if effective UID is non-root
/// 3. `/var/lib/minibox` if running as root
fn resolve_data_dir_for_uid(uid: u32) -> std::path::PathBuf {
    if let Ok(explicit) = std::env::var("MINIBOX_DATA_DIR") {
        return std::path::PathBuf::from(explicit);
    }
    if uid == 0 {
        std::path::PathBuf::from("/var/lib/minibox")
    } else {
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".mbx/cache"))
            .unwrap_or_else(|_| std::path::PathBuf::from("/var/lib/minibox"))
    }
}

fn resolve_data_dir() -> std::path::PathBuf {
    resolve_data_dir_for_uid(nix::unistd::getuid().as_raw())
}
```

Then in `main()`, replace the existing data_dir line with:
```rust
let data_dir = resolve_data_dir();
```

- [ ] **Step 2.4: Run tests — expect all pass**

```bash
cargo test -p miniboxd -- --nocapture
cargo clippy -p miniboxd -- -D warnings
```

- [ ] **Step 2.5: Commit**

```bash
git add crates/miniboxd/src/main.rs
git commit -m "feat(miniboxd): resolve data dir to ~/.mbx/cache for non-root"
```

---

## Task 2b: Preflight Doctor Check for Data Dir

**Files:**
- Modify: `crates/minibox-lib/src/preflight.rs`

Add a line to the preflight doctor report that prints the active data dir. This helps users and operators confirm which storage path is in use.

- [ ] **Step 2b.1: Read `preflight.rs` before editing**

```bash
cat crates/minibox-lib/src/preflight.rs
```

Find the `format_report` function (or equivalent) that builds the human-readable preflight output string.

- [ ] **Step 2b.2: Add data dir line to the doctor report**

In `crates/minibox-lib/src/preflight.rs`, find where the report string is formatted (look for the existing lines like `"cgroups v2: ..."`, `"overlay fs: ..."`, etc.) and add:

```rust
// At the top of the function or alongside other env resolution:
let data_dir = {
    // Mirror the resolution logic from resolve_data_dir_for_uid in miniboxd.
    // preflight.rs lives in minibox-lib so it cannot import miniboxd directly.
    // Inline the resolution here using nix::unistd::getuid():
    let uid = nix::unistd::getuid().as_raw();
    if let Ok(explicit) = std::env::var("MINIBOX_DATA_DIR") {
        std::path::PathBuf::from(explicit)
    } else if uid == 0 {
        std::path::PathBuf::from("/var/lib/minibox")
    } else {
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".mbx/cache"))
            .unwrap_or_else(|_| std::path::PathBuf::from("/var/lib/minibox"))
    }
};

// In the formatted report string, add:
//   "data dir: {}", data_dir.display()
// alongside the other capability lines.
```

The exact insertion point depends on `preflight.rs` structure. Add the line so it appears in the report output, e.g.:

```
data dir: /var/lib/minibox          (when running as root)
data dir: /home/user/.mbx/cache    (when running as non-root)
```

- [ ] **Step 2b.3: Verify existing test still passes**

```bash
cargo test -p minibox-lib preflight -- --nocapture
```

The existing `test_format_report_does_not_panic` (or equivalent smoke test for the report formatter) must still pass — the new line does not change the test's pass/fail condition since it only checks that formatting does not panic.

- [ ] **Step 2b.4: Commit**

```bash
git add crates/minibox-lib/src/preflight.rs
git commit -m "feat(preflight): include active data dir in doctor report"
```

---

## Task 3: `GhcrRegistry` Adapter

**Files:**
- Create: `crates/minibox-lib/src/adapters/ghcr.rs`
- Modify: `crates/minibox-lib/src/adapters/mod.rs`

`GhcrRegistry` follows the same pattern as `DockerHubRegistry`. It wraps an `Arc<ImageStore>` and makes HTTPS calls to `ghcr.io` using the OCI Distribution Spec auth flow.

- [ ] **Step 3.1: Read these files before writing any code**

```bash
# Understand the DockerHubRegistry pattern you're following
cat crates/minibox-lib/src/adapters/registry.rs
cat crates/minibox-lib/src/image/registry.rs
cat crates/minibox-lib/src/adapters/mod.rs
```

- [ ] **Step 3.2: Write failing tests in `ghcr.rs`**

Create `crates/minibox-lib/src/adapters/ghcr.rs` with the struct stub and tests:

```rust
use crate::domain::{ImageMetadata, ImageRegistry, LayerInfo};
use crate::image::ImageStore;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info};

/// GitHub Container Registry adapter.
///
/// Auth: `GHCR_TOKEN` env var (GitHub PAT with `read:packages` scope).
/// If unset, attempts anonymous auth (works for public images).
#[derive(Debug, Clone)]
pub struct GhcrRegistry {
    store: Arc<ImageStore>,
    token: Option<String>,
    http: Client,
}

impl GhcrRegistry {
    pub fn new(store: Arc<ImageStore>) -> Result<Self> {
        let token = std::env::var("GHCR_TOKEN").ok();
        let http = Client::builder()
            .https_only(true)
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()?;
        Ok(Self { store, token, http })
    }

    pub fn store(&self) -> &Arc<ImageStore> {
        &self.store
    }
}

minibox_macros::impl_as_any!(GhcrRegistry);

#[async_trait]
impl ImageRegistry for GhcrRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        let store_key = format!("ghcr.io/{}", name);
        self.store.has_image(&store_key, tag)
    }

    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata> {
        todo!("ghcr pull_image not yet implemented")
    }

    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        let store_key = format!("ghcr.io/{}", name);
        self.store.get_image_layers(&store_key, tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_registry() -> (GhcrRegistry, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(dir.path().join("images")).unwrap());
        let registry = GhcrRegistry::new(store).unwrap();
        (registry, dir)
    }

    #[tokio::test]
    async fn has_image_false_for_nonexistent() {
        let (registry, _dir) = make_registry();
        assert!(!registry.has_image("org/minibox-rust-ci", "stable").await);
    }

    #[test]
    fn get_image_layers_errors_for_nonexistent() {
        let (registry, _dir) = make_registry();
        let result = registry.get_image_layers("org/minibox-rust-ci", "stable");
        assert!(result.is_err());
    }

    #[test]
    fn store_key_includes_ghcr_prefix() {
        let (registry, _dir) = make_registry();
        // has_image must use "ghcr.io/org/image" as the store key, not just "org/image"
        // We verify this indirectly: storing under the wrong key should not be found
        assert!(!registry.store().has_image("org/minibox-rust-ci", "stable"));
        // and the correct key doesn't exist either (no image has been pulled)
        assert!(!registry.store().has_image("ghcr.io/org/minibox-rust-ci", "stable"));
    }
}
```

- [ ] **Step 3.3: Run tests — expect compile errors or failures**

```bash
cargo test -p minibox-lib adapters::ghcr -- --nocapture
```

Expected: compile errors until wired up. Fix import issues as you go.

- [ ] **Step 3.4: Wire `GhcrRegistry` into `adapters/mod.rs`**

Open `crates/minibox-lib/src/adapters/mod.rs` and add:
```rust
pub mod ghcr;
pub use ghcr::GhcrRegistry;
```

(Follow the existing pattern for how `DockerHubRegistry` is exported.)

- [ ] **Step 3.5: Implement `pull_image` for `GhcrRegistry`**

Replace the `todo!()` with the full auth + pull flow. Read `crates/minibox-lib/src/image/registry.rs` (`RegistryClient`) first — `GhcrRegistry.pull_image` follows the same orchestration but with a different auth endpoint.

The auth flow for ghcr.io:

```rust
async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata> {
    let store_key = format!("ghcr.io/{}", name);
    info!("ghcr: pulling {}:{}", store_key, tag);

    let token = self.authenticate(name).await?;

    // Get manifest (reuse OciManifest types from minibox-lib)
    let manifest = self.get_manifest(name, tag, &token).await?;
    // Note: get_manifest signature is (repo, tag_or_digest, token) — for manifest lists it
    // recursively calls self.get_manifest(repo, &desc.digest, token) to resolve to a single manifest.

    // Pull and extract each layer
    let mut layer_infos = Vec::new();
    for layer in &manifest.layers {
        let data = self.pull_layer(name, &layer.digest, &token).await?;
        self.store.store_layer(&store_key, tag, &layer.digest, &data[..])?;
        layer_infos.push(LayerInfo {
            digest: layer.digest.clone(),
            size: layer.size,
        });
    }

    self.store.store_manifest(&store_key, tag, &manifest)?;

    Ok(ImageMetadata {
        name: store_key,
        tag: tag.to_owned(),
        layers: layer_infos,
    })
}

async fn authenticate(&self, repo: &str) -> Result<String> {
    // 1. Send unauthenticated request to get WWW-Authenticate header
    let manifest_url = format!("https://ghcr.io/v2/{}/manifests/latest", repo);
    let probe = self.http.get(&manifest_url).send().await?;

    if probe.status() == reqwest::StatusCode::OK {
        // Public image, no auth needed — return empty token
        return Ok(String::new());
    }

    // 2. Parse WWW-Authenticate header
    let www_auth = probe
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let (realm, service, scope) = parse_www_authenticate(&www_auth)?;

    // 3. Exchange for JWT
    let mut req = self.http.get(&realm)
        .query(&[("service", &service), ("scope", &scope)]);

    if let Some(pat) = &self.token {
        req = req.bearer_auth(pat);
    }

    let resp: serde_json::Value = req.send().await?.json().await?;
    let token = resp["token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("ghcr: no token in auth response"))?
        .to_owned();

    debug!("ghcr: authenticated for {}", repo);
    Ok(token)
}
```

Add a pure-function helper for parsing the `WWW-Authenticate` header:

```rust
/// Parse `Bearer realm="...",service="...",scope="..."` into (realm, service, scope).
fn parse_www_authenticate(header: &str) -> Result<(String, String, String)> {
    let header = header.strip_prefix("Bearer ").unwrap_or(header);

    let get_param = |key: &str| -> Option<String> {
        let needle = format!(r#"{}=""#, key);
        let start = header.find(&needle)? + needle.len();
        let end = header[start..].find('"')? + start;
        Some(header[start..end].to_owned())
    };

    let realm = get_param("realm")
        .ok_or_else(|| anyhow::anyhow!("ghcr: no realm in WWW-Authenticate"))?;
    let service = get_param("service").unwrap_or_default();
    let scope = get_param("scope").unwrap_or_default();

    Ok((realm, service, scope))
}
```

Add a test for `parse_www_authenticate`:

```rust
#[test]
fn parse_www_authenticate_full() {
    let header = r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:org/image:pull""#;
    let (realm, service, scope) = parse_www_authenticate(header).unwrap();
    assert_eq!(realm, "https://ghcr.io/token");
    assert_eq!(service, "ghcr.io");
    assert_eq!(scope, "repository:org/image:pull");
}

#[test]
fn parse_www_authenticate_no_service() {
    let header = r#"Bearer realm="https://ghcr.io/token",scope="repository:org/image:pull""#;
    let (realm, service, _) = parse_www_authenticate(header).unwrap();
    assert_eq!(realm, "https://ghcr.io/token");
    assert_eq!(service, ""); // default empty
}
```

Implement `get_manifest` and `pull_layer` following the same pattern as `RegistryClient` in `image/registry.rs` but pointing to `ghcr.io`:

```rust
async fn get_manifest(&self, repo: &str, tag_or_digest: &str, token: &str)
    -> Result<crate::image::manifest::OciManifest>
{
    use crate::image::manifest::{
        ManifestResponse,
        MEDIA_TYPE_OCI_MANIFEST,
        MEDIA_TYPE_OCI_INDEX,
        MEDIA_TYPE_DOCKER_MANIFEST,
        MEDIA_TYPE_DOCKER_MANIFEST_LIST,
    };

    const ACCEPT: &str = concat!(
        "application/vnd.oci.image.manifest.v1+json, ",
        "application/vnd.oci.image.index.v1+json, ",
        "application/vnd.docker.distribution.manifest.v2+json, ",
        "application/vnd.docker.distribution.manifest.list.v2+json"
    );
    // Alternatively, build from the four constants at runtime:
    // let accept = format!("{}, {}, {}, {}",
    //     MEDIA_TYPE_OCI_MANIFEST, MEDIA_TYPE_OCI_INDEX,
    //     MEDIA_TYPE_DOCKER_MANIFEST, MEDIA_TYPE_DOCKER_MANIFEST_LIST);

    let url = format!("https://ghcr.io/v2/{}/manifests/{}", repo, tag_or_digest);
    let mut req = self.http.get(&url).header("Accept", ACCEPT);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }

    let resp = req.send().await?;
    anyhow::ensure!(resp.status().is_success(),
        "ghcr: manifest fetch failed: {}", resp.status());

    // Extract Content-Type before consuming body
    let content_type = resp
        .headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let bytes = resp.bytes().await?;
    let manifest_resp = ManifestResponse::parse(&bytes, &content_type)
        .with_context(|| format!("ghcr: parsing manifest for {}:{}", repo, tag_or_digest))?;

    match manifest_resp {
        ManifestResponse::Single(m) => Ok(m),
        ManifestResponse::List(list) => {
            let desc = list
                .find_linux_amd64()
                .ok_or_else(|| anyhow::anyhow!("ghcr: no linux/amd64 manifest in list for {}:{}", repo, tag_or_digest))?;
            // Recursively fetch the single-arch manifest by digest
            Box::pin(self.get_manifest(repo, &desc.digest, token)).await
        }
    }
}

async fn pull_layer(&self, name: &str, digest: &str, token: &str) -> Result<bytes::Bytes> {
    use crate::image::registry::MAX_LAYER_SIZE;

    let url = format!("https://ghcr.io/v2/{}/blobs/{}", name, digest);
    let mut req = self.http.get(&url);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }

    let resp = req.send().await?;
    anyhow::ensure!(resp.status().is_success(),
        "ghcr: blob fetch failed for {}: {}", digest, resp.status());

    if let Some(len) = resp.content_length() {
        anyhow::ensure!(len <= MAX_LAYER_SIZE,
            "ghcr: layer {} too large: {} bytes", digest, len);
    }

    let bytes = resp.bytes().await?;
    anyhow::ensure!(bytes.len() as u64 <= MAX_LAYER_SIZE,
        "ghcr: layer {} too large after download", digest);

    Ok(bytes)
}
```

**Note on multi-arch manifests:** `get_manifest` handles manifest lists by calling `list.find_linux_amd64()` and then recursively calling `self.get_manifest(repo, &desc.digest, token)` to fetch the resolved single-arch manifest. The recursive call is wrapped in `Box::pin(...)` because async recursion requires heap allocation.

- [ ] **Step 3.6: Run tests — expect all pass**

```bash
cargo test -p minibox-lib adapters::ghcr -- --nocapture
cargo clippy -p minibox-lib -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 3.7: Commit**

```bash
git add crates/minibox-lib/src/adapters/ghcr.rs \
        crates/minibox-lib/src/adapters/mod.rs
git commit -m "feat(minibox-lib): add GhcrRegistry adapter for ghcr.io"
```

---

## Task 4: Registry Selection in Handler

**Files:**
- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/miniboxd/src/main.rs`

Add `ghcr_registry` to `HandlerDependencies`, add `select_registry()`, update `handle_pull` and `handle_run` to route via `ImageRef`.

- [ ] **Step 4.1: Read `handler.rs` and `main.rs` completely before editing**

```bash
cat crates/daemonbox/src/handler.rs
cat crates/miniboxd/src/main.rs
```

- [ ] **Step 4.2: Write failing tests for `select_registry`**

These tests must FAIL before `select_registry` is implemented. Add a stub with `unimplemented!()` first so the tests compile:

```rust
// Temporary stub so tests compile (replace with real impl in Step 4.4):
fn select_registry<'a>(
    _image_ref: &minibox_lib::image::reference::ImageRef,
    _docker: &'a dyn minibox_lib::domain::ImageRegistry,
    _ghcr: &'a dyn minibox_lib::domain::ImageRegistry,
) -> &'a dyn minibox_lib::domain::ImageRegistry {
    unimplemented!()
}
```

Then add tests that call `select_registry` directly (not through `HandlerDependencies`):

```rust
#[cfg(test)]
mod tests {
    // ... existing tests ...

    #[test]
    fn select_registry_routes_ghcr() {
        use minibox_lib::image::reference::ImageRef;
        use std::sync::Arc;

        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(minibox_lib::image::ImageStore::new(temp.path().join("images")).unwrap());
        let docker: Arc<dyn minibox_lib::domain::ImageRegistry> =
            Arc::new(minibox_lib::adapters::DockerHubRegistry::new(Arc::clone(&store)).unwrap());
        let ghcr: Arc<dyn minibox_lib::domain::ImageRegistry> =
            Arc::new(minibox_lib::adapters::GhcrRegistry::new(Arc::clone(&store)).unwrap());

        let ghcr_ref = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();

        // select_registry must return the ghcr registry (same object pointer)
        let selected = select_registry(&ghcr_ref, docker.as_ref(), ghcr.as_ref());
        assert!(std::ptr::eq(
            selected as *const dyn minibox_lib::domain::ImageRegistry as *const (),
            ghcr.as_ref() as *const dyn minibox_lib::domain::ImageRegistry as *const ()
        ));
    }

    #[test]
    fn select_registry_routes_docker() {
        use minibox_lib::image::reference::ImageRef;
        use std::sync::Arc;

        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(minibox_lib::image::ImageStore::new(temp.path().join("images")).unwrap());
        let docker: Arc<dyn minibox_lib::domain::ImageRegistry> =
            Arc::new(minibox_lib::adapters::DockerHubRegistry::new(Arc::clone(&store)).unwrap());
        let ghcr: Arc<dyn minibox_lib::domain::ImageRegistry> =
            Arc::new(minibox_lib::adapters::GhcrRegistry::new(Arc::clone(&store)).unwrap());

        let docker_ref = ImageRef::parse("alpine").unwrap();

        // select_registry must return the docker registry (same object pointer)
        let selected = select_registry(&docker_ref, docker.as_ref(), ghcr.as_ref());
        assert!(std::ptr::eq(
            selected as *const dyn minibox_lib::domain::ImageRegistry as *const (),
            docker.as_ref() as *const dyn minibox_lib::domain::ImageRegistry as *const ()
        ));
    }
}
```

These tests panic at the `unimplemented!()` stub — which is the expected failure before Step 4.4.

- [ ] **Step 4.3: Add `ghcr_registry` to `HandlerDependencies`**

In `crates/daemonbox/src/handler.rs`, find the `HandlerDependencies` struct and add:

```rust
#[derive(Clone)]
pub struct HandlerDependencies {
    pub registry: DynImageRegistry,
    pub ghcr_registry: DynImageRegistry,   // ← add this
    pub filesystem: DynFilesystemProvider,
    pub resource_limiter: DynResourceLimiter,
    pub runtime: DynContainerRuntime,
    pub containers_base: PathBuf,
    pub run_containers_base: PathBuf,
}
```

- [ ] **Step 4.4: Add `select_registry` function in `handler.rs`**

The function takes the two registries directly (so the tests from Step 4.2 can call it without a full `HandlerDependencies`):

```rust
use minibox_lib::image::reference::ImageRef;

fn select_registry<'a>(
    image_ref: &ImageRef,
    docker: &'a dyn minibox_lib::domain::ImageRegistry,
    ghcr: &'a dyn minibox_lib::domain::ImageRegistry,
) -> &'a dyn minibox_lib::domain::ImageRegistry {
    if image_ref.registry == "ghcr.io" {
        ghcr
    } else {
        docker
    }
}
```

At call sites in `handle_pull` and `handle_run`, pass `deps.registry.as_ref()` and `deps.ghcr_registry.as_ref()`:

```rust
let registry = select_registry(&image_ref, deps.registry.as_ref(), deps.ghcr_registry.as_ref());
```

- [ ] **Step 4.5: Update `handle_pull` to use `ImageRef`**

Find `handle_pull` in `handler.rs`. It currently receives `image: String, tag: Option<String>`.

Replace the image name normalization logic with `ImageRef::parse`:

```rust
pub async fn handle_pull(
    image: String,
    tag: Option<String>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    // Reconstruct full reference and parse
    let full_ref = match &tag {
        Some(t) if !t.is_empty() => format!("{}:{}", image, t),
        _ => image.clone(),
    };
    let image_ref = match ImageRef::parse(&full_ref) {
        Ok(r) => r,
        Err(e) => return DaemonResponse::Error { message: e.to_string() },
    };

    let registry = select_registry(&image_ref, deps.registry.as_ref(), deps.ghcr_registry.as_ref());
    let name = image_ref.cache_name();   // e.g. "library/alpine" or "ghcr.io/org/image"
    let tag = &image_ref.tag;

    info!(image = %name, tag = %tag, registry = %image_ref.registry, "pull: starting");

    match registry.pull_image(&image_ref.repository(), tag).await {
        Ok(_) => DaemonResponse::Success {
            message: format!("pulled {}:{}", name, tag),
        },
        Err(e) => DaemonResponse::Error { message: e.to_string() },
    }
}
```

**Note:** `registry.pull_image()` is called with `image_ref.repository()` (the API name, e.g., `library/alpine`) but the store operations inside each registry use the registry-qualified key (`cache_name()`). `DockerHubRegistry` will need a small update (Step 4.7) to use `cache_name` for storage.

- [ ] **Step 4.6: Update `run_inner` to use `ImageRef`**

In `handle_run` / `run_inner`, replace the manual `library/` normalization:

```rust
// OLD (remove this):
let image_name = if image.contains('/') {
    image.clone()
} else {
    format!("library/{}", image)
};

// NEW:
let full_ref = match &tag {
    Some(t) if !t.is_empty() => format!("{}:{}", image, t),
    _ => image.clone(),
};
let image_ref = ImageRef::parse(&full_ref)?;
let registry = select_registry(&image_ref, deps.registry.as_ref(), deps.ghcr_registry.as_ref());
let api_name = image_ref.repository();     // for registry API calls
let store_name = image_ref.cache_name();   // for ImageStore lookups
```

Then update the `has_image`, `pull_image`, and `get_image_layers` calls to use `api_name` and `store_name` appropriately. Pass `api_name` to the registry calls (the registry prepends its own prefix for storage).

- [ ] **Step 4.7: Update `DockerHubRegistry` to accept `pull_image` via `api_name`**

`DockerHubRegistry.pull_image(name, tag)` currently uses `name` directly for storage. Since `handle_pull` now calls it with `image_ref.repository()` (e.g., `library/alpine` for docker.io), no change is needed — the repository path IS already the correct store key for docker.io (backward compat). Verify this by reading `DockerHubRegistry::pull_image` and confirming it passes `name` directly to `store.store_layer(name, tag, ...)`.

- [ ] **Step 4.8: Wire `GhcrRegistry` in `miniboxd/src/main.rs`**

Find where `DockerHubRegistry` is initialized and add `GhcrRegistry` alongside it:

```rust
use minibox_lib::adapters::{DockerHubRegistry, GhcrRegistry};

// In main():
let ghcr_registry = Arc::new(
    GhcrRegistry::new(Arc::clone(&state.image_store))
        .context("creating GHCR registry adapter")?,
);

let deps = Arc::new(HandlerDependencies {
    registry: Arc::clone(&registry) as DynImageRegistry,
    ghcr_registry: ghcr_registry as DynImageRegistry,   // ← add
    filesystem: Arc::new(OverlayFilesystem::new()),
    resource_limiter: Arc::new(CgroupV2Limiter::new()),
    runtime: Arc::new(LinuxNamespaceRuntime::new()),
    containers_base: PathBuf::from(&containers_dir),
    run_containers_base: PathBuf::from(&run_containers_dir),
});
```

Also update all test helpers in `crates/miniboxd/tests/` and `crates/daemonbox/tests/` that construct `HandlerDependencies` to include `ghcr_registry`. Use a `DockerHubRegistry` for both in tests (they can share the store):

```rust
let deps = Arc::new(HandlerDependencies {
    registry: Arc::new(DockerHubRegistry::new(Arc::clone(&state.image_store)).unwrap()),
    ghcr_registry: Arc::new(DockerHubRegistry::new(Arc::clone(&state.image_store)).unwrap()),
    // ... rest unchanged
});
```

- [ ] **Step 4.9: Run all tests — expect pass (macOS-safe subset)**

```bash
cargo test -p minibox-lib -p daemonbox --lib -- --nocapture
cargo test -p daemonbox --test handler_tests -- --nocapture
cargo test -p daemonbox --test conformance_tests -- --nocapture
cargo clippy -p minibox-lib -p daemonbox -p miniboxd -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 4.10: Commit**

```bash
git add crates/daemonbox/src/handler.rs \
        crates/miniboxd/src/main.rs \
        crates/miniboxd/tests/
git commit -m "feat(daemonbox): add GhcrRegistry, select_registry routing by image prefix"
```

---

## Task 4b: Update `DockerHubRegistry` to Accept `ImageRef`

**Files:**
- Modify: `crates/minibox-lib/src/adapters/registry.rs`
- Modify: `crates/minibox-lib/src/domain.rs` (trait signature)

The `ImageRegistry` trait currently has `pull_image(name: &str, tag: &str)`. Since `handle_pull` now has an `ImageRef`, pass it directly to the registry so each registry can control how it maps ref to API path and storage key.

⚠️ **This is a breaking trait change** — all `impl ImageRegistry` blocks must be updated.

- [ ] **Step 4b.1: Update the `ImageRegistry` trait in `domain.rs`**

Change:
```rust
async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata>;
```
To:
```rust
async fn pull_image(&self, image_ref: &ImageRef) -> Result<ImageMetadata>;
```

where `ImageRef` is imported from `crate::image::reference::ImageRef`.

- [ ] **Step 4b.2: Update `DockerHubRegistry` in `registry.rs`**

In `crates/minibox-lib/src/adapters/registry.rs`, update `pull_image` to accept `image_ref: &ImageRef`:

```rust
async fn pull_image(&self, image_ref: &ImageRef) -> Result<ImageMetadata> {
    // Use image_ref.repository() for the Docker Hub API path (e.g. "library/alpine")
    let api_name = image_ref.repository();
    // Use image_ref.cache_name() for the ImageStore key (e.g. "library/alpine" for docker.io)
    let store_name = image_ref.cache_name();
    let tag = &image_ref.tag;

    // ... rest of pull logic using api_name for HTTP calls and store_name for store.store_layer(...) ...
}
```

For backward compatibility, `cache_name()` for docker.io returns `library/alpine` (no registry prefix), which matches the existing on-disk layout.

- [ ] **Step 4b.3: Update `GhcrRegistry` in `ghcr.rs`**

Similarly update `GhcrRegistry::pull_image` to accept `image_ref: &ImageRef`:

```rust
async fn pull_image(&self, image_ref: &ImageRef) -> Result<ImageMetadata> {
    let store_key = image_ref.cache_name(); // e.g. "ghcr.io/org/image"
    let repo = image_ref.repository();      // e.g. "org/image" (for ghcr.io API path)
    let tag = &image_ref.tag;

    // ... authenticate(repo), get_manifest(repo, tag, token), pull layers ...
    // Use store_key for all self.store.store_layer(store_key, ...) calls
}
```

- [ ] **Step 4b.4: Update all other `impl ImageRegistry` blocks**

The following also implement `ImageRegistry` and must be updated:
- `MockImageRegistry` in `crates/minibox-lib/src/adapters/mocks.rs` — update signature; mock can ignore the ref or use `image_ref.cache_name()` as a key
- Any other adapters found by `grep -r "impl ImageRegistry"` in the workspace

- [ ] **Step 4b.5: Update all call sites in `handler.rs`**

`handle_pull` and `run_inner` currently call `registry.pull_image(&image_ref.repository(), tag)`. Change to:
```rust
registry.pull_image(&image_ref).await
```

- [ ] **Step 4b.6: Run tests**

```bash
cargo test -p minibox-lib -p daemonbox --lib -- --nocapture
cargo clippy -p minibox-lib -p daemonbox -p miniboxd -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 4b.7: Commit**

```bash
git add crates/minibox-lib/src/adapters/registry.rs \
        crates/minibox-lib/src/adapters/ghcr.rs \
        crates/minibox-lib/src/adapters/mocks.rs \
        crates/minibox-lib/src/domain.rs \
        crates/daemonbox/src/handler.rs
git commit -m "refactor(minibox-lib): pull_image accepts ImageRef directly, enabling registry-specific routing"
```

---

## Task 5: Protocol Streaming Types

**Files:**
- Modify: `crates/minibox-lib/src/protocol.rs`

Add `ContainerOutput`, `ContainerStopped`, `OutputStreamKind`, and `ephemeral` to `DaemonRequest::Run`. All changes are additive — existing clients that don't send `ephemeral` get `false` via `#[serde(default)]`.

- [ ] **Step 5.1: Write failing tests at the bottom of `protocol.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_request_defaults_ephemeral_false() {
        let json = r#"{"type":"Run","image":"alpine","tag":"latest","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;
        let req: DaemonRequest = serde_json::from_str(json).unwrap();
        match req {
            DaemonRequest::Run { ephemeral, .. } => assert!(!ephemeral),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn run_request_explicit_ephemeral_true() {
        let json = r#"{"type":"Run","image":"alpine","tag":"latest","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null,"ephemeral":true}"#;
        let req: DaemonRequest = serde_json::from_str(json).unwrap();
        match req {
            DaemonRequest::Run { ephemeral, .. } => assert!(ephemeral),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn container_output_roundtrip() {
        let msg = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "aGVsbG8=".to_owned(), // base64("hello")
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: DaemonResponse = serde_json::from_str(&json).unwrap();
        match back {
            DaemonResponse::ContainerOutput { stream, data } => {
                assert_eq!(stream, OutputStreamKind::Stdout);
                assert_eq!(data, "aGVsbG8=");
            }
            _ => panic!("expected ContainerOutput"),
        }
    }

    #[test]
    fn container_stopped_roundtrip() {
        let msg = DaemonResponse::ContainerStopped { exit_code: 42 };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"exit_code\":42"));
        let back: DaemonResponse = serde_json::from_str(&json).unwrap();
        match back {
            DaemonResponse::ContainerStopped { exit_code } => assert_eq!(exit_code, 42),
            _ => panic!("expected ContainerStopped"),
        }
    }

    #[test]
    fn output_stream_kind_serde_lowercase() {
        let stdout = serde_json::to_string(&OutputStreamKind::Stdout).unwrap();
        let stderr = serde_json::to_string(&OutputStreamKind::Stderr).unwrap();
        assert_eq!(stdout, r#""stdout""#);
        assert_eq!(stderr, r#""stderr""#);
    }
}
```

- [ ] **Step 5.2: Run tests — expect compile errors**

```bash
cargo test -p minibox-lib protocol -- --nocapture
```

- [ ] **Step 5.3: Add new types to `protocol.rs`**

Add `ephemeral: bool` to `DaemonRequest::Run`:

```rust
// In DaemonRequest enum, update Run variant:
Run {
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    #[serde(default)]
    ephemeral: bool,
},
```

Add `OutputStreamKind` and new `DaemonResponse` variants:

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputStreamKind {
    Stdout,
    Stderr,
}

// In DaemonResponse enum, add:
/// A chunk of output from a running container's stdout or stderr.
/// Data is base64-encoded raw bytes. Sent zero or more times before ContainerStopped.
ContainerOutput {
    stream: OutputStreamKind,
    data: String,
},

/// Terminal message after a streaming run. Exit code of the container process.
/// Exactly one per streaming run; signals end of ContainerOutput stream.
ContainerStopped {
    exit_code: i32,
},
```

- [ ] **Step 5.4: Run tests — expect all pass**

```bash
cargo test -p minibox-lib protocol -- --nocapture
cargo clippy -p minibox-lib -- -D warnings
cargo fmt --all --check
```

Fix any match arms in the codebase that need updating for the new enum variants (the compiler will tell you where).

- [ ] **Step 5.5: Commit**

```bash
git add crates/minibox-lib/src/protocol.rs
git commit -m "feat(protocol): add ContainerOutput/ContainerStopped streaming types, ephemeral flag"
```

---

## Task 6: Container Stdout Pipe + Daemon Streaming Loop

**Files:**
- Modify: `crates/minibox-lib/src/domain.rs`
- Modify: `crates/minibox-lib/src/container/process.rs`
- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`

⚠️ **Linux only:** This task involves `pipe(2)`, `dup2(2)`, and process fork. Tests require Linux. Run them in the integration test environment.

Before touching any code, read these files completely:

```bash
cat crates/minibox-lib/src/container/process.rs
cat crates/minibox-lib/src/domain.rs
cat crates/daemonbox/src/server.rs
cat crates/daemonbox/src/handler.rs
```

- [ ] **Step 6.1: Extend `ContainerRuntime::spawn_process` to return an output reader**

In `crates/minibox-lib/src/domain.rs`, update the trait and the `ContainerSpawnConfig` struct (note: the existing type is named `ContainerSpawnConfig`, not `ContainerConfig` — do not rename it):

```rust
use std::os::fd::OwnedFd;

/// Returned by ContainerRuntime::spawn_process.
pub struct SpawnResult {
    pub pid: u32,
    /// Present when `ContainerSpawnConfig::capture_output` was true.
    /// The read end of a pipe connected to the container's stdout+stderr.
    pub output_reader: Option<OwnedFd>,
}

// Add to the existing ContainerSpawnConfig struct (do NOT rename):
pub struct ContainerSpawnConfig {
    // ... existing fields unchanged ...
    /// When true, container stdout+stderr are captured to a pipe.
    /// The read end is returned in SpawnResult::output_reader.
    pub capture_output: bool,
    /// stdout_fd override — used internally when output_reader pipe is set up.
    pub stdout_fd: Option<OwnedFd>,
}
```

Update the trait signature:
```rust
pub trait ContainerRuntime: AsAny + Send + Sync {
    fn spawn_process(&self, config: ContainerSpawnConfig) -> Result<SpawnResult>;
}
```

Fix ALL `impl ContainerRuntime` blocks to match the new signature. The following adapters need updating:
- `LinuxNamespaceRuntime` (in `crates/minibox-lib/src/container/process.rs`) — full pipe implementation (Step 6.2)
- `ProotRuntime` (in `crates/minibox-lib/src/adapters/`) — return `SpawnResult { pid, output_reader: None }`
- `ColimaRuntime` (in `crates/minibox-lib/src/adapters/`) — return `SpawnResult { pid, output_reader: None }`
- `WslRuntime` (in `crates/minibox-lib/src/adapters/`) — return `SpawnResult { pid, output_reader: None }`
- `DockerDesktopRuntime` (in `crates/minibox-lib/src/adapters/`) — return `SpawnResult { pid, output_reader: None }`
- `MockContainerRuntime` (in `crates/minibox-lib/src/adapters/mocks.rs`) — return `SpawnResult { pid, output_reader: None }`

- [ ] **Step 6.2: Set up stdout/stderr pipe in `process.rs`**

In `crates/minibox-lib/src/container/process.rs`, in the `spawn_process` implementation for `LinuxNamespaceRuntime`:

```rust
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use nix::unistd::{pipe, dup2, close};

// Before clone():
let (read_fd, write_fd) = if config.capture_output {
    let (r, w) = pipe().context("creating output pipe")?;
    (Some(r), Some(w))
} else {
    (None, None)
};

// In the child process (after clone, before execvp):
if config.capture_output {
    if let Some(ref write_fd) = write_fd {
        // Redirect stdout and stderr to the write end of the pipe
        dup2(write_fd.as_raw_fd(), libc::STDOUT_FILENO)
            .context("dup2 stdout")?;
        dup2(write_fd.as_raw_fd(), libc::STDERR_FILENO)
            .context("dup2 stderr")?;
        // Close the original write fd (now duplicated into stdout/stderr slots)
        // The read_fd should already not be inherited by child due to O_CLOEXEC,
        // but we close write_fd explicitly after dup2.
    }
    // Close read end in child (parent will read from it)
    if let Some(ref read_fd) = read_fd {
        close(read_fd.as_raw_fd()).ok();
    }
}

// After clone() returns to parent:
// Close the write end in the parent (child has it via dup2, we don't need it)
if config.capture_output {
    drop(write_fd); // close write end
}

// Return result
let output_reader = if config.capture_output { read_fd } else { None };
Ok(SpawnResult { pid, output_reader })
```

**Critical:** Set `O_CLOEXEC` on the read_fd so it's not inherited by the child. Use `nix::fcntl::OFlag::O_CLOEXEC` with the pipe flags:

```rust
use nix::fcntl::OFlag;
let (r, w) = nix::unistd::pipe2(OFlag::O_CLOEXEC).context("creating output pipe")?;
```

Use `pipe2(O_CLOEXEC)` instead of `pipe()` — this atomically sets close-on-exec on both ends. The child will not inherit them across `execvp`.

However: the child needs the write end to dup2 into stdout/stderr BEFORE execvp. The child gets a copy of the write fd before exec. After dup2, the original write fd slot in the child should be closed. After execvp, O_CLOEXEC closes it automatically. This is the correct flow.

- [ ] **Step 6.3: Add streaming loop in `handler.rs`**

In `handle_run` (or `run_inner`), detect the `ephemeral` flag and switch to streaming mode:

```rust
if ephemeral {
    // Ephemeral/streaming path: capture output, stream back to client via tx
    return handle_run_streaming(image, tag, command, state, deps, tx).await;
}
// Non-ephemeral: existing behavior unchanged
```

For the streaming path, add a new helper:

```rust
/// Streaming run: sends ContainerOutput chunks then ContainerStopped.
/// Used when ephemeral=true. The caller (server) reads from `tx` and writes to socket.
pub async fn handle_run_streaming(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: tokio::sync::mpsc::Sender<DaemonResponse>,
) {
    // ... ImageRef parsing, registry selection, layer fetching (same as existing run_inner) ...

    // spawn_process with capture_output = true
    let config = ContainerSpawnConfig {
        // ... existing fields ...
        capture_output: true,
        stdout_fd: None,  // will be filled by spawn_process pipe setup
    };

    let spawn_result = match deps.runtime.spawn_process(config) {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(DaemonResponse::Error { message: e.to_string() }).await;
            return;
        }
    };

    let output_reader = spawn_result.output_reader.unwrap();
    let pid = spawn_result.pid;

    // Read stdout/stderr in a blocking task (OwnedFd is not async)
    let tx_clone = tx.clone();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        use base64::Engine;
        let mut file = unsafe { std::fs::File::from_raw_fd(output_reader.into_raw_fd()) };
        let mut buf = [0u8; 4096];

        loop {
            match file.read(&mut buf) {
                Ok(0) => break, // EOF — process exited, pipe closed
                Ok(n) => {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    let _ = tx_clone.blocking_send(DaemonResponse::ContainerOutput {
                        stream: OutputStreamKind::Stdout,
                        data: encoded,
                    });
                }
                Err(_) => break,
            }
        }
    });

    // Wait for process exit (reuse daemon_wait_for_exit pattern)
    let exit_code = wait_for_pid(pid).await;

    // Auto-remove if ephemeral
    // ... cleanup overlay dirs, remove state entry ...

    let _ = tx.send(DaemonResponse::ContainerStopped { exit_code }).await;
}

/// Wait for a child process to exit and return its exit code.
///
/// Uses `nix::sys::wait::waitpid` in a blocking task to avoid blocking
/// the async executor.
async fn wait_for_pid(pid: u32) -> i32 {
    tokio::task::spawn_blocking(move || {
        use nix::sys::wait::{waitpid, WaitStatus};
        use nix::unistd::Pid;
        match waitpid(Pid::from_raw(pid as i32), None) {
            Ok(WaitStatus::Exited(_, code)) => code,
            Ok(WaitStatus::Signaled(_, sig, _)) => -(sig as i32),
            Ok(_) => 0,
            Err(e) => {
                tracing::warn!(pid = pid, error = %e, "wait_for_pid: waitpid failed");
                -1
            }
        }
    })
    .await
    .unwrap_or(-1)
}
```

Note: `base64` crate needs to be added to `daemonbox/Cargo.toml`. Check if it's already a workspace dep; if not, add `base64 = "0.22"`.

- [ ] **Step 6.4: Update `server.rs` to support streaming responses**

Read `server.rs` to understand its current `handle_connection` loop. The server currently:
1. Reads one request
2. Dispatches to handler (gets one `DaemonResponse`)
3. Writes one response
4. (Loops to next request or closes)

For streaming, change step 2-3 to pass a channel:

```rust
// In handle_connection():
let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(64);

// Spawn handler with sender
tokio::spawn(async move {
    dispatch_request(request, state, deps, tx).await;
});

// Write responses as they arrive
while let Some(response) = rx.recv().await {
    let is_terminal = matches!(
        &response,
        DaemonResponse::ContainerStopped { .. } | DaemonResponse::Error { .. }
    );
    let json = serde_json::to_string(&response)
        .unwrap_or_else(|e| format!("{{\"type\":\"Error\",\"message\":\"{e}}}\"\n"));
    writer.write_all(format!("{json}\n").as_bytes()).await?;
    writer.flush().await?;
    if is_terminal {
        break;
    }
}
```

For non-streaming responses, the handler sends exactly one message (existing `DaemonResponse` variant) and the loop exits on the first non-streaming message. Add a helper:

```rust
fn is_streaming_terminal(r: &DaemonResponse) -> bool {
    matches!(r, DaemonResponse::ContainerStopped { .. })
}

fn is_single_response(r: &DaemonResponse) -> bool {
    !matches!(r, DaemonResponse::ContainerOutput { .. })
}
```

**Backward compat:** Non-ephemeral requests send exactly one response (e.g., `ContainerCreated`), so the loop exits after one message — same behavior as before.

- [ ] **Step 6.5: Integration test on Linux**

This step requires Linux + root. Add an integration test in `crates/miniboxd/tests/` (or `daemonbox/tests/`) marked `#[ignore]`:

```rust
#[tokio::test]
#[ignore = "requires Linux, root, miniboxd running"]
async fn ephemeral_run_streams_output() {
    // Verifies that an ephemeral container run sends ContainerOutput then ContainerStopped.
    // Test by constructing deps directly and calling handle_run_streaming.
    // See existing integration_tests.rs for the pattern.

    require_root();
    require_cgroups_v2();

    // ... setup deps with real adapters ...
    // ... call handle_run_streaming with image="alpine", command=["echo", "hello"] ...
    // ... collect all DaemonResponse from channel ...
    // ... assert at least one ContainerOutput with decoded data containing "hello" ...
    // ... assert last message is ContainerStopped { exit_code: 0 } ...
}
```

Run it manually on Linux:
```bash
sudo cargo test -p miniboxd --test integration_tests ephemeral_run_streams_output -- --ignored --nocapture
```

- [ ] **Step 6.6: Commit**

```bash
git add crates/minibox-lib/src/domain.rs \
        crates/minibox-lib/src/container/process.rs \
        crates/daemonbox/src/handler.rs \
        crates/daemonbox/src/server.rs \
        crates/miniboxd/tests/
git commit -m "feat(daemonbox): pipe container stdout/stderr, stream ContainerOutput to client"
```

---

## Task 7: CLI Streaming Handler + Final Wire-Up

**Files:**
- Modify: `crates/minibox-cli/src/commands/run.rs`

The CLI currently reads one response. Update it to read a stream until `ContainerStopped`, writing decoded chunks to stdout/stderr and exiting with the container's exit code.

- [ ] **Step 7.1: Read `run.rs` and the socket client code before editing**

```bash
cat crates/minibox-cli/src/commands/run.rs
# Also find send_request implementation
grep -r "send_request" crates/minibox-cli/src/
```

- [ ] **Step 7.2: Write failing test for the streaming response handler**

Add to `run.rs` or a test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use minibox_lib::protocol::{DaemonResponse, OutputStreamKind};

    #[test]
    fn decode_output_chunk() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"hello world\n");
        let response = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: encoded,
        };
        // verify decode roundtrip
        if let DaemonResponse::ContainerOutput { data, .. } = response {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&data)
                .unwrap();
            assert_eq!(decoded, b"hello world\n");
        }
    }
}
```

- [ ] **Step 7.3: Update `run.rs` to stream responses**

Find the current `execute()` function. It calls `send_request()` and expects a single `DaemonResponse`. Replace it with the corrected version that:
1. Connects to the socket using `super::socket_path()` (already defined in `crates/minibox-cli/src/commands/mod.rs` — do NOT use `minibox_lib::get_socket_path()` which does not exist)
2. Sends the `RunContainer` request
3. Reads one JSON line at a time in a loop
4. Decodes and writes `ContainerOutput` chunks to stdout/stderr immediately (no buffering)
5. On `ContainerStopped`, breaks and calls `std::process::exit(exit_code)`

```rust
pub async fn execute(
    image: String,
    tag: String,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
) -> Result<()> {
    use base64::Engine;
    use minibox_lib::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let request = DaemonRequest::Run {
        image: image.clone(),
        tag: Some(tag.clone()),
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral: true,   // All CLI runs are ephemeral + streaming
    };

    // Connect to daemon using the same socket_path() function used by other commands
    let path = super::socket_path();
    let mut stream = UnixStream::connect(&path).await
        .with_context(|| format!("connecting to daemon at {:?}", path))?;

    // Send the request
    let json = serde_json::to_string(&request)?;
    stream.write_all(format!("{json}\n").as_bytes()).await?;
    stream.flush().await?;

    // Read streaming responses inline — no intermediate Vec
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        if line.is_empty() {
            break; // EOF — daemon closed connection
        }
        let response: DaemonResponse = serde_json::from_str(line.trim())
            .with_context(|| format!("parsing daemon response: {:?}", line.trim()))?;

        match response {
            DaemonResponse::ContainerCreated { id } => {
                // Non-streaming daemon (backward compat) — print ID and return
                println!("{id}");
                return Ok(());
            }
            DaemonResponse::ContainerOutput { stream: kind, data } => {
                // Decode and write immediately — no buffering
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .unwrap_or_default();
                match kind {
                    OutputStreamKind::Stdout => {
                        use std::io::Write;
                        std::io::stdout().write_all(&bytes)?;
                        std::io::stdout().flush()?;
                    }
                    OutputStreamKind::Stderr => {
                        use std::io::Write;
                        std::io::stderr().write_all(&bytes)?;
                        std::io::stderr().flush()?;
                    }
                }
            }
            DaemonResponse::ContainerStopped { exit_code } => {
                // Terminal message — exit with container's exit code
                std::process::exit(exit_code);
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            _ => {}
        }
    }

    Ok(())
}
```

**Key points:**
- `super::socket_path()` is the private function in `commands/mod.rs` — it reads `MINIBOX_SOCKET_PATH` env var or returns the default path. Use it directly; do not duplicate the logic.
- Responses are processed one line at a time as they arrive. No `Vec<DaemonResponse>` is accumulated.
- `ContainerStopped` triggers `std::process::exit(exit_code)` directly — this propagates the container exit code to the shell.

**Note:** If `send_request` already exists in the CLI codebase, read it to understand the existing socket path and I/O patterns before editing.

- [ ] **Step 7.4: Add `base64` dependency to CLI and daemonbox `Cargo.toml`**

```bash
# Check if base64 is in workspace.dependencies first:
grep "base64" Cargo.toml

# If not, add to workspace:
# [workspace.dependencies]
# base64 = "0.22"

# Then reference in crates/minibox-cli/Cargo.toml and crates/daemonbox/Cargo.toml:
# [dependencies]
# base64.workspace = true
```

- [ ] **Step 7.5: Run all macOS-safe tests**

```bash
cargo test -p minibox-cli -p minibox-lib -p daemonbox --lib -- --nocapture
cargo clippy -p minibox-cli -p minibox-lib -p daemonbox -p miniboxd -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 7.6: Smoke test on Linux (manual)**

On the Linux integration host:

```bash
# Start daemon
sudo ./target/release/miniboxd &

# Pull a public image from ghcr.io
sudo ./target/release/minibox pull ghcr.io/org/minibox-rust-ci:stable

# Streaming run (smoke test)
sudo ./target/release/minibox run ghcr.io/org/minibox-rust-ci:stable -- rustc --version
# Expected: "rustc X.Y.Z (... ...)" printed, exit 0

# Verify no residual containers
sudo ./target/release/minibox ps
# Expected: empty list
```

- [ ] **Step 7.7: Commit**

```bash
git add crates/minibox-cli/src/ Cargo.toml crates/minibox-cli/Cargo.toml crates/daemonbox/Cargo.toml
git commit -m "feat(minibox-cli): stream ContainerOutput to terminal, exit with container exit code"
```

---

## Quality Gates After Every Task

```bash
# Run on macOS after each task (Linux-only tests will be skipped automatically)
cargo xtask pre-commit   # fmt-check + lint + build-release

# On Linux integration host (Tasks 6–7):
cargo xtask test-unit
just test-integration    # cgroups tests
just test-e2e-suite      # daemon+CLI e2e
```

## Common Pitfalls

| Pitfall | Fix |
|---------|-----|
| `set_var` compile error in tests | Wrap in `unsafe {}`, use `static Mutex<()>` guard |
| `clippy::crate_in_macro_def` on `as_any!` | Add `#[allow(clippy::crate_in_macro_def)]`, do not change to `$crate` |
| `pivot_root` EINVAL | Must call `mount("", "/", MS_REC\|MS_PRIVATE)` before bind-mount |
| `close_extra_fds` panic | Collect FD numbers into Vec before iterating, then close |
| `pipe2` O_CLOEXEC on child write end | Child must `dup2` write_fd → stdout/stderr before `execvp`; close original after dup2 |
| `ImageStore` path: `/` replaced with `_` | `cache_name()` uses forward slashes; `ImageStore` handles sanitisation internally |
| Backward compat: existing docker.io caches | `cache_name()` for docker.io returns `library/alpine` (no prefix) — don't change this |
