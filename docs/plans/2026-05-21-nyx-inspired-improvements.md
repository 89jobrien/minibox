# Plan: Nyx-Inspired Improvements

## Goal

Add compile-time path safety (`ValidatedPath` newtype) and a TOML
config file layer to minibox, inspired by Nyx's design principles:
type-level enforcement at boundaries and layered configuration with
explicit bounds.

## Architecture

### Crates affected

| Crate | Change |
|-------|--------|
| `minibox-core` | New `ValidatedPath` type + module; update domain trait signatures |
| `minibox` | Update adapter impls; migrate validation logic to core |
| `miniboxd` | New `config` module; `DaemonConfig`; TOML loader |

### Data flow

```
User request (raw string paths)
  -> protocol deserialization (PathBuf)
  -> handler validates: ValidatedPath::new(path, base_dir)?
  -> domain trait call with ValidatedPath
  -> adapter uses .as_path() for filesystem ops
```

### Path classification

**User-derived (change to `ValidatedPath`):**

| Site | File | Rationale |
|------|------|-----------|
| `RootfsSetup::setup_rootfs` `image_layers` | `domain.rs:677` | From registry, could be tampered |
| `RootfsSetup::setup_rootfs` `container_dir` | `domain.rs:677` | Daemon-constructed but should be validated |
| `BindMount::host_path` | `domain.rs:417` | User-supplied via protocol |
| `BindMount::container_path` | `domain.rs:419` | User-supplied via protocol |
| `ImageLoader::load_image` `path` | `domain.rs:606` | User-supplied |
| `ImageRegistry::get_image_layers` return | `domain.rs:565` | Registry-derived layer paths |

**Daemon-internal (change to `InternalPath`):**

`InternalPath(PathBuf)` is a thin newtype for daemon-constructed
paths that are deliberately unvalidated. It has `Deref<Target=Path>`
for ergonomic reads but cannot be passed where `ValidatedPath` is
expected (and vice versa). The name makes intent explicit: every
path in the codebase is either `ValidatedPath` (user-derived,
checked) or `InternalPath` (daemon-constructed, trusted).

| Site | File | Rationale |
|------|------|-----------|
| `ContainerSpawnConfig::rootfs` | `domain.rs:1021` | Output of validated setup_rootfs |
| `ContainerSpawnConfig::cgroup_path` | `domain.rs:1031` | Daemon-constructed |
| `RootfsLayout::merged_dir` | `domain.rs:781` | Output of setup_rootfs |
| `BackendRootfsMetadata::Overlay::upper_dir` | `domain.rs:755` | Adapter output |
| `ContainerRecord` fields | `extensions.rs:368-374` | Daemon state |
| `ExecutionManifest::manifest_path` | `execution_manifest.rs:37` | Daemon-generated |

## Tech Stack

- Rust 2024
- `toml = "0.8"` (new direct dep in miniboxd; already transitive
  in Cargo.lock)
- `proptest` (existing dev dep) for property tests
- No new deps in minibox-core or minibox

## Tasks

### Task 1: Create ValidatedPath type

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/path.rs`
**Run**: `cargo nextest run -p minibox-core -- validated_path`

1. Write failing tests:

   ```rust
   // crates/minibox-core/src/path.rs (bottom)
   #[cfg(test)]
   mod tests {
       use super::*;
       use std::fs;
       use tempfile::TempDir;

       #[test]
       fn new_rejects_absolute_path() {
           let base = TempDir::new().unwrap();
           let err = ValidatedPath::new(
               Path::new("/etc/passwd"),
               base.path(),
           ).unwrap_err();
           assert!(
               format!("{err:?}").contains("absolute"),
               "error should mention 'absolute': {err:?}"
           );
       }

       #[test]
       fn new_rejects_parent_traversal() {
           let base = TempDir::new().unwrap();
           let err = ValidatedPath::new(
               Path::new("../escape"),
               base.path(),
           ).unwrap_err();
           assert!(
               format!("{err:?}").contains(".."),
               "error should mention '..': {err:?}"
           );
       }

       #[test]
       fn new_accepts_valid_relative_path() {
           let base = TempDir::new().unwrap();
           fs::create_dir_all(base.path().join("sub/dir")).unwrap();
           let vp = ValidatedPath::new(
               Path::new("sub/dir"),
               base.path(),
           ).expect("valid relative path");
           assert!(vp.as_path().ends_with("sub/dir"));
           assert_eq!(vp.base_dir(), base.path());
       }

       #[test]
       fn new_rejects_symlink_escape() {
           let base = TempDir::new().unwrap();
           let outside = TempDir::new().unwrap();
           let link = base.path().join("escape_link");
           std::os::unix::fs::symlink(
               outside.path(),
               &link,
           ).unwrap();
           let err = ValidatedPath::new(
               Path::new("escape_link"),
               base.path(),
           ).unwrap_err();
           assert!(
               format!("{err:?}").contains("outside"),
               "error should mention 'outside': {err:?}"
           );
       }

       #[test]
       fn as_path_returns_inner() {
           let base = TempDir::new().unwrap();
           let sub = base.path().join("x");
           fs::create_dir(&sub).unwrap();
           let vp = ValidatedPath::new(
               Path::new("x"), base.path(),
           ).unwrap();
           assert_eq!(
               vp.as_path().canonicalize().unwrap(),
               sub.canonicalize().unwrap(),
           );
       }

       #[test]
       fn from_absolute_validates_containment() {
           let base = TempDir::new().unwrap();
           let inside = base.path().join("ok");
           fs::create_dir(&inside).unwrap();
           let vp = ValidatedPath::from_absolute(
               &inside, base.path(),
           ).unwrap();
           assert!(vp.as_path().starts_with(base.path()));
       }

       #[test]
       fn from_absolute_rejects_outside() {
           let base = TempDir::new().unwrap();
           let outside = TempDir::new().unwrap();
           let err = ValidatedPath::from_absolute(
               outside.path(), base.path(),
           ).unwrap_err();
           assert!(format!("{err:?}").contains("outside"));
       }

       #[test]
       fn join_validated_revalidates() {
           let base = TempDir::new().unwrap();
           fs::create_dir_all(
               base.path().join("a/b"),
           ).unwrap();
           let vp = ValidatedPath::new(
               Path::new("a"), base.path(),
           ).unwrap();
           let joined = vp.join_validated(
               Path::new("b"),
           ).unwrap();
           assert!(joined.as_path().ends_with("a/b"));
       }

       #[test]
       fn join_validated_rejects_escape() {
           let base = TempDir::new().unwrap();
           fs::create_dir(base.path().join("a")).unwrap();
           let vp = ValidatedPath::new(
               Path::new("a"), base.path(),
           ).unwrap();
           assert!(vp.join_validated(Path::new("../../etc")).is_err());
       }

       #[test]
       fn display_shows_path() {
           let base = TempDir::new().unwrap();
           fs::create_dir(base.path().join("d")).unwrap();
           let vp = ValidatedPath::new(
               Path::new("d"), base.path(),
           ).unwrap();
           let s = format!("{vp}");
           assert!(s.contains("d"), "display should contain 'd': {s}");
       }
   }
   ```

2. Implement `ValidatedPath`:

   ```rust
   // crates/minibox-core/src/path.rs
   use anyhow::{bail, Context, Result};
   use std::path::{Component, Path, PathBuf};

   #[derive(Debug, Clone, PartialEq, Eq, Hash)]
   pub struct ValidatedPath {
       inner: PathBuf,
       base: PathBuf,
   }

   impl ValidatedPath {
       pub fn new(path: &Path, base_dir: &Path) -> Result<Self> {
           if path.is_absolute() {
               bail!(
                   "path validation failed: absolute path \
                    not allowed: {path:?}"
               );
           }
           if has_parent_component(path) {
               bail!(
                   "path validation failed: '..' component \
                    not allowed: {path:?}"
               );
           }
           let full = base_dir.join(path);
           let canonical_base = base_dir
               .canonicalize()
               .with_context(|| {
                   format!("canonicalize base {base_dir:?}")
               })?;
           if let Some(parent) = full.parent() {
               if parent.exists() {
                   let canonical = parent.canonicalize()?;
                   if !canonical.starts_with(&canonical_base) {
                       bail!(
                           "path validation failed: {path:?} \
                            resolves outside base {base_dir:?}"
                       );
                   }
               }
           }
           if full.exists() {
               let canonical = full.canonicalize()?;
               if !canonical.starts_with(&canonical_base) {
                   bail!(
                       "path validation failed: {path:?} \
                        resolves outside base {base_dir:?}"
                   );
               }
           }
           Ok(Self {
               inner: full,
               base: canonical_base,
           })
       }

       pub fn from_absolute(
           abs_path: &Path,
           base_dir: &Path,
       ) -> Result<Self> {
           let canonical_base = base_dir
               .canonicalize()
               .with_context(|| {
                   format!("canonicalize base {base_dir:?}")
               })?;
           let canonical = abs_path
               .canonicalize()
               .with_context(|| {
                   format!("canonicalize path {abs_path:?}")
               })?;
           if !canonical.starts_with(&canonical_base) {
               bail!(
                   "path validation failed: {abs_path:?} \
                    is outside base {base_dir:?}"
               );
           }
           Ok(Self {
               inner: canonical,
               base: canonical_base,
           })
       }

       pub fn as_path(&self) -> &Path {
           &self.inner
       }

       pub fn base_dir(&self) -> &Path {
           &self.base
       }

       pub fn join_validated(
           &self,
           component: &Path,
       ) -> Result<Self> {
           if component.is_absolute() {
               bail!(
                   "join_validated: component must be \
                    relative: {component:?}"
               );
           }
           if has_parent_component(component) {
               bail!(
                   "join_validated: '..' not allowed \
                    in component: {component:?}"
               );
           }
           let joined = self.inner.join(component);
           if joined.exists() {
               let canonical = joined.canonicalize()?;
               if !canonical.starts_with(&self.base) {
                   bail!(
                       "join_validated: {component:?} \
                        escapes base {:?}",
                       self.base
                   );
               }
           }
           Ok(Self {
               inner: joined,
               base: self.base.clone(),
           })
       }
   }

   impl std::fmt::Display for ValidatedPath {
       fn fmt(
           &self,
           f: &mut std::fmt::Formatter<'_>,
       ) -> std::fmt::Result {
           self.inner.display().fmt(f)
       }
   }

   fn has_parent_component(path: &Path) -> bool {
       path.components()
           .any(|c| matches!(c, Component::ParentDir))
   }
   ```

3. Register module in `crates/minibox-core/src/lib.rs`:

   ```rust
   pub mod path;
   ```

4. Add `tempfile` dev-dep to `crates/minibox-core/Cargo.toml` if
   not already present.

5. Verify:

   ```
   cargo nextest run -p minibox-core -- validated_path  -> all green
   cargo clippy -p minibox-core -- -D warnings          -> zero warnings
   ```

6. Run: `git branch --show-current`
   Commit: `feat(minibox-core): add ValidatedPath newtype with
   no-Deref safety`

### Task 2: Create InternalPath type

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/path.rs`
**Run**: `cargo nextest run -p minibox-core -- internal_path`

1. Write failing tests:

   ```rust
   // append to #[cfg(test)] mod tests in path.rs

   #[test]
   fn internal_path_deref_to_path() {
       let ip = InternalPath::new(PathBuf::from("/var/lib/minibox"));
       let p: &Path = &ip;
       assert_eq!(p, Path::new("/var/lib/minibox"));
   }

   #[test]
   fn internal_path_display() {
       let ip = InternalPath::new(PathBuf::from("/tmp/merged"));
       assert_eq!(format!("{ip}"), "/tmp/merged");
   }

   #[test]
   fn internal_path_from_pathbuf() {
       let ip = InternalPath::from(PathBuf::from("/x"));
       assert_eq!(ip.as_ref(), Path::new("/x"));
   }

   #[test]
   fn internal_path_into_pathbuf() {
       let ip = InternalPath::new(PathBuf::from("/y"));
       let pb: PathBuf = ip.into_inner();
       assert_eq!(pb, PathBuf::from("/y"));
   }
   ```

2. Implement:

   ```rust
   // in crates/minibox-core/src/path.rs

   /// A daemon-internal path that is deliberately unvalidated.
   ///
   /// Used for paths constructed by trusted daemon code (rootfs
   /// outputs, cgroup paths, container state dirs). The newtype
   /// makes intent explicit: this path was NOT derived from user
   /// input and does not need traversal validation.
   ///
   /// Has `Deref<Target=Path>` for ergonomic reads. Cannot be
   /// passed where `ValidatedPath` is expected.
   #[derive(Debug, Clone, PartialEq, Eq, Hash,
            serde::Serialize, serde::Deserialize)]
   #[serde(transparent)]
   pub struct InternalPath(PathBuf);

   impl InternalPath {
       pub fn new(path: PathBuf) -> Self {
           Self(path)
       }

       pub fn into_inner(self) -> PathBuf {
           self.0
       }
   }

   impl std::ops::Deref for InternalPath {
       type Target = Path;
       fn deref(&self) -> &Path {
           &self.0
       }
   }

   impl AsRef<Path> for InternalPath {
       fn as_ref(&self) -> &Path {
           &self.0
       }
   }

   impl From<PathBuf> for InternalPath {
       fn from(p: PathBuf) -> Self {
           Self(p)
       }
   }

   impl std::fmt::Display for InternalPath {
       fn fmt(
           &self,
           f: &mut std::fmt::Formatter<'_>,
       ) -> std::fmt::Result {
           self.0.display().fmt(f)
       }
   }
   ```

3. Verify:

   ```
   cargo nextest run -p minibox-core -- internal_path  -> green
   cargo clippy -p minibox-core -- -D warnings         -> zero
   ```

4. Commit: `feat(minibox-core): add InternalPath newtype for
   daemon-constructed paths`

### Task 3: Add property tests for ValidatedPath

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/path.rs`
**Run**: `cargo nextest run -p minibox-core -- proptest_validated`

1. Write property tests:

   ```rust
   // append to #[cfg(test)] mod tests in path.rs
   mod proptest_validated {
       use super::*;
       use proptest::prelude::*;
       use tempfile::TempDir;

       proptest! {
           #[test]
           fn valid_path_is_within_base(
               name in "[a-z]{1,8}"
           ) {
               let base = TempDir::new().unwrap();
               std::fs::create_dir(
                   base.path().join(&name),
               ).unwrap();
               let vp = ValidatedPath::new(
                   Path::new(&name), base.path(),
               ).unwrap();
               let canonical_base = base.path()
                   .canonicalize().unwrap();
               let canonical_path = vp.as_path()
                   .canonicalize().unwrap();
               prop_assert!(
                   canonical_path.starts_with(&canonical_base)
               );
           }

           #[test]
           fn traversal_always_rejected(
               prefix in "[a-z]{0,4}",
               suffix in "[a-z]{1,4}"
           ) {
               let base = TempDir::new().unwrap();
               let evil = format!("{prefix}/../{suffix}");
               let result = ValidatedPath::new(
                   Path::new(&evil), base.path(),
               );
               prop_assert!(result.is_err());
           }
       }
   }
   ```

2. Verify:

   ```
   cargo nextest run -p minibox-core -- proptest_validated  -> green
   ```

3. Commit: `test(minibox-core): add property tests for
   ValidatedPath`

### Task 4: Update fuzz harness

**Crate**: `minibox-core`
**File(s)**: `crates/minibox/fuzz/fuzz_targets/fuzz_validate_tar_path.rs`
**Run**: `cargo fuzz run fuzz_validate_tar_path -- -max_total_time=30`

1. Update fuzz target to call `ValidatedPath::new()` alongside
   existing `validate_layer_path`:

   ```rust
   use minibox_core::path::ValidatedPath;

   fuzz_target!(|data: &[u8]| {
       if let Ok(s) = std::str::from_utf8(data) {
           let path = Path::new(s);
           let dest = std::env::temp_dir();
           let _ = validate_layer_path(path);
           // Also exercise the newtype constructor
           let _ = ValidatedPath::new(path, &dest);
       }
   });
   ```

2. Verify fuzz runs without crash for 30s.

3. Commit: `test(minibox): update fuzz harness for ValidatedPath`

### Task 5: Update domain trait signatures

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain.rs`
**Run**: `cargo check -p minibox-core`

1. Add `use crate::path::ValidatedPath;` to domain.rs.

2. Change these signatures:

   ```rust
   // RootfsSetup::setup_rootfs (line ~677)
   fn setup_rootfs(
       &self,
       image_layers: &[ValidatedPath],
       container_dir: &ValidatedPath,
   ) -> Result<RootfsLayout>;

   // RootfsSetup::cleanup (line ~690)
   fn cleanup(&self, container_dir: &ValidatedPath) -> Result<()>;

   // ChildInit::pivot_root (line ~723) — stays &Path
   // (daemon-internal, constructed from RootfsLayout::merged_dir)

   // ImageRegistry::get_image_layers (line ~565)
   fn get_image_layers(
       &self, name: &str, tag: &str,
   ) -> Result<Vec<ValidatedPath>>;

   // ImageLoader::load_image (line ~606)
   async fn load_image(
       &self,
       path: &ValidatedPath,
       name: &str,
       tag: &str,
   ) -> anyhow::Result<()>;
   ```

3. Change `BindMount` (line ~415):

   ```rust
   pub struct BindMount {
       pub host_path: ValidatedPath,
       pub container_path: std::path::PathBuf,
       // container_path stays PathBuf — it's a container-
       // internal absolute path, not a host traversal risk
       pub read_only: bool,
   }
   ```

   Note: `BindMount` has serde derives. `ValidatedPath` does not
   implement `Serialize`/`Deserialize` because it requires a
   base_dir for construction. The protocol layer will deserialize
   as `PathBuf` and validate in the handler. `BindMount` in domain
   uses `ValidatedPath`; a separate `wire::BindMount` in protocol
   uses `PathBuf`.

4. Change daemon-internal sites to `InternalPath`:

   ```rust
   use crate::path::InternalPath;

   // ContainerSpawnConfig (line ~1018)
   pub struct ContainerSpawnConfig {
       pub rootfs: InternalPath,
       // ...
       pub cgroup_path: InternalPath,
       // ... rest unchanged
   }

   // RootfsLayout (line ~778)
   pub struct RootfsLayout {
       pub merged_dir: InternalPath,
       // ... rest unchanged
   }

   // BackendRootfsMetadata::Overlay (line ~754)
   Overlay {
       upper_dir: InternalPath,
       // ...
   }
   ```

   Also in `extensions.rs`:

   ```rust
   // ContainerRecord (line ~368)
   pub container_dir: InternalPath,
   pub rootfs: InternalPath,
   pub cgroup_path: InternalPath,
   ```

   And `execution_manifest.rs`:

   ```rust
   pub manifest_path: Option<InternalPath>,
   ```

   `ChildInit::pivot_root` changes from `&Path` to
   `&InternalPath` — since `InternalPath` has
   `Deref<Target=Path>`, callers can pass `&ip` directly.

5. This will NOT compile yet — adapters need updating (Task 6-8).

6. Commit: `refactor(minibox-core): change domain signatures to
   ValidatedPath + InternalPath`

### Task 6: Update adapter implementations

**Crate**: `minibox`
**File(s)**: All files in `crates/minibox/src/adapters/` that
implement `RootfsSetup`, `ImageRegistry`, or `ImageLoader`
**Run**: `cargo check -p minibox`

Affected adapter files (from context map):
- `filesystem.rs` — `RootfsSetup` impl
- `colima.rs` — `RootfsSetup` + `ImageRegistry` impls
- `smolvm.rs` — `ContainerRuntime` (no path trait changes)
- `registry.rs` — `ImageRegistry` impl
- `ghcr.rs` — `ImageRegistry` impl
- `image_loader.rs` — `ImageLoader` impl
- `gke.rs` — `RootfsSetup` impl
- `vf.rs` — `RootfsSetup` impl
- `wsl2.rs` — `RootfsSetup` impl
- `hcs.rs` — `RootfsSetup` impl
- `docker_desktop.rs` — `RootfsSetup` impl

1. For each adapter's `setup_rootfs`: change parameter types,
   use `.as_path()` at filesystem call sites. Return
   `RootfsLayout` with `InternalPath` fields:
   `InternalPath::new(merged_dir)`.

2. For each `get_image_layers`: wrap returned paths in
   `ValidatedPath::from_absolute(path, images_base)?`.

3. For `ImageLoader::load_image`: change `path: &Path` to
   `path: &ValidatedPath`, use `.as_path()` internally.

4. Migrate `validate_layer_path` logic from `filesystem.rs:50`
   — callers now receive `ValidatedPath` so the manual validation
   call in `filesystem.rs:131` becomes redundant and is removed.

5. In `ContainerSpawnConfig` construction sites (handlers),
   wrap daemon-constructed paths:
   `rootfs: InternalPath::new(layout.merged_dir.into_inner())`
   or `rootfs: layout.merged_dir` (since both are `InternalPath`).

6. Commit: `refactor(minibox): update adapters for ValidatedPath
   + InternalPath`

### Task 7: Update mock adapters

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/adapters/mocks.rs`
**Run**: `cargo check -p minibox`

1. Update all mock trait impls to accept `ValidatedPath` params.
2. Mock `get_image_layers` returns
   `ValidatedPath::from_absolute(...)`.

3. Commit: `refactor(minibox): update mock adapters for
   ValidatedPath`

### Task 8: Update handlers

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handler/run.rs`,
`handler/image.rs`, `handler/pipeline.rs`
**Run**: `cargo check -p minibox`

1. In `handle_run`: validate bind mount host paths from the
   protocol request into `ValidatedPath` before constructing
   `BindMount`:

   ```rust
   let validated_host = ValidatedPath::from_absolute(
       &wire_mount.host_path,
       &allowed_mount_base,
   ).context("bind mount host_path validation")?;
   ```

2. In `handle_run`: validate image layer paths returned by
   `get_image_layers` (already `ValidatedPath` after Task 5).

3. In `handle_run`: validate `container_dir` before passing to
   `setup_rootfs`.

4. Verify full workspace compiles:
   ```
   cargo check --workspace
   cargo clippy --workspace -- -D warnings
   ```

5. Commit: `refactor(minibox): validate paths in handlers with
   ValidatedPath`

### Task 9: Run full test suite

**Crate**: (workspace)
**Run**: `cargo xtask verify`

1. Run `cargo xtask verify` — fix any remaining compilation or
   test failures.
2. Run `cargo nextest run --workspace` to verify all tests pass.
3. Commit any fixes: `fix(minibox): resolve ValidatedPath
   migration test failures`

### Task 10: Add DaemonConfig type

**Crate**: `miniboxd`
**File(s)**: `crates/miniboxd/src/config.rs`
**Run**: `cargo nextest run -p miniboxd -- config`

1. Add `toml = "0.8"` to `crates/miniboxd/Cargo.toml`
   dependencies.

2. Write failing tests:

   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn empty_toml_produces_defaults() {
           let cfg: DaemonConfig = toml::from_str("")
               .expect("empty TOML");
           assert!(cfg.adapter.is_none());
           assert!(cfg.log_level.is_none());
           assert!(cfg.policy.allow_privileged.is_none());
       }

       #[test]
       fn parses_full_config() {
           let toml = r#"
               adapter = "smolvm"
               log_level = "debug"

               [policy]
               allow_privileged = false
               max_image_size_mb = 1024
           "#;
           let cfg: DaemonConfig = toml::from_str(toml)
               .expect("valid TOML");
           assert_eq!(
               cfg.adapter.as_deref(),
               Some("smolvm"),
           );
           assert_eq!(
               cfg.policy.max_image_size_mb,
               Some(1024),
           );
       }

       #[test]
       fn env_overrides_file() {
           let file_cfg = DaemonConfig {
               adapter: Some("krun".into()),
               ..Default::default()
           };
           let merged = file_cfg.with_env_overrides();
           // When MINIBOX_ADAPTER is not set, file value
           // is preserved
           assert_eq!(
               merged.adapter.as_deref(),
               Some("krun"),
           );
       }

       #[test]
       fn profile_dev_sets_defaults() {
           let cfg = DaemonConfig::profile("dev");
           assert_eq!(
               cfg.log_level.as_deref(),
               Some("debug"),
           );
           assert_eq!(cfg.policy.allow_privileged, Some(true));
       }

       #[test]
       fn profile_production_sets_defaults() {
           let cfg = DaemonConfig::profile("production");
           assert_eq!(
               cfg.log_level.as_deref(),
               Some("info"),
           );
           assert_eq!(
               cfg.policy.allow_privileged,
               Some(false),
           );
       }

       #[test]
       fn missing_file_returns_defaults() {
           let cfg = DaemonConfig::load_from_path(
               Path::new("/nonexistent/config.toml"),
           );
           assert!(cfg.adapter.is_none());
       }

       #[test]
       fn invalid_toml_returns_error() {
           let result = toml::from_str::<DaemonConfig>(
               "not valid [[[ toml",
           );
           assert!(result.is_err());
       }
   }
   ```

3. Implement:

   ```rust
   // crates/miniboxd/src/config.rs
   use anyhow::{Context, Result};
   use serde::Deserialize;
   use std::path::{Path, PathBuf};

   #[derive(Debug, Clone, Deserialize, Default)]
   pub struct DaemonConfig {
       #[serde(default)]
       pub adapter: Option<String>,
       #[serde(default)]
       pub log_level: Option<String>,
       #[serde(default)]
       pub socket_path: Option<PathBuf>,
       #[serde(default)]
       pub state_dir: Option<PathBuf>,
       #[serde(default)]
       pub images_dir: Option<PathBuf>,
       #[serde(default)]
       pub policy: PolicyConfig,
   }

   #[derive(Debug, Clone, Deserialize, Default)]
   pub struct PolicyConfig {
       #[serde(default)]
       pub allow_privileged: Option<bool>,
       #[serde(default)]
       pub allow_bind_mounts: Option<bool>,
       #[serde(default)]
       pub max_image_size_mb: Option<u64>,
   }

   impl DaemonConfig {
       pub fn load() -> Result<Self> {
           let mut cfg = Self::default();

           // Layer 1: system config
           cfg = cfg.merge(Self::load_from_path(
               Path::new("/etc/minibox/config.toml"),
           ));

           // Layer 2: user config
           if let Ok(home) = std::env::var("HOME") {
               let user_path = PathBuf::from(home)
                   .join(".config/minibox/config.toml");
               cfg = cfg.merge(
                   Self::load_from_path(&user_path),
               );
           }

           // Layer 3: env var overrides
           cfg = cfg.with_env_overrides();

           Ok(cfg)
       }

       pub fn load_from_path(path: &Path) -> Self {
           match std::fs::read_to_string(path) {
               Ok(content) => {
                   toml::from_str(&content).unwrap_or_else(
                       |e| {
                           tracing::warn!(
                               path = %path.display(),
                               error = %e,
                               "config: invalid TOML, \
                                using defaults"
                           );
                           Self::default()
                       },
                   )
               }
               Err(_) => Self::default(),
           }
       }

       pub fn profile(name: &str) -> Self {
           match name {
               "dev" => Self {
                   log_level: Some("debug".into()),
                   policy: PolicyConfig {
                       allow_privileged: Some(true),
                       allow_bind_mounts: Some(true),
                       ..Default::default()
                   },
                   ..Default::default()
               },
               "production" => Self {
                   log_level: Some("info".into()),
                   policy: PolicyConfig {
                       allow_privileged: Some(false),
                       allow_bind_mounts: Some(false),
                       max_image_size_mb: Some(2048),
                   },
                   ..Default::default()
               },
               _ => {
                   tracing::warn!(
                       profile = name,
                       "config: unknown profile, \
                        using defaults"
                   );
                   Self::default()
               }
           }
       }

       pub fn with_env_overrides(mut self) -> Self {
           if let Ok(v) = std::env::var("MINIBOX_ADAPTER") {
               self.adapter = Some(v);
           }
           if let Ok(v) = std::env::var("MINIBOX_LOG_LEVEL")
           {
               self.log_level = Some(v);
           }
           if let Ok(v) = std::env::var("MINIBOX_SOCKET") {
               self.socket_path = Some(PathBuf::from(v));
           }
           if let Ok(v) = std::env::var("MINIBOX_STATE_DIR")
           {
               self.state_dir = Some(PathBuf::from(v));
           }
           if let Ok(v) = std::env::var("MINIBOX_IMAGES_DIR")
           {
               self.images_dir = Some(PathBuf::from(v));
           }
           self
       }

       fn merge(self, other: Self) -> Self {
           Self {
               adapter: other.adapter.or(self.adapter),
               log_level: other.log_level.or(self.log_level),
               socket_path: other.socket_path
                   .or(self.socket_path),
               state_dir: other.state_dir
                   .or(self.state_dir),
               images_dir: other.images_dir
                   .or(self.images_dir),
               policy: PolicyConfig {
                   allow_privileged: other
                       .policy
                       .allow_privileged
                       .or(self.policy.allow_privileged),
                   allow_bind_mounts: other
                       .policy
                       .allow_bind_mounts
                       .or(self.policy.allow_bind_mounts),
                   max_image_size_mb: other
                       .policy
                       .max_image_size_mb
                       .or(self.policy.max_image_size_mb),
               },
           }
       }
   }
   ```

4. Register module in `crates/miniboxd/src/main.rs`:
   `mod config;`

5. Verify:

   ```
   cargo nextest run -p miniboxd -- config  -> all green
   cargo clippy -p miniboxd -- -D warnings  -> zero
   ```

6. Commit: `feat(miniboxd): add DaemonConfig with TOML loading
   and env overrides`

### Task 11: Wire config into daemon startup

**Crate**: `miniboxd`
**File(s)**: `crates/miniboxd/src/main.rs`
**Run**: `cargo check -p miniboxd`

1. In `main()`, load config before adapter resolution:

   ```rust
   let config = config::DaemonConfig::load()
       .context("load daemon config")?;
   info!(
       adapter = ?config.adapter,
       log_level = ?config.log_level,
       "config loaded"
   );
   ```

2. Use `config.adapter` to influence adapter selection (feed
   into existing `MINIBOX_ADAPTER` resolution or replace it).

3. Use `config.policy` fields to set execution policy defaults.

4. Verify: `cargo check -p miniboxd`

5. Commit: `feat(miniboxd): wire DaemonConfig into daemon startup`

### Task 12: Add cargo xtask lint-paths

**Crate**: `xtask`
**File(s)**: `xtask/src/main.rs` (or new `xtask/src/lint_paths.rs`)
**Run**: `cargo xtask lint-paths`

1. Add a `lint-paths` subcommand that:
   - Reads `crates/minibox-core/src/domain.rs` and all files in
     `crates/minibox-core/src/domain/`
   - Finds `fn ` lines containing `PathBuf` or `&Path` in
     trait method signatures
   - Checks against an allowlist of daemon-internal sites
   - Exits non-zero if un-allowlisted raw path params found

2. Add `lint-paths` to `cargo xtask verify` gate.

3. Write a test that the current codebase passes the lint.

4. Commit: `feat(xtask): add lint-paths gate for ValidatedPath
   enforcement`

### Task 13: Final verification

**Crate**: (workspace)
**Run**: `cargo xtask verify`

1. Run full verification gate.
2. Run `cargo nextest run --workspace`.
3. Commit any remaining fixes.
