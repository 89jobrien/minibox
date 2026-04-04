# test-linux Dogfood Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cargo xtask test-linux` runs the full minibox Linux test suite on macOS by cross-compiling test binaries into an OCI image (`mbx-tester`), loading it via `minibox load`, and running it with `minibox run --privileged`.

**Architecture:** Three sequential layers: (1) protocol + handler + CLI for `LoadImage`, (2) xtask `build-test-image` that cross-compiles and assembles the OCI tarball, (3) xtask `test-linux` that orchestrates load + run. The outer `miniboxd` uses `MINIBOX_ADAPTER=colima`; the inner `miniboxd` inside the container uses `MINIBOX_ADAPTER=native`.

**Tech Stack:** Rust 2024, xshell, clap, serde_json, tar + flate2 + sha2, minibox_core::image::ImageStore, nerdctl (Colima VM).

---

## File Map

| File                                      | Change                                                                 |
| ----------------------------------------- | ---------------------------------------------------------------------- |
| `crates/minibox-core/src/protocol.rs`     | Add `LoadImage` request + `ImageLoaded` response                       |
| `crates/mbx/src/protocol.rs`              | Same (two protocol files must stay in sync)                            |
| `crates/minibox-core/src/domain.rs`       | Add `ImageLoader` trait + `DynImageLoader`                             |
| `crates/mbx/src/adapters/image_loader.rs` | New: `NativeImageLoader` (extracts tarball into ImageStore)            |
| `crates/mbx/src/adapters/mod.rs`          | Wire `pub mod image_loader`                                            |
| `crates/mbx/src/adapters/colima.rs`       | Add `impl ImageLoader for ColimaRegistry`                              |
| `crates/daemonbox/src/handler.rs`         | Add `handle_load_image`, `image_loader` field on `HandlerDependencies` |
| `crates/daemonbox/src/server.rs`          | Wire `LoadImage` in `dispatch()` + `is_terminal_response()`            |
| `crates/daemonbox/tests/handler_tests.rs` | Add `test_load_image_success`, `test_load_image_missing_file`          |
| `crates/macbox/src/lib.rs`                | Inject `ColimaRegistry` as `image_loader` in `HandlerDependencies`     |
| `crates/miniboxd/src/lib.rs`              | Inject `NativeImageLoader` as `image_loader` in `HandlerDependencies`  |
| `crates/minibox-cli/src/commands/load.rs` | New: `execute()` for `minibox load <path>`                             |
| `crates/minibox-cli/src/commands/mod.rs`  | `pub mod load;`                                                        |
| `crates/minibox-cli/src/main.rs`          | Add `Load` variant + dispatch                                          |
| `crates/xtask/src/test_image.rs`          | New: `build_test_image()` + OCI tarball assembly                       |
| `crates/xtask/src/gates.rs`               | Add `test_linux()`                                                     |
| `crates/xtask/src/main.rs`                | Wire `build-test-image` + `test-linux`                                 |

---

## Task 1: Add `LoadImage`/`ImageLoaded` to both protocol files

**Files:**

- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/mbx/src/protocol.rs`

- [ ] **Step 1: Add variants to `minibox-core/src/protocol.rs`**

In `crates/minibox-core/src/protocol.rs`, add after the `Pull` variant in `DaemonRequest`:

```rust
/// Load a local OCI image tarball into the daemon's image store.
LoadImage {
    /// Absolute path to the OCI tarball on the host filesystem.
    path: String,
    /// Image name to register (e.g. `"mbx-tester"`).
    name: String,
    /// Image tag to register (e.g. `"latest"`).
    tag: String,
},
```

Add to `DaemonResponse` after the `ContainerList` variant:

```rust
/// Confirmation that a local image tarball was loaded successfully.
ImageLoaded {
    /// The image reference that was registered, e.g. `"mbx-tester:latest"`.
    image: String,
},
```

- [ ] **Step 2: Apply the same additions to `crates/mbx/src/protocol.rs`**

Identical additions to `DaemonRequest` and `DaemonResponse` in `crates/mbx/src/protocol.rs`. The two files are independent definitions that must stay in sync (see CLAUDE.md protocol gotchas).

- [ ] **Step 3: Run cargo check**

```bash
cargo check -p minibox-core -p mbx
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-core/src/protocol.rs crates/mbx/src/protocol.rs
git commit -m "feat(protocol): add LoadImage request and ImageLoaded response variants"
```

---

## Task 2: Add `ImageLoader` domain trait

**Files:**

- Modify: `crates/minibox-core/src/domain.rs`

- [ ] **Step 1: Write a failing test**

At the bottom of `crates/minibox-core/src/domain.rs`, add to the existing `#[cfg(test)]` block (or create one):

```rust
#[cfg(test)]
mod image_loader_tests {
    use super::*;
    use std::path::Path;

    struct AlwaysOkLoader;

    #[async_trait::async_trait]
    impl ImageLoader for AlwaysOkLoader {
        async fn load_image(&self, _path: &Path, _name: &str, _tag: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn image_loader_trait_is_object_safe() {
        let loader: Box<dyn ImageLoader> = Box::new(AlwaysOkLoader);
        let result = loader
            .load_image(std::path::Path::new("/fake.tar"), "mbx-tester", "latest")
            .await;
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cargo test -p minibox-core image_loader_tests 2>&1 | head -20
```

Expected: compile error — `ImageLoader` not defined.

- [ ] **Step 3: Add `ImageLoader` trait**

In `crates/minibox-core/src/domain.rs`, after the `ImageRegistry` trait:

```rust
/// Port for loading a local OCI image tarball into the image store.
///
/// Implementations:
/// - `NativeImageLoader`: extracts tarball directly into `ImageStore`
/// - `ColimaRegistry`: delegates to `nerdctl load -i <path>` in the Lima VM
#[async_trait::async_trait]
pub trait ImageLoader: Send + Sync {
    /// Load the OCI tarball at `path` and register it as `name:tag`.
    async fn load_image(
        &self,
        path: &std::path::Path,
        name: &str,
        tag: &str,
    ) -> anyhow::Result<()>;
}

pub type DynImageLoader = Arc<dyn ImageLoader>;
```

- [ ] **Step 4: Run test to confirm it passes**

```bash
cargo test -p minibox-core image_loader_tests
```

Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-core/src/domain.rs
git commit -m "feat(domain): add ImageLoader trait port for local OCI tarball loading"
```

---

## Task 3: `NativeImageLoader` adapter

**Files:**

- Create: `crates/mbx/src/adapters/image_loader.rs`
- Modify: `crates/mbx/src/adapters/mod.rs`

- [ ] **Step 1: Create `image_loader.rs` with failing test**

```rust
//! Native ImageLoader adapter — extracts a local OCI tarball into ImageStore.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use minibox_core::domain::ImageLoader;
use minibox_core::image::ImageStore;
use std::path::Path;
use std::sync::Arc;

pub struct NativeImageLoader {
    store: Arc<ImageStore>,
}

impl NativeImageLoader {
    pub fn new(store: Arc<ImageStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ImageLoader for NativeImageLoader {
    async fn load_image(&self, path: &Path, name: &str, tag: &str) -> Result<()> {
        todo!("implement NativeImageLoader::load_image")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn load_image_rejects_nonexistent_path() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
        let loader = NativeImageLoader::new(store);
        let result = loader
            .load_image(Path::new("/nonexistent/fake.tar"), "test", "latest")
            .await;
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Wire in `mod.rs`**

Add to `crates/mbx/src/adapters/mod.rs`:

```rust
pub mod image_loader;
pub use image_loader::NativeImageLoader;
```

- [ ] **Step 3: Run test to confirm `todo!` panics**

```bash
cargo test -p mbx adapters::image_loader::tests::load_image_rejects_nonexistent_path 2>&1 | head -20
```

Expected: test panics with `not yet implemented`.

- [ ] **Step 4: Implement `load_image`**

Replace `todo!` in `load_image`:

```rust
async fn load_image(&self, path: &Path, name: &str, tag: &str) -> Result<()> {
    if !path.exists() {
        bail!("image tarball not found: {}", path.display());
    }

    // Unpack the outer OCI layout tarball into a temp dir
    let file = std::fs::File::open(path)
        .with_context(|| format!("open tarball {}", path.display()))?;
    let mut outer = tar::Archive::new(file);
    let tmp = tempfile::TempDir::new().context("create temp dir")?;
    outer.unpack(tmp.path()).context("unpack OCI tarball")?;

    // Parse manifest.json for layer digests
    let manifest_path = tmp.path().join("manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: minibox_core::image::manifest::OciManifest =
        serde_json::from_slice(&manifest_bytes).context("parse manifest.json")?;

    // Store manifest
    self.store
        .store_manifest(name, tag, &manifest)
        .with_context(|| format!("store manifest for {name}:{tag}"))?;

    // Extract each layer blob into the image store
    for layer_desc in &manifest.layers {
        let digest_hex = layer_desc.digest.trim_start_matches("sha256:");
        let blob_path = tmp.path().join("blobs").join("sha256").join(digest_hex);
        let blob_file = std::fs::File::open(&blob_path)
            .with_context(|| format!("open blob {}", blob_path.display()))?;
        self.store
            .store_layer(name, tag, &layer_desc.digest, blob_file)
            .with_context(|| format!("store layer {}", layer_desc.digest))?;
    }

    Ok(())
}
```

Add imports at the top of the file:

```rust
use minibox_core::image::manifest::OciManifest;
```

Ensure `tar` and `tempfile` are in `crates/mbx/Cargo.toml` dev/regular deps (check first).

- [ ] **Step 5: Run test**

```bash
cargo test -p mbx adapters::image_loader::tests
```

Expected: 1 passed (`load_image_rejects_nonexistent_path`).

- [ ] **Step 6: Commit**

```bash
git add crates/mbx/src/adapters/image_loader.rs crates/mbx/src/adapters/mod.rs
git commit -m "feat(adapters): NativeImageLoader — extract OCI tarball into ImageStore"
```

---

## Task 4: `ColimaRegistry` implements `ImageLoader`

**Files:**

- Modify: `crates/mbx/src/adapters/colima.rs`

- [ ] **Step 1: Write the failing test**

In the `#[cfg(test)]` block at the bottom of `crates/mbx/src/adapters/colima.rs`, add:

```rust
#[tokio::test]
async fn colima_load_image_calls_nerdctl_load() {
    use std::sync::Mutex;
    let called_args: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let called_clone = Arc::clone(&called_args);

    let loader = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        called_clone
            .lock()
            .unwrap()
            .extend(args.iter().map(|s| s.to_string()));
        Ok(String::new())
    }));

    let result = loader
        .load_image(std::path::Path::new("/tmp/mbx-tester.tar"), "mbx-tester", "latest")
        .await;
    assert!(result.is_ok(), "load_image failed: {result:?}");

    let args = called_args.lock().unwrap();
    assert!(
        args.iter().any(|a| a == "nerdctl"),
        "expected nerdctl call, got: {args:?}"
    );
    assert!(
        args.iter().any(|a| a == "load"),
        "expected 'load' arg, got: {args:?}"
    );
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test -p mbx adapters::colima::colima_load_image_calls_nerdctl_load 2>&1 | head -20
```

Expected: compile error — `ImageLoader` not implemented for `ColimaRegistry`.

- [ ] **Step 3: Implement `ImageLoader` for `ColimaRegistry`**

Add after the existing `impl ImageRegistry for ColimaRegistry` block:

```rust
#[async_trait::async_trait]
impl minibox_core::domain::ImageLoader for ColimaRegistry {
    /// Load a local OCI tarball into the Colima VM's containerd image store.
    ///
    /// The tarball path must be reachable from inside the Lima VM.
    /// Lima automatically shares `/tmp` and `$HOME`, so place the tarball
    /// under `~/.mbx/` (which is in `$HOME`) for guaranteed access.
    async fn load_image(
        &self,
        path: &std::path::Path,
        _name: &str,
        _tag: &str,
    ) -> anyhow::Result<()> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 path: {}", path.display()))?;
        self.lima_exec(&["nerdctl", "load", "-i", path_str])
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("nerdctl load failed: {e}"))
    }
}
```

- [ ] **Step 4: Run test**

```bash
cargo test -p mbx adapters::colima::colima_load_image_calls_nerdctl_load
```

Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/mbx/src/adapters/colima.rs
git commit -m "feat(adapters): impl ImageLoader for ColimaRegistry via nerdctl load"
```

---

## Task 5: Wire `ImageLoader` into `HandlerDependencies` + `handle_load_image`

**Files:**

- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`
- Modify: `crates/daemonbox/tests/handler_tests.rs`

- [ ] **Step 1: Add `image_loader` field to `HandlerDependencies`**

In `crates/daemonbox/src/handler.rs`, add to `HandlerDependencies`:

```rust
/// Loader for local OCI image tarballs.
pub image_loader: minibox_core::domain::DynImageLoader,
```

Add builder method on `HandlerDependencies` (after the struct definition):

```rust
impl HandlerDependencies {
    pub fn with_image_loader(mut self, loader: minibox_core::domain::DynImageLoader) -> Self {
        self.image_loader = loader;
        self
    }
}
```

- [ ] **Step 2: Add `NoopImageLoader` test double in `handler.rs`**

Near the bottom of `handler.rs` in the `#[cfg(test)]` section:

```rust
#[cfg(test)]
pub struct NoopImageLoader;

#[cfg(test)]
#[async_trait::async_trait]
impl minibox_core::domain::ImageLoader for NoopImageLoader {
    async fn load_image(
        &self,
        _path: &std::path::Path,
        _name: &str,
        _tag: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
```

Update `create_test_deps_with_dir` in `crates/daemonbox/tests/handler_tests.rs` to add:

```rust
image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
```

- [ ] **Step 3: Write the failing handler tests**

In `crates/daemonbox/tests/handler_tests.rs`, add:

```rust
#[tokio::test]
async fn test_load_image_success() {
    let tmp = tempfile::TempDir::new().unwrap();

    struct OkLoader;
    #[async_trait::async_trait]
    impl minibox_core::domain::ImageLoader for OkLoader {
        async fn load_image(
            &self,
            _p: &std::path::Path,
            _n: &str,
            _t: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    let deps = create_test_deps_with_dir(tmp.path())
        .with_image_loader(Arc::new(OkLoader) as minibox_core::domain::DynImageLoader);

    let state = create_test_state_with_dir(tmp.path());
    let response = handle_load_image_once(
        "/tmp/fake.tar".to_string(),
        "mbx-tester".to_string(),
        "latest".to_string(),
        state,
        Arc::new(deps),
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::ImageLoaded { .. }),
        "expected ImageLoaded, got: {response:?}"
    );
}

#[tokio::test]
async fn test_load_image_failure() {
    let tmp = tempfile::TempDir::new().unwrap();

    struct FailLoader;
    #[async_trait::async_trait]
    impl minibox_core::domain::ImageLoader for FailLoader {
        async fn load_image(
            &self,
            _p: &std::path::Path,
            _n: &str,
            _t: &str,
        ) -> anyhow::Result<()> {
            anyhow::bail!("file not found")
        }
    }

    let deps = create_test_deps_with_dir(tmp.path())
        .with_image_loader(Arc::new(FailLoader) as minibox_core::domain::DynImageLoader);

    let state = create_test_state_with_dir(tmp.path());
    let response = handle_load_image_once(
        "/nonexistent/fake.tar".to_string(),
        "mbx-tester".to_string(),
        "latest".to_string(),
        state,
        Arc::new(deps),
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "expected Error, got: {response:?}"
    );
}

/// Helper: call handle_load_image and return the single response.
async fn handle_load_image_once(
    path: String,
    name: String,
    tag: String,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    daemonbox::handler::handle_load_image(path, name, tag, deps).await
}
```

- [ ] **Step 4: Run to confirm tests fail**

```bash
cargo test -p daemonbox --test handler_tests test_load_image 2>&1 | head -20
```

Expected: compile error — `handle_load_image` not defined.

- [ ] **Step 5: Add `handle_load_image` to `handler.rs`**

In `crates/daemonbox/src/handler.rs`, add after `handle_pull`:

```rust
// ─── Load Image ─────────────────────────────────────────────────────────────

/// Load a local OCI image tarball into the image store.
#[instrument(skip(deps), fields(path = %path, name = %name, tag = %tag))]
pub async fn handle_load_image(
    path: String,
    name: String,
    tag: String,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let image_path = std::path::Path::new(&path);
    match deps.image_loader.load_image(image_path, &name, &tag).await {
        Ok(()) => {
            info!(
                path = %path,
                image = %format!("{name}:{tag}"),
                "load_image: loaded successfully"
            );
            DaemonResponse::ImageLoaded {
                image: format!("{name}:{tag}"),
            }
        }
        Err(e) => {
            error!("handle_load_image error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}
```

- [ ] **Step 6: Wire `LoadImage` into `dispatch()` and `is_terminal_response()`**

In `crates/daemonbox/src/server.rs`:

Add to `is_terminal_response()` match:

```rust
| DaemonResponse::ImageLoaded { .. }
```

Add to `dispatch()` match:

```rust
DaemonRequest::LoadImage { path, name, tag } => {
    let response = handler::handle_load_image(path, name, tag, deps).await;
    let _ = tx.send(response).await;
}
```

- [ ] **Step 7: Run tests**

```bash
cargo test -p daemonbox --test handler_tests test_load_image
```

Expected: 2 passed.

Also run:

```bash
cargo test -p daemonbox server::tests::test_is_terminal_response_all_variants
```

Expected: 1 passed.

- [ ] **Step 8: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/daemonbox/src/server.rs crates/daemonbox/tests/handler_tests.rs
git commit -m "feat(handler): handle_load_image + wire LoadImage into dispatch and is_terminal_response"
```

---

## Task 6: Wire `image_loader` in composition roots

**Files:**

- Modify: `crates/macbox/src/lib.rs`
- Modify: `crates/miniboxd/src/lib.rs` (check exact file with `grep -rn "HandlerDependencies {" crates/miniboxd/`)

- [ ] **Step 1: Find `HandlerDependencies` construction sites**

```bash
grep -rn "HandlerDependencies {" crates/macbox/ crates/miniboxd/
```

Read the matching file(s) to understand the exact struct literal pattern.

- [ ] **Step 2: Inject `ColimaRegistry` as `image_loader` in macOS path (`crates/macbox/src/lib.rs`)**

`ColimaRegistry` is already instantiated as the registry adapter. Create a second instance (or share via `Arc`) for the loader:

```rust
use mbx::adapters::ColimaRegistry;

let image_loader = Arc::new(ColimaRegistry::new()) as minibox_core::domain::DynImageLoader;
```

Add `image_loader` to the `HandlerDependencies { ... }` struct literal.

- [ ] **Step 3: Inject `NativeImageLoader` in Linux path**

In the Linux composition root, the `ImageStore` is already created as `Arc<ImageStore>`. Add:

```rust
use mbx::adapters::NativeImageLoader;

let image_loader = Arc::new(NativeImageLoader::new(Arc::clone(&image_store)))
    as minibox_core::domain::DynImageLoader;
```

Add `image_loader` to the `HandlerDependencies { ... }` struct literal.

- [ ] **Step 4: cargo check**

```bash
cargo check -p macbox -p miniboxd
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/macbox/src/lib.rs crates/miniboxd/src/lib.rs
git commit -m "feat(wiring): inject ImageLoader into HandlerDependencies for macOS and Linux"
```

---

## Task 7: `minibox load` CLI subcommand

**Files:**

- Create: `crates/minibox-cli/src/commands/load.rs`
- Modify: `crates/minibox-cli/src/commands/mod.rs`
- Modify: `crates/minibox-cli/src/main.rs`

- [ ] **Step 1: Create `load.rs` with stub and test**

```rust
//! `minibox load` — load a local OCI image tarball into the daemon.

use anyhow::Context;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

pub async fn execute(
    path: String,
    name: String,
    tag: String,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    async fn serve_once(socket_path: &std::path::Path, response: DaemonResponse) {
        let listener = UnixListener::bind(socket_path).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let mut resp = serde_json::to_string(&response).unwrap();
        resp.push('\n');
        write_half.write_all(resp.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    }

    #[tokio::test]
    async fn execute_succeeds_on_image_loaded_response() {
        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let sp = socket_path.clone();
        tokio::spawn(async move {
            serve_once(
                &sp,
                DaemonResponse::ImageLoaded {
                    image: "mbx-tester:latest".to_string(),
                },
            )
            .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute(
            "/tmp/mbx-tester.tar".to_string(),
            "mbx-tester".to_string(),
            "latest".to_string(),
            &socket_path,
        )
        .await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }
}
```

- [ ] **Step 2: Wire module**

In `crates/minibox-cli/src/commands/mod.rs`:

```rust
pub mod load;
```

- [ ] **Step 3: Run test to confirm `todo!` panics**

```bash
cargo test -p minibox-cli commands::load::tests::execute_succeeds_on_image_loaded_response 2>&1 | head -20
```

Expected: panics with `not yet implemented`.

- [ ] **Step 4: Implement `execute`**

Replace `todo!()`:

```rust
pub async fn execute(
    path: String,
    name: String,
    tag: String,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    eprintln!("Loading {name}:{tag} from {path}...");

    let request = DaemonRequest::LoadImage {
        path,
        name: name.clone(),
        tag: tag.clone(),
    };
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client.call(request).await.context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ImageLoaded { image } => {
                println!("Loaded {image}");
                Ok(())
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}
```

- [ ] **Step 5: Add `Load` variant to `Commands` enum in `main.rs`**

```rust
/// Load a local OCI image tarball into the daemon's image store.
Load {
    /// Path to the OCI tarball (e.g. ~/.mbx/test-image/mbx-tester.tar)
    path: String,
    /// Image name to register (default: derived from filename stem)
    #[arg(short, long)]
    name: Option<String>,
    /// Image tag to register
    #[arg(short, long, default_value = "latest")]
    tag: String,
},
```

Add to `match cli.command`:

```rust
Commands::Load { path, name, tag } => {
    let name = name.unwrap_or_else(|| {
        std::path::Path::new(&path)
            .file_stem()
            .unwrap_or(std::ffi::OsStr::new("image"))
            .to_string_lossy()
            .to_string()
    });
    commands::load::execute(path, name, tag, socket_path).await
}
```

- [ ] **Step 6: Run test**

```bash
cargo test -p minibox-cli commands::load::tests::execute_succeeds_on_image_loaded_response
```

Expected: 1 passed.

- [ ] **Step 7: cargo check**

```bash
cargo check -p minibox-cli
```

Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add crates/minibox-cli/src/commands/load.rs crates/minibox-cli/src/commands/mod.rs crates/minibox-cli/src/main.rs
git commit -m "feat(cli): add 'minibox load' subcommand for local OCI tarball loading"
```

---

## Task 8: Full quality gate for Tasks 1–7

- [ ] **Step 1: Pre-commit gate**

```bash
cargo xtask pre-commit
```

Expected: `pre-commit checks passed`.

- [ ] **Step 2: Unit tests**

```bash
cargo xtask test-unit
```

Expected: all pass; count increases by ~5 new tests.

- [ ] **Step 3: Commit any auto-fmt fixes**

```bash
git diff --quiet || git add -A && git commit -m "chore: cargo fmt after LoadImage wiring"
```

---

## Task 9: `cargo xtask build-test-image`

**Files:**

- Create: `crates/xtask/src/test_image.rs`
- Modify: `crates/xtask/src/main.rs`
- Modify: `crates/xtask/Cargo.toml` (if `walkdir` not already present)

- [ ] **Step 1: Check existing deps in `crates/xtask/Cargo.toml`**

```bash
grep -E "walkdir|sha2|flate2|^tar" crates/xtask/Cargo.toml
```

Add any missing ones:

```toml
walkdir = "2"
sha2 = "0.10"
flate2 = "1"
tar = "0.4"
```

(`vm_image.rs` may already pull some of these in.)

- [ ] **Step 2: Create `test_image.rs` with stub and unit test**

Create `crates/xtask/src/test_image.rs`:

```rust
//! Build the `mbx-tester` OCI image tarball for Linux test dogfooding.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

pub fn default_test_image_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".mbx").join("test-image")
}

pub fn build_test_image(sh: &Shell, out_dir: &Path, force: bool) -> Result<()> {
    todo!("implement build_test_image")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_test_image_dir_under_mbx() {
        let dir = default_test_image_dir();
        let s = dir.to_string_lossy();
        assert!(s.contains(".mbx"), "expected .mbx in path: {s}");
        assert!(s.contains("test-image"), "expected test-image in path: {s}");
    }
}
```

- [ ] **Step 3: Wire in `main.rs`**

Add to `crates/xtask/src/main.rs`:

```rust
mod test_image;
```

Add to `match` block:

```rust
Some("build-test-image") => {
    let force = env::args().any(|a| a == "--force");
    let out_dir = test_image::default_test_image_dir();
    test_image::build_test_image(&sh, &out_dir, force)
}
Some("test-linux") => gates::test_linux(&sh),
```

Add to help text:

```rust
eprintln!("  build-test-image build mbx-tester OCI tarball (cross-compile aarch64-musl)");
eprintln!("  test-linux       run Linux tests via minibox on macOS (Colima)");
```

- [ ] **Step 4: Run unit test**

```bash
cargo test -p xtask test_image::tests
```

Expected: 1 passed.

- [ ] **Step 5: Implement `build_test_image`**

Replace `todo!` with the full implementation. This is the longest step — take it in sub-parts:

**5a.** Add helper types and cache check at top of function:

```rust
pub fn build_test_image(sh: &Shell, out_dir: &Path, force: bool) -> Result<()> {
    let target = "aarch64-unknown-linux-musl";
    let tarball = out_dir.join("mbx-tester.tar");

    if !force && tarball.exists() && tarball_is_fresh(&tarball)? {
        eprintln!("[build-test-image] cached: {}", tarball.display());
        return Ok(());
    }

    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("create {}", out_dir.display()))?;

    // 1. Cross-compile
    eprintln!("[build-test-image] cross-compiling for {target}...");
    for test_name in &["cgroup_tests", "e2e_tests", "integration_tests", "sandbox_tests"] {
        cmd!(sh, "cargo test -p miniboxd --test {test_name} --no-run --release --target {target}")
            .run()
            .with_context(|| format!("build {test_name}"))?;
    }
    cmd!(sh, "cargo build --release --target {target} -p miniboxd -p minibox-cli")
        .run()
        .context("build miniboxd + minibox-cli")?;

    // 2. Collect binaries
    let target_dir = sh.current_dir().join("target").join(target).join("release");
    let deps_dir = target_dir.join("deps");
    let bins = collect_binaries(&target_dir, &deps_dir)?;

    // 3. Assemble OCI tarball
    eprintln!("[build-test-image] assembling mbx-tester.tar...");
    assemble_oci_tarball(&bins, &tarball)?;

    eprintln!("[build-test-image] done: {}", tarball.display());
    Ok(())
}
```

**5b.** Add cache freshness helper:

```rust
fn tarball_is_fresh(tarball: &Path) -> Result<bool> {
    let tar_mtime = std::fs::metadata(tarball)
        .and_then(|m| m.modified())
        .context("stat tarball")?;
    let newest = newest_rs_mtime("crates")?;
    Ok(tar_mtime > newest)
}

fn newest_rs_mtime(root: &str) -> Result<std::time::SystemTime> {
    use std::time::SystemTime;
    let mut newest = SystemTime::UNIX_EPOCH;
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "rs").unwrap_or(false))
    {
        if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
            if mtime > newest {
                newest = mtime;
            }
        }
    }
    Ok(newest)
}
```

**5c.** Add binary collection:

```rust
struct ImageBinaries {
    miniboxd: PathBuf,
    minibox: PathBuf,
    cgroup_tests: PathBuf,
    e2e_tests: PathBuf,
    integration_tests: PathBuf,
    sandbox_tests: PathBuf,
}

fn collect_binaries(target_dir: &Path, deps_dir: &Path) -> Result<ImageBinaries> {
    let find = |prefix: &str| -> Result<PathBuf> {
        crate::gates::find_test_binary(deps_dir.to_str().unwrap_or(""), prefix)
            .with_context(|| format!("{prefix} binary not found in {}", deps_dir.display()))
    };
    Ok(ImageBinaries {
        miniboxd:          target_dir.join("miniboxd"),
        minibox:           target_dir.join("minibox"),
        cgroup_tests:      find("cgroup_tests")?,
        e2e_tests:         find("e2e_tests")?,
        integration_tests: find("integration_tests")?,
        sandbox_tests:     find("sandbox_tests")?,
    })
}
```

**5d.** Add OCI tarball assembly:

```rust
const RUN_TESTS_SH: &[u8] = b"#!/bin/sh\n\
set -e\n\
export MINIBOX_ADAPTER=native\n\
echo '=== cgroup_tests ==='\n\
/usr/local/bin/cgroup_tests --test-threads=1 --nocapture\n\
echo '=== integration_tests ==='\n\
/usr/local/bin/integration_tests --test-threads=1 --ignored --nocapture\n\
echo '=== e2e_tests ==='\n\
MINIBOX_TEST_BIN_DIR=/usr/local/bin /usr/local/bin/e2e_tests --test-threads=1 --nocapture\n\
echo '=== sandbox_tests ==='\n\
MINIBOX_TEST_BIN_DIR=/usr/local/bin /usr/local/bin/sandbox_tests --test-threads=1 --ignored --nocapture\n\
echo '=== all Linux tests passed ==='\n";

fn assemble_oci_tarball(bins: &ImageBinaries, tarball: &Path) -> Result<()> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use sha2::{Digest, Sha256};
    use tar::Builder;

    // Build layer (gzipped tar of binaries + entrypoint)
    let mut layer_gz = Vec::new();
    {
        let enc = GzEncoder::new(&mut layer_gz, Compression::default());
        let mut tb = Builder::new(enc);

        let mut add_bin = |path: &Path, dest: &str| -> Result<()> {
            let mut f = std::fs::File::open(path)
                .with_context(|| format!("open {}", path.display()))?;
            let size = f.metadata()?.len();
            let mut hdr = tar::Header::new_gnu();
            hdr.set_size(size);
            hdr.set_mode(0o755);
            hdr.set_cksum();
            tb.append_data(&mut hdr, dest, &mut f)
                .with_context(|| format!("append {dest}"))
        };

        add_bin(&bins.miniboxd,          "usr/local/bin/miniboxd")?;
        add_bin(&bins.minibox,           "usr/local/bin/minibox")?;
        add_bin(&bins.cgroup_tests,      "usr/local/bin/cgroup_tests")?;
        add_bin(&bins.e2e_tests,         "usr/local/bin/e2e_tests")?;
        add_bin(&bins.integration_tests, "usr/local/bin/integration_tests")?;
        add_bin(&bins.sandbox_tests,     "usr/local/bin/sandbox_tests")?;

        let mut hdr = tar::Header::new_gnu();
        hdr.set_size(RUN_TESTS_SH.len() as u64);
        hdr.set_mode(0o755);
        hdr.set_cksum();
        tb.append_data(&mut hdr, "run-tests.sh", RUN_TESTS_SH)
            .context("append run-tests.sh")?;

        tb.into_inner()?.finish()?;
    }

    let layer_digest = format!("sha256:{:x}", Sha256::digest(&layer_gz));
    let layer_size = layer_gz.len() as u64;

    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "size": 0
        },
        "layers": [{"mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                    "digest": layer_digest, "size": layer_size}]
    });
    let manifest_bytes = serde_json::to_vec(&manifest).context("serialize manifest")?;
    let manifest_digest = format!("sha256:{:x}", Sha256::digest(&manifest_bytes));

    let index = serde_json::json!({
        "schemaVersion": 2,
        "manifests": [{"mediaType": "application/vnd.oci.image.manifest.v1+json",
                       "digest": manifest_digest,
                       "size": manifest_bytes.len(),
                       "annotations": {"org.opencontainers.image.ref.name": "mbx-tester:latest"}}]
    });

    // Write outer tarball
    let out = std::fs::File::create(tarball)
        .with_context(|| format!("create {}", tarball.display()))?;
    let mut outer = Builder::new(out);

    let mut write_bytes = |path: &str, data: &[u8]| -> Result<()> {
        let mut hdr = tar::Header::new_gnu();
        hdr.set_size(data.len() as u64);
        hdr.set_mode(0o644);
        hdr.set_cksum();
        outer.append_data(&mut hdr, path, data)
            .with_context(|| format!("append {path}"))
    };

    let blob_path = format!("blobs/sha256/{}", layer_digest.trim_start_matches("sha256:"));
    write_bytes(&blob_path, &layer_gz)?;
    write_bytes("manifest.json", &manifest_bytes)?;
    write_bytes("index.json", &serde_json::to_vec(&index).unwrap())?;

    outer.finish().context("finish outer tar")
}
```

Add imports at top of `test_image.rs`:

```rust
use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
```

- [ ] **Step 6: cargo check**

```bash
cargo check -p xtask
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add crates/xtask/src/test_image.rs crates/xtask/src/main.rs crates/xtask/Cargo.toml
git commit -m "feat(xtask): build-test-image — cross-compile + assemble mbx-tester OCI tarball"
```

---

## Task 10: `cargo xtask test-linux`

**Files:**

- Modify: `crates/xtask/src/gates.rs`

- [ ] **Step 1: Add `test_linux` function**

In `crates/xtask/src/gates.rs`, add:

```rust
/// Run the full Linux test suite on macOS via minibox (Colima adapter).
///
/// Requires:
/// - miniboxd running with MINIBOX_ADAPTER=colima
/// - Colima VM running (`colima status`)
/// - aarch64-linux-musl-gcc cross-compiler on PATH
pub fn test_linux(sh: &Shell) -> Result<()> {
    let out_dir = crate::test_image::default_test_image_dir();
    let tarball = out_dir.join("mbx-tester.tar");
    let force = std::env::args().any(|a| a == "--force");

    // Step 1: build image (cached unless --force)
    crate::test_image::build_test_image(sh, &out_dir, force)
        .context("build-test-image failed")?;

    // Step 2: load into minibox daemon
    let tarball_str = tarball.to_str().context("non-UTF-8 tarball path")?;
    cmd!(sh, "minibox load {tarball_str} --name mbx-tester --tag latest")
        .run()
        .context("minibox load failed -- is miniboxd running with MINIBOX_ADAPTER=colima?")?;

    // Step 3: run tests in privileged container, stream output
    cmd!(sh, "minibox run --privileged mbx-tester -- /run-tests.sh")
        .run()
        .context("minibox run failed")?;

    eprintln!("[test-linux] all Linux tests passed");
    Ok(())
}
```

- [ ] **Step 2: cargo check**

```bash
cargo check -p xtask
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/xtask/src/gates.rs
git commit -m "feat(xtask): test-linux -- run Linux tests via minibox run --privileged"
```

---

## Task 11: End-to-end smoke test

- [ ] **Step 1: Full quality gate**

```bash
cargo xtask pre-commit && cargo xtask test-unit
```

Expected: all pass.

- [ ] **Step 2: Start miniboxd (macOS, Colima adapter)**

```bash
MINIBOX_ADAPTER=colima cargo build --release -p miniboxd && \
  sudo MINIBOX_ADAPTER=colima ./target/release/miniboxd &
```

Wait 2 seconds for the socket to appear.

- [ ] **Step 3: Build the test image**

```bash
cargo xtask build-test-image
```

Expected: cross-compiles for `aarch64-unknown-linux-musl`, outputs `~/.mbx/test-image/mbx-tester.tar`.

- [ ] **Step 4: Run test-linux**

```bash
cargo xtask test-linux
```

Expected output (streamed from container):

```
=== cgroup_tests ===
test cgroup_create ... ok
...
=== e2e_tests ===
test test_e2e_pull_alpine ... ok
...
=== all Linux tests passed ===
[test-linux] all Linux tests passed
```

Exit code: 0.

- [ ] **Step 5: Verify cache works**

Run again without `--force`:

```bash
cargo xtask test-linux
```

Expected: `[build-test-image] cached: ...` — skips cross-compile, goes straight to load + run.

- [ ] **Step 6: Update HANDOFF.md**

- Add `cargo xtask test-linux` to the quality gates section
- Add `~/.mbx/test-image/mbx-tester.tar` to the runtime paths table
- Mark the three test-linux dogfood todos as done in Next up

```bash
git add HANDOFF.md
git commit -m "docs(handoff): update for test-linux dogfood feature completion"
```
