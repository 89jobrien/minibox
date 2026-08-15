# Plan: Structured Errors with Miette

## Goal

Replace inline `anyhow::bail!` / `anyhow!` error strings with typed error variants
across all crates, add `miette::Diagnostic` to every error enum, and propagate
`miette::Result` from the CLI surface down through the daemon boundary — so every
user-visible error has a machine-readable code, a help string, and rich terminal
rendering.

## Current State

- `minibox-core/src/error.rs` and `minibox/src/error.rs`: all enums already have
  `#[derive(Diagnostic)]` with codes and help text. **These are the model.**
- 177 `bail!` / 91 `anyhow!` calls across 9 crates use raw strings instead of
  typed variants. These are invisible to tooling, have no codes, and cannot carry
  structured context.
- `MacboxError`, `WinboxError`, `DomainError` (both crates), `ImageRefError`:
  `thiserror::Error` only — no `Diagnostic`.
- `mbx` CLI surfaces errors through `miette::Result<()>` in `main()` but only
  `CliError` itself has diagnostic metadata; inner errors are opaque anyhow chains.

## Architecture

- **Crates affected**: `minibox-core`, `minibox`, `mbx`, `macbox`, `winbox`,
  `smolbox`, `miniboxd`
- **No new crate dependencies** — `miette` (with `fancy` feature) and `thiserror`
  are already workspace deps in every affected crate
- **Data flow**: typed error variant → `?` propagates → miette renders at CLI

---

## Tasks

### Task 1: Add `Diagnostic` to `DomainError`, `MacboxError`, `WinboxError`

**Crate**: `minibox-core`, `minibox`, `macbox`, `winbox`
**File(s)**:
- `crates/minibox-core/src/domain.rs` (line ~1220)
- `crates/minibox/src/domain.rs` (line ~599)
- `crates/macbox/src/lib.rs` (line ~44)
- `crates/winbox/src/lib.rs` (line ~33)
**Run**: `cargo check --workspace`

For each enum, add `miette::Diagnostic` to the derive and a `#[diagnostic(...)]`
attribute on each variant.

`crates/minibox-core/src/domain.rs`:
```rust
// Before:
#[derive(Debug, thiserror::Error)]
pub enum DomainError { ... }

// After:
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum DomainError {
    #[error("container not found: {id}")]
    #[diagnostic(
        code(minibox::domain::container_not_found),
        help("run `mbx ps` to list running containers")
    )]
    ContainerNotFound { id: String },

    #[error("invalid container state: {message}")]
    #[diagnostic(code(minibox::domain::invalid_state))]
    InvalidState { message: String },

    // ... add #[diagnostic(code(...))] to each existing variant
}
```

`crates/macbox/src/lib.rs`:
```rust
// Before:
#[derive(thiserror::Error, Debug)]
pub enum MacboxError { ... }

// After:
#[derive(thiserror::Error, Debug, miette::Diagnostic)]
pub enum MacboxError {
    // add #[diagnostic(code(minibox::mac::...))] to each variant
}
```

`crates/winbox/src/lib.rs`:
```rust
#[derive(thiserror::Error, Debug, miette::Diagnostic)]
pub enum WinboxError {
    #[error("no backend — enable Windows Containers or install WSL2")]
    #[diagnostic(
        code(minibox::win::no_backend),
        help("enable Windows Containers via Windows Features, or install WSL2")
    )]
    NoBackendAvailable,
}
```

Verify:
```
cargo check --workspace  → zero errors
cargo clippy --workspace -- -D warnings  → zero warnings
```

Commit: `git commit -m "feat(errors): add Diagnostic derive to DomainError, MacboxError, WinboxError"`

---

### Task 2: Add typed variants for domain validation `bail!` calls in `minibox-core`

**Crate**: `minibox-core`
**File(s)**:
- `crates/minibox-core/src/domain.rs`
- `crates/minibox-core/src/path.rs`
**Run**: `cargo nextest run -p minibox-core`

The `domain.rs` file has ~16 `anyhow::bail!` calls for volume/mount parsing
validation. Consolidate them into a typed `ParseError` enum.

1. Add to `crates/minibox-core/src/error.rs`:

```rust
/// Parse errors for CLI-supplied values (volumes, mounts, container IDs).
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum ParseError {
    #[error("invalid volume format {input:?}: expected src:dst or src:dst:ro")]
    #[diagnostic(
        code(minibox::parse::invalid_volume),
        help("use the format: /host/path:/container/path or /host/path:/container/path:ro")
    )]
    InvalidVolume { input: String },

    #[error("unsupported mount type {mount_type:?}: only 'bind' is supported")]
    #[diagnostic(
        code(minibox::parse::unsupported_mount_type),
        help("remove the 'type=' key or set it to 'bind'")
    )]
    UnsupportedMountType { mount_type: String },

    #[error("--mount missing required key '{key}'")]
    #[diagnostic(
        code(minibox::parse::missing_mount_key),
        help("provide the key with --mount {key}=/path")
    )]
    MissingMountKey { key: &'static str },

    #[error("invalid destination path {path:?}: {reason}")]
    #[diagnostic(code(minibox::parse::invalid_path))]
    InvalidPath { path: String, reason: String },

    #[error("invalid container ID {id:?}: {reason}")]
    #[diagnostic(
        code(minibox::parse::invalid_container_id),
        help("container IDs must be non-empty, alphanumeric, and at most 64 characters")
    )]
    InvalidContainerId { id: String, reason: String },

    #[error("unsupported network mode {mode:?}: expected none, bridge, host, or tailnet")]
    #[diagnostic(
        code(minibox::parse::unsupported_network_mode),
        help("valid modes: none, bridge, host, tailnet")
    )]
    UnsupportedNetworkMode { mode: String },
}
```

2. Replace all `anyhow::bail!` in `domain.rs` volume/mount/container-ID parsing
   with `return Err(ParseError::InvalidVolume { input: s.to_string() }.into())` or
   use `?` after constructing the variant.

   Key sites in `domain.rs` (lines ~473–555 volume parsing, ~1399–1408 container
   ID validation, ~1752–1855 unsupported adapter ops):

```rust
// Before (line ~473):
anyhow::bail!("invalid volume format {s:?}: expected src:dst or src:dst:ro");

// After:
return Err(ParseError::InvalidVolume { input: s.to_string() })?;
// equivalent but idiomatic: use the ? on an Err to convert via From
```

   For unsupported adapter ops (checkpoint, PTY), create `UnsupportedOperation`
   variants in `DomainError` rather than generic strings:

```rust
// Before:
anyhow::bail!("pty: PTY allocation is not supported in this environment")

// After (in DomainError):
#[error("PTY allocation is not supported by this adapter")]
#[diagnostic(
    code(minibox::domain::pty_unsupported),
    help("use a Linux-native adapter or check adapter capabilities with `mbx doctor`")
)]
PtyUnsupported,
```

3. In `path.rs`, replace `bail!` with `InternalPathError` variants:

```rust
// Add to error.rs:
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum InternalPathError {
    #[error("path contains parent directory component (..)")]
    #[diagnostic(
        code(minibox::path::parent_traversal),
        help("paths must not escape the container root")
    )]
    ParentTraversal,

    #[error("path is absolute: {path}")]
    #[diagnostic(code(minibox::path::absolute_path))]
    AbsolutePath { path: String },
}
```

Verify:
```
cargo nextest run -p minibox-core  → all green
cargo clippy -p minibox-core -- -D warnings  → zero warnings
```

Commit: `git commit -m "feat(minibox-core): typed ParseError/InternalPathError replace inline bail!"`

---

### Task 3: Replace `bail!` in `minibox` crate (adapters + containers)

**Crate**: `minibox`
**File(s)**:
- `crates/minibox/src/adapters/ghcr.rs`
- `crates/minibox/src/adapters/colima.rs`
- `crates/minibox/src/container/filesystem.rs`
- `crates/minibox/src/nesting.rs`
**Run**: `cargo nextest run -p minibox`

The 81 `bail!` calls in `minibox` fall into three categories:

**a) Registry/HTTP errors** — in `ghcr.rs` these should use the existing
`RegistryError` enum variants (already in `minibox-core/src/error.rs`).
Currently ghcr.rs duplicates `RegistryError` logic:

```rust
// Before (ghcr.rs line ~205):
anyhow::bail!("ghcr: no WWW-Authenticate realm for {repo}");

// After — use existing RegistryError or add a new variant:
#[error("no WWW-Authenticate realm for repository {repo}")]
#[diagnostic(
    code(minibox::registry::no_realm),
    help("check that the repository exists and is accessible")
)]
NoRealm { repo: String },
// Then:
return Err(RegistryError::NoRealm { repo: repo.to_string() })?;
```

Size limit checks map to existing `RegistryError::Other` — replace with named
variants `ManifestTooLarge` and `LayerTooLarge`:

```rust
#[error("manifest too large: {size} bytes (max {max})")]
#[diagnostic(code(minibox::registry::manifest_too_large))]
ManifestTooLarge { size: u64, max: u64 },

#[error("layer too large: {size} bytes (max {max})")]
#[diagnostic(code(minibox::registry::layer_too_large))]
LayerTooLarge { size: u64, max: u64 },
```

**b) Filesystem errors** — `filesystem.rs` repeated `.map_err(|source|
FilesystemError::Mount { ... })`. Extract a helper closure:

```rust
fn mount_err(fs: impl Into<String>, target: impl Into<String>) -> impl FnOnce(nix::Error) -> FilesystemError {
    move |source| FilesystemError::Mount {
        fs: fs.into(),
        target: PathBuf::from(target.into()),
        source,
    }
}
// Usage:
mount(...).map_err(mount_err("tmpfs", &dev_dir))?;
```

**c) Nesting/DinD** — replace `bail!` in `nesting.rs` with `NestingError`:

```rust
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum NestingError {
    #[error("nested container environment variable {var} has invalid value {value:?}")]
    #[diagnostic(code(minibox::nesting::invalid_env_var))]
    InvalidEnvVar { var: String, value: String },
}
```

Verify:
```
cargo nextest run -p minibox  → all green
cargo clippy -p minibox -- -D warnings  → zero warnings
```

Commit: `git commit -m "feat(minibox): typed errors replace bail! in adapters and containers"`

---

### Task 4: Replace `bail!` in `mbx` CLI commands

**Crate**: `mbx`
**File(s)**:
- `crates/mbx/src/commands/run.rs`
- `crates/mbx/src/commands/upgrade.rs`
- `crates/mbx/src/commands/events.rs`
- `crates/mbx/src/commands/manifest.rs`
- `crates/mbx/src/commands/update.rs`
**Run**: `cargo nextest run -p mbx`

The existing `RequestError` in `crates/mbx/src/commands/mod.rs` already has
`Diagnostic`. Extend it with variants for each current `bail!` site:

```rust
// crates/mbx/src/commands/mod.rs — extend RequestError:
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum RequestError {
    // existing variants ...

    #[error("unsupported platform: {os}/{arch}")]
    #[diagnostic(
        code(mbx::upgrade::unsupported_platform),
        help("mbx releases are available for linux/amd64 and linux/arm64")
    )]
    UnsupportedPlatform { os: String, arch: String },

    #[error("mbx binary not found in release tarball")]
    #[diagnostic(
        code(mbx::upgrade::binary_not_found),
        help("the downloaded release may be corrupt; try again")
    )]
    BinaryNotInTarball,

    #[error("unknown network mode {mode:?}")]
    #[diagnostic(
        code(mbx::run::unknown_network_mode),
        help("valid modes: none, bridge, host, tailnet")
    )]
    UnknownNetworkMode { mode: String },

    #[error("daemon returned an error: {message}")]
    #[diagnostic(code(mbx::daemon::error))]
    DaemonError { message: String },

    #[error("unexpected response from daemon: {response:?}")]
    #[diagnostic(
        code(mbx::daemon::unexpected_response),
        help("this may indicate a version mismatch between mbx and miniboxd")
    )]
    UnexpectedResponse { response: String },

    #[error("no response from daemon")]
    #[diagnostic(
        code(mbx::daemon::no_response),
        help("check that miniboxd is running: systemctl status miniboxd")
    )]
    NoResponse,
}
```

Replace `bail!` at each call site:

```rust
// Before (run.rs line ~94):
bail!("unknown network mode: {other} ...");
// After:
return Err(RequestError::UnknownNetworkMode { mode: other.to_string() })?;

// Before (events.rs, manifest.rs, update.rs):
anyhow::bail!("{message}");
// After:
return Err(RequestError::DaemonError { message })?;

// Before:
anyhow::bail!("unexpected response: {other:?}");
// After:
return Err(RequestError::UnexpectedResponse { response: format!("{other:?}") })?;

// Before:
anyhow::bail!("no response from daemon");
// After:
return Err(RequestError::NoResponse)?;
```

Verify:
```
cargo nextest run -p mbx  → all green
cargo clippy -p mbx -- -D warnings  → zero warnings
```

Commit: `git commit -m "feat(mbx): typed RequestError variants replace inline bail! in commands"`

---

### Task 5: Replace `Err(...).into()` in `registry.rs` with `?`-propagation

**Crate**: `minibox-core`
**File(s)**:
- `crates/minibox-core/src/image/registry.rs`
**Run**: `cargo nextest run -p minibox-core`

The 10 `.into()` patterns (BP-010 findings) in `registry.rs` are
`return Err(RegistryError::Variant { ... }.into())`. With the `ManifestTooLarge`
and `LayerTooLarge` variants added in Task 3, replace `.into()` patterns using `?`:

```rust
// Before:
return Err(RegistryError::AuthFailed {
    image: image_name.to_owned(),
    message: format!("HTTP {status}: {msg}"),
}.into());

// After (RegistryError implements std::error::Error, anyhow converts via ?):
Err(RegistryError::AuthFailed {
    image: image_name.to_owned(),
    message: format!("HTTP {status}: {msg}"),
})?;
```

Apply to all 10 sites. For `RegistryError::Other(format!(...))` sites that are
now covered by named variants (`ManifestTooLarge`, `LayerTooLarge`), use the
named variant.

Verify:
```
cargo nextest run -p minibox-core  → all green
```

Commit: `git commit -m "refactor(registry): replace Err(.into()) with typed variants and ?"`

---

### Task 6: Surface `miette::Result` through daemon handler boundary

**Crate**: `miniboxd`, `minibox`
**File(s)**:
- `crates/minibox/src/daemon/handler/mod.rs`
- `crates/miniboxd/src/main.rs`
**Run**: `cargo nextest run -p minibox`

Today `miniboxd` uses `anyhow::Result` internally. Errors serialize to a plain
`DaemonResponse::Error { message: String }`. Upgrade the handler boundary to
preserve error codes in the serialized response:

1. Add `code` field to `DaemonResponse::Error` in `minibox-core/src/protocol.rs`:

```rust
// Before:
Error { message: String },

// After:
Error {
    message: String,
    /// Optional miette diagnostic code, e.g. "minibox::image::not_found".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<String>,
},
```

2. In `minibox/src/daemon/handler/mod.rs`, when converting an error to
   `DaemonResponse::Error`, extract the miette code if available:

```rust
use miette::Diagnostic;

fn error_response(err: &anyhow::Error) -> DaemonResponse {
    let code = err
        .chain()
        .find_map(|e| {
            let diag = (e as &dyn std::error::Error)
                .downcast_ref::<dyn Diagnostic>()?;
            diag.code().map(|c| c.to_string())
        });
    DaemonResponse::Error {
        message: format!("{err:#}"),
        code,
    }
}
```

3. In `mbx` CLI, when receiving `DaemonResponse::Error`, construct a
   `miette::Report` with the code for rich display:

```rust
// crates/mbx/src/commands/mod.rs:
fn daemon_error(message: String, code: Option<String>) -> RequestError {
    // preserve the code in the diagnostic if present
    RequestError::DaemonError { message, code }
}
```

   Update `RequestError::DaemonError` to carry `code: Option<String>` and
   implement `Diagnostic` manually so the code field renders:

```rust
// In RequestError (or standalone):
impl miette::Diagnostic for RequestError {
    fn code<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        match self {
            Self::DaemonError { code: Some(c), .. } =>
                Some(Box::new(c.as_str())),
            _ => None,
        }
    }
}
```

Verify:
```
cargo nextest run -p minibox -p mbx  → all green
cargo clippy --workspace -- -D warnings  → zero warnings
```

Commit: `git commit -m "feat(protocol): propagate miette error codes through daemon boundary"`

---

### Task 7: Run full rustqual pass and update baseline

**Run**: `rustqual . --no-fail --compare .ctx/rustqual-baseline.json`

After all tasks, run rustqual and update the baseline:

```bash
rustqual . --no-fail --save-baseline .ctx/rustqual-baseline.json
```

Expected improvements:
- Most BOILERPLATE BP-009 (manual From error conversions) → resolved by typed variants
- Most BP-010 (`.into()` patterns) → resolved in Task 5
- Some BP-001 (inline bail! with string context) → resolved by typed variants

Commit: `git commit -m "quality: update rustqual baseline after miette migration"`

---

## Ordering Dependencies

```
Task 1 (Diagnostic derives)
  └─→ Task 2 (ParseError/InternalPathError in minibox-core)
        └─→ Task 3 (bail! in minibox adapters — uses ParseError)
              └─→ Task 5 (registry.rs .into() — uses new RegistryError variants)
Task 4 (mbx CLI RequestError) — independent, run in parallel with Task 3
Task 6 (daemon boundary) — depends on Tasks 3 + 4
Task 7 (rustqual baseline) — final step
```

Tasks 3 and 4 can run in parallel. Task 6 is the most invasive (touches the
protocol wire format) and must come last.

---

## Risk

- **Protocol wire format change** (Task 6): `DaemonResponse::Error` gains a new
  optional `code` field. The new field uses `#[serde(default)]` so old daemons and
  old clients remain compatible. No migration needed.
- **bail! count**: 268 total usages. Not all will be migrated — `bail!` in test code
  and internal assertion paths can stay as-is. Only user-visible error paths need
  typed variants. Target: eliminate bail! from all non-test, non-internal-assertion
  paths (estimated ~120 of the 268 call sites).
- **macOS/Linux gating**: Some error types in `minibox` are Linux-only
  (`CgroupError`, `NamespaceError`). Their `#[diagnostic]` additions need no
  `#[cfg]` gates — the types are already cfg-gated at the mod level.
