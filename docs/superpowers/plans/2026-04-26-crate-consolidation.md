# Crate Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate minibox workspace from 13 crates to 7, reducing
internal coupling, simplifying the publish chain, and making the project
easier to navigate.

**Architecture:** Merge `minibox-oci` and `minibox-client` into
`minibox-core` (the single published library). Merge `linuxbox` and
`daemonbox` into a unified `minibox` runtime crate. Absorb
`minibox-testers` into `minibox-core` behind `test-utils` feature. Drop
`minibox-llm` (orphan, zero dependents). Keep `macbox`, `winbox`,
`miniboxd`, `mbx`, `xtask`, `minibox-macros` as separate crates.

**Tech Stack:** Rust 2024 edition, cargo workspace, serde, tokio, nix

---

## Pre-Consolidation Dependency Graph

```
minibox-macros (proc-macro, 85 LOC)
  -> minibox-oci (3464 LOC)
       -> minibox-core (4463 LOC)
            -> minibox-client (270 LOC)
            -> linuxbox (8750 LOC)
            -> daemonbox (3832 LOC)
                 -> macbox (2122 LOC)
                 -> winbox (100 LOC)
            -> minibox-testers (1427 LOC, dev-only)
  minibox-llm (492 LOC, orphan — zero dependents)
  miniboxd (679 LOC, binary)
  mbx (1940 LOC, CLI binary)
  xtask (3213 LOC, dev-only)
```

## Post-Consolidation Target

```
minibox-macros (proc-macro)                     [PUBLISHED]
  -> minibox-core (core + oci + client)         [PUBLISHED]
       -> minibox (linuxbox + daemonbox)        [NOT published]
            -> macbox                           [NOT published]
            -> winbox                           [NOT published]
  miniboxd (binary)                             [NOT published]
  mbx (CLI binary)                              [NOT published]
  xtask (dev-only)                              [NOT published]
```

Crate count: 13 -> 8 (7 + xtask). Published: 4 -> 2.

## File Structure Changes

### Crates removed (absorbed):
- `crates/minibox-oci/` -> absorbed into `crates/minibox-core/`
- `crates/minibox-client/` -> absorbed into `crates/minibox-core/`
- `crates/minibox-testers/` -> absorbed into `crates/minibox-core/`
  (behind `test-utils` feature)
- `crates/linuxbox/` -> renamed to `crates/minibox/`
- `crates/daemonbox/` -> absorbed into `crates/minibox/`
- `crates/minibox-llm/` -> deleted

### Crates kept as-is:
- `crates/minibox-macros/` (proc-macro must be separate)
- `crates/macbox/`
- `crates/winbox/`
- `crates/miniboxd/`
- `crates/mbx/`
- `crates/xtask/`

## Phasing Strategy

Each phase produces a compiling, tested workspace. Phases are ordered by
dependency depth — leaf merges first, then work inward.

---

### Phase 0: Pre-work — license files and publish guards

**Files:**
- Create: `LICENSE-MIT`
- Create: `LICENSE-APACHE`
- Modify: `crates/minibox-llm/Cargo.toml` (delete entire crate)
- Modify: `Cargo.toml` (remove minibox-llm from workspace members)
- Modify: 7 Cargo.toml files (add `publish = false`)

- [ ] **Step 1: Create LICENSE-MIT**

Create `LICENSE-MIT` at repo root:

```text
MIT License

Copyright (c) 2026 Joseph O'Brien

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

- [ ] **Step 2: Create LICENSE-APACHE**

Create `LICENSE-APACHE` at repo root with the full Apache License 2.0
text (317 lines). Copy from https://www.apache.org/licenses/LICENSE-2.0.txt
with copyright line: `Copyright 2026 Joseph O'Brien`.

- [ ] **Step 3: Delete minibox-llm crate**

Remove `crates/minibox-llm/` directory entirely. Remove
`"crates/minibox-llm"` from `Cargo.toml` workspace members list. Remove
any `minibox-llm` entry from `[workspace.dependencies]` if present.

- [ ] **Step 4: Add `publish = false` to non-published crates**

Add `publish = false` under `[package]` in these Cargo.toml files:
- `crates/linuxbox/Cargo.toml`
- `crates/daemonbox/Cargo.toml`
- `crates/macbox/Cargo.toml`
- `crates/winbox/Cargo.toml`
- `crates/miniboxd/Cargo.toml`
- `crates/mbx/Cargo.toml`

(xtask and minibox-testers already have it.)

- [ ] **Step 5: Verify workspace compiles**

Run: `cargo check --workspace`
Run: `cargo xtask pre-commit`
Expected: clean pass

- [ ] **Step 6: Commit**

```bash
git add LICENSE-MIT LICENSE-APACHE Cargo.toml crates/*/Cargo.toml
git commit -m "chore: add license files, drop minibox-llm, \
add publish = false guards"
```

---

### Phase 1: Absorb minibox-oci into minibox-core

This is the first merge because minibox-oci is a leaf dependency of
minibox-core (core depends on oci, nothing else depends on oci
directly).

**Files:**
- Move: `crates/minibox-oci/src/image/` ->
  `crates/minibox-core/src/image/` (replace re-export with real code)
- Move: `crates/minibox-oci/src/error.rs` -> merge into
  `crates/minibox-core/src/error.rs`
- Move: `crates/minibox-oci/src/lib.rs` top-level `pull()` fn ->
  `crates/minibox-core/src/image/mod.rs` or `src/pull.rs`
- Delete: `crates/minibox-oci/` directory
- Modify: `Cargo.toml` workspace members (remove minibox-oci)
- Modify: `crates/minibox-core/Cargo.toml` (absorb oci deps: reqwest,
  bytes, futures, sha2, hex, tar, flate2, tokio-util, pin-project-lite)
- Modify: `crates/minibox-core/src/lib.rs` (replace `pub use
  minibox_oci::image` with `pub mod image`)
- Modify: every crate that had `minibox-oci` as dependency (only
  minibox-core — via workspace dep)
- Update: `[workspace.dependencies]` remove minibox-oci entry

- [ ] **Step 1: Copy minibox-oci source into minibox-core**

Move `crates/minibox-oci/src/image/` to `crates/minibox-core/src/image/`
(replacing the re-export). Move `crates/minibox-oci/src/error.rs` types
(`ImageError`, `RegistryError`) into `crates/minibox-core/src/error.rs`.
Move the top-level `pull()` function into
`crates/minibox-core/src/image/mod.rs` as `pub async fn pull(...)`.

- [ ] **Step 2: Update minibox-core/Cargo.toml**

Add all minibox-oci dependencies that minibox-core doesn't already have:
`bytes`, `futures`, `sha2`, `hex`, `tar`, `flate2`, `tokio-util`,
`pin-project-lite`, `reqwest`. Remove `minibox-oci` from deps.

Add to dev-dependencies: `wiremock`, `proptest` (if not already present
from oci's dev-deps).

Add `fuzzing` feature forwarding if crate had one.

- [ ] **Step 3: Update minibox-core/src/lib.rs**

Replace:
```rust
pub use minibox_oci::image;
```
With:
```rust
pub mod image;
```

Move re-exports from minibox-oci's lib.rs into minibox-core's lib.rs:
```rust
pub use image::ImageStore;
pub use image::reference::{ImageRef, ImageRefError};
pub use image::registry::RegistryClient;
```

Add the `pull()` re-export:
```rust
pub use image::pull;
```

- [ ] **Step 4: Fix internal `use minibox_oci::` paths**

Within the moved image modules, replace any `use crate::error::` paths
that referred to minibox-oci's error module to use
`crate::error::ImageError` / `crate::error::RegistryError` from
minibox-core's unified error module.

Replace `use minibox_macros::` with `crate::` re-exports where needed.

- [ ] **Step 5: Delete minibox-oci crate**

Remove `crates/minibox-oci/` directory. Remove `"crates/minibox-oci"`
from workspace members. Remove `minibox-oci` from
`[workspace.dependencies]`.

- [ ] **Step 6: Update downstream crate references**

Search all Cargo.toml and `.rs` files for `minibox_oci` or
`minibox-oci`. Update any remaining references to use `minibox_core`
instead. Key sites:
- `crates/linuxbox/Cargo.toml` — should not directly depend on oci
- `crates/minibox-macros/` — oci depended on macros, not the reverse
- Any test files importing `minibox_oci::*`

- [ ] **Step 7: Move minibox-oci tests**

Move `crates/minibox-oci/src/image/*.rs` test modules along with the
source. Move any integration tests from `crates/minibox-oci/tests/` into
`crates/minibox-core/tests/`.

- [ ] **Step 8: Verify**

Run: `cargo check --workspace`
Run: `cargo xtask test-unit`
Run: `cargo xtask pre-commit`
Expected: clean pass, same test count (minus any oci-specific
integration tests that need path fixup)

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: absorb minibox-oci into minibox-core"
```

---

### Phase 2: Absorb minibox-client into minibox-core

minibox-client depends only on minibox-core and thiserror. Small crate
(270 LOC). The `default_socket_path()`, `DaemonClient`,
`DaemonResponseStream` move into minibox-core under a `client` module.

**Files:**
- Move: `crates/minibox-client/src/` ->
  `crates/minibox-core/src/client/`
- Delete: `crates/minibox-client/`
- Modify: `Cargo.toml` workspace (remove member)
- Modify: `crates/minibox-core/src/lib.rs` (add `pub mod client`)
- Modify: `crates/mbx/Cargo.toml` (replace minibox-client dep with
  minibox-core)
- Modify: `crates/mbx/src/` (replace `use minibox_client::` with
  `use minibox_core::client::`)

- [ ] **Step 1: Copy minibox-client source into minibox-core**

Create `crates/minibox-core/src/client/` directory. Copy:
- `crates/minibox-client/src/lib.rs` ->
  `crates/minibox-core/src/client/mod.rs`
- `crates/minibox-client/src/error.rs` ->
  `crates/minibox-core/src/client/error.rs`
- `crates/minibox-client/src/socket.rs` ->
  `crates/minibox-core/src/client/socket.rs`

- [ ] **Step 2: Update minibox-core/src/lib.rs**

Add:
```rust
pub mod client;
```

Add re-exports for backward compat in this phase:
```rust
pub use client::{DaemonClient, DaemonResponseStream, DaemonWriter};
pub use client::default_socket_path;
```

- [ ] **Step 3: Fix internal paths in client modules**

In `client/mod.rs` and `client/socket.rs`, replace:
- `use minibox_core::protocol::*` -> `use crate::protocol::*`
- `use minibox_core::*` -> `use crate::*`

- [ ] **Step 4: Update mbx crate**

In `crates/mbx/Cargo.toml`: remove `minibox-client` dep.

In `crates/mbx/src/**/*.rs`: replace all `use minibox_client::` with
`use minibox_core::client::`.

- [ ] **Step 5: Delete minibox-client crate**

Remove `crates/minibox-client/`. Remove from workspace members and
`[workspace.dependencies]`.

- [ ] **Step 6: Verify**

Run: `cargo check --workspace`
Run: `cargo xtask test-unit`
Expected: clean pass

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: absorb minibox-client into minibox-core"
```

---

### Phase 3: Absorb minibox-testers into minibox-core (test-utils feature)

minibox-testers (1427 LOC) provides mocks, fixtures, conformance
helpers. It depends on minibox-core, linuxbox, daemonbox. After Phase 4
(linuxbox+daemonbox merge), this crate's deps collapse to minibox-core +
minibox. For now, move the parts that only depend on minibox-core into
`minibox-core` behind `test-utils`, and defer the rest to Phase 4.

**Important:** This phase must run AFTER Phase 4 if any testers modules
depend on linuxbox/daemonbox internals. Check at execution time. If all
testers code can compile against minibox-core alone, do this phase
before Phase 4 instead.

**Files:**
- Move: `crates/minibox-testers/src/` ->
  `crates/minibox-core/src/testing/` (behind `test-utils` feature)
- Delete: `crates/minibox-testers/`
- Modify: `Cargo.toml` workspace (remove member)
- Modify: all crates that had `minibox-testers` as dev-dep (linuxbox,
  daemonbox, miniboxd) — replace with `minibox-core = { features =
  ["test-utils"] }`
- Modify: all test files importing `minibox_testers::*` — replace with
  `minibox_core::testing::*`

- [ ] **Step 1: Audit minibox-testers deps on linuxbox/daemonbox**

Check which modules in `crates/minibox-testers/src/` import from
`linuxbox` or `daemonbox`. If any do, those modules must wait until
after Phase 4 (when linuxbox+daemonbox become `minibox`). Move only
the minibox-core-only modules in this phase.

- [ ] **Step 2: Move testers source into minibox-core**

Create `crates/minibox-core/src/testing/`. Copy all `.rs` files from
`crates/minibox-testers/src/`. Gate the module:

In `crates/minibox-core/src/lib.rs`:
```rust
#[cfg(feature = "test-utils")]
pub mod testing;
```

- [ ] **Step 3: Update test-utils feature deps**

In `crates/minibox-core/Cargo.toml`, ensure the `test-utils` feature
pulls in `tempfile` (already does) plus any other deps from
minibox-testers: `anyhow`, `async-trait`, `serde`, `serde_json`, `tokio`
(most already present as non-optional deps).

- [ ] **Step 4: Update downstream dev-deps**

In `crates/linuxbox/Cargo.toml`, `crates/daemonbox/Cargo.toml`,
`crates/miniboxd/Cargo.toml`: remove `minibox-testers` from
dev-dependencies. Ensure `minibox-core = { features = ["test-utils"] }`
is present.

- [ ] **Step 5: Update all test imports**

Search for `use minibox_testers::` across all `.rs` files. Replace with
`use minibox_core::testing::`.

- [ ] **Step 6: Delete minibox-testers crate**

Remove `crates/minibox-testers/`. Remove from workspace members and
`[workspace.dependencies]`.

- [ ] **Step 7: Verify**

Run: `cargo check --workspace`
Run: `cargo xtask test-unit`
Expected: clean pass

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor: absorb minibox-testers into minibox-core \
test-utils feature"
```

---

### Phase 4: Merge linuxbox + daemonbox into unified `minibox` crate

This is the largest phase. `linuxbox` (8750 LOC) and `daemonbox`
(3832 LOC) merge into a single `minibox` crate. The directory is
`crates/minibox/` (rename `crates/linuxbox/`).

**Key concern:** The `as_any!` and `adapt!` macros expand to
`crate::domain::AsAny` — they resolve at the call site. After the
rename from `linuxbox` to `minibox`, the macros still expand to
`crate::domain::AsAny`, which works because `minibox` re-exports
`minibox_core::domain`. No macro changes needed.

**Files:**
- Rename: `crates/linuxbox/` -> `crates/minibox/`
- Rename: `crates/linuxbox/Cargo.toml` name field `linuxbox` ->
  `minibox`
- Move: `crates/daemonbox/src/` -> `crates/minibox/src/daemon/`
- Delete: `crates/daemonbox/`
- Modify: `Cargo.toml` workspace members (linuxbox -> minibox, remove
  daemonbox)
- Modify: `[workspace.dependencies]` (linuxbox -> minibox, remove
  daemonbox)
- Modify: `crates/minibox/Cargo.toml` (absorb daemonbox deps: dashmap,
  tracing-subscriber, prometheus-client, axum, opentelemetry-*, chrono)
- Modify: `crates/minibox/src/lib.rs` (add `pub mod daemon`)
- Modify: `crates/macbox/Cargo.toml` (linuxbox -> minibox, daemonbox ->
  minibox)
- Modify: `crates/winbox/Cargo.toml` (daemonbox -> minibox)
- Modify: `crates/miniboxd/Cargo.toml` (linuxbox -> minibox, daemonbox
  -> minibox)
- Modify: all `.rs` files with `use linuxbox::` or `use daemonbox::`
- Modify: benchmarks in `crates/linuxbox/benches/` (update crate name)

- [ ] **Step 1: Rename crates/linuxbox -> crates/minibox**

```bash
mv crates/linuxbox crates/minibox
```

In `crates/minibox/Cargo.toml`: change `name = "linuxbox"` to
`name = "minibox"`.

In root `Cargo.toml`: change `"crates/linuxbox"` to `"crates/minibox"`
in workspace members. Update `[workspace.dependencies]`:
- Remove `linuxbox = { path = "crates/linuxbox" }`
- Add `minibox = { path = "crates/minibox" }`

- [ ] **Step 2: Copy daemonbox source into minibox**

Create `crates/minibox/src/daemon/`. Move all daemonbox source files:
- `crates/daemonbox/src/handler.rs` -> `crates/minibox/src/daemon/handler.rs`
- `crates/daemonbox/src/server.rs` -> `crates/minibox/src/daemon/server.rs`
- `crates/daemonbox/src/state.rs` -> `crates/minibox/src/daemon/state.rs`
- `crates/daemonbox/src/network_lifecycle.rs` -> `crates/minibox/src/daemon/network_lifecycle.rs`
- `crates/daemonbox/src/telemetry.rs` -> `crates/minibox/src/daemon/telemetry.rs`
- `crates/daemonbox/src/lib.rs` -> `crates/minibox/src/daemon/mod.rs`

In `crates/minibox/src/lib.rs`, add:
```rust
pub mod daemon;
```

- [ ] **Step 3: Absorb daemonbox deps into minibox Cargo.toml**

Add to `crates/minibox/Cargo.toml` from daemonbox's deps (skip
duplicates): `dashmap`, `tracing-subscriber`.

Add optional deps behind features:
```toml
[features]
metrics = ["dep:prometheus-client", "dep:axum"]
otel = [
    "dep:opentelemetry",
    "dep:opentelemetry_sdk",
    "dep:opentelemetry-otlp",
    "dep:tracing-opentelemetry",
]
```

- [ ] **Step 4: Fix internal paths in daemon modules**

In all `crates/minibox/src/daemon/*.rs` files:
- Replace `use minibox_core::` with `use crate::` (via re-exports) or
  `use minibox_core::` (direct — both work since minibox depends on
  minibox-core)
- Replace `use linuxbox::` with `use crate::` (now same crate)
- Remove `use daemonbox::` — now `use crate::daemon::`

- [ ] **Step 5: Delete daemonbox crate**

Remove `crates/daemonbox/`. Remove from workspace members and
`[workspace.dependencies]`.

- [ ] **Step 6: Update all downstream crates**

In `crates/macbox/Cargo.toml`:
- Replace `linuxbox` -> `minibox`
- Replace `daemonbox` -> `minibox`

In `crates/winbox/Cargo.toml`:
- Replace `daemonbox` -> `minibox`

In `crates/miniboxd/Cargo.toml`:
- Replace `linuxbox` -> `minibox`
- Replace `daemonbox` -> `minibox`
- Update feature forwarding: `metrics = ["minibox/metrics"]`,
  `otel = ["minibox/otel"]`

- [ ] **Step 7: Global find-replace of use paths**

Search all `.rs` files for:
- `use linuxbox::` -> `use minibox::` (but check for ambiguity with
  the binary crate `mbx` which won't import this)
- `use daemonbox::` -> `use minibox::daemon::`
- `linuxbox::` in string literals (test names, tracing targets) ->
  `minibox::`

Also update `extern crate` statements if any exist.

- [ ] **Step 8: Update benchmarks**

In `crates/minibox/benches/trait_overhead.rs` and
`protocol_codec.rs`: update any `use linuxbox::` to `use minibox::`.

Update `[[bench]]` sections in Cargo.toml if the bench harness
references the crate name.

- [ ] **Step 9: Move daemonbox tests**

Move `crates/daemonbox/tests/` into `crates/minibox/tests/daemon/` or
inline. Update imports.

- [ ] **Step 10: Verify**

Run: `cargo check --workspace`
Run: `cargo xtask test-unit`
Run: `cargo xtask pre-commit`
Expected: clean pass

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "refactor: merge linuxbox + daemonbox into unified \
minibox crate"
```

---

### Phase 5: Configurable default adapter + smolvm cross-platform

Make the default adapter suite trivially switchable (single const) and
update the smolvm adapter docs to reflect that smolmachines supports
both macOS and Linux (libkrun-based, not VZ-only).

**Files:**
- Modify: `crates/miniboxd/src/main.rs`
- Modify: `crates/linuxbox/src/adapters/smolvm.rs` (or
  `crates/minibox/src/adapters/smolvm.rs` after Phase 4)

- [ ] **Step 1: Extract DEFAULT_ADAPTER_SUITE const in miniboxd**

In `crates/miniboxd/src/main.rs`, add a const above `AdapterSuite`:

```rust
/// Default adapter suite when `MINIBOX_ADAPTER` is unset.
///
/// Change this single value to switch the default runtime for all
/// platforms. Current options: `"native"`, `"gke"`, `"colima"`,
/// `"smolvm"`.
const DEFAULT_ADAPTER_SUITE: &str = "native";
```

Update `AdapterSuite::from_env()`:

```rust
fn from_env() -> Result<Self> {
    let val = std::env::var("MINIBOX_ADAPTER")
        .unwrap_or_else(|_| DEFAULT_ADAPTER_SUITE.to_string());
    match val.as_str() {
        "native" => Ok(Self::Native),
        "gke" => Ok(Self::Gke),
        "colima" => Ok(Self::Colima),
        "smolvm" => Ok(Self::SmolVm),
        other => anyhow::bail!(
            "unknown MINIBOX_ADAPTER value {other:?} \
             (expected \"native\", \"gke\", \"colima\", or \"smolvm\")"
        ),
    }
}
```

- [ ] **Step 2: Update smolvm.rs module docs for cross-platform**

Replace the module-level doc comment to reflect that smolmachines
uses libkrun (not Apple VZ-only) and works on both macOS and Linux:

```rust
//! SmolVM adapter suite — lightweight Linux VMs via smolmachines.
//!
//! Delegates container operations into a smolmachines VM. smolmachines
//! uses libkrun (a lightweight VMM) to boot Linux VMs with sub-second
//! cold starts. Works on both macOS (Apple Silicon / Intel) and Linux.
//!
//! # How it works
//!
//! Each adapter runs commands inside a smolmachines VM using
//! `smolvm machine run --image <image> -- <command>`. The VM boots a
//! Linux kernel, provides cgroups v2, overlay FS, and network isolation
//! inside the guest. Images are cached locally after first pull.
//!
//! # Adapter selection
//!
//! Selected by `MINIBOX_ADAPTER=smolvm`. These adapters are compiled on
//! all platforms.
//!
//! # Requirements
//!
//! - smolmachines installed (https://smolmachines.com)
//!   - macOS: `brew install smolvm`
//!   - Linux: see smolmachines docs
//! - macOS with Apple Silicon or Intel, or Linux x86_64/aarch64
```

- [ ] **Step 3: Update SmolVmLimiter and SmolVmFilesystem doc comments**

Remove "macOS host side" language — replace with "host side" since it
now applies to both platforms. Affected doc comments:
- `SmolVmFilesystem`: "on the host side" (already correct)
- `SmolVmLimiter`: "on the macOS host side" -> "on the host side"

- [ ] **Step 4: Verify**

Run: `cargo check --workspace`
Run: `cargo xtask test-unit`
Expected: clean pass, no test regressions

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(miniboxd): extract DEFAULT_ADAPTER_SUITE const, \
update smolvm docs for cross-platform"
```

---

### Phase 6: Update release pipeline and documentation

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/ci.yml`
- Modify: `CLAUDE.md`
- Modify: `README.md` (if exists)
- Modify: `docs/` as needed

- [ ] **Step 1: Update release.yml publish chain**

The publish chain shrinks from 4 crates to 2:

```yaml
- name: Publish crates
  env:
    CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
  run: |
    cargo publish -p minibox-macros
    sleep 30
    cargo publish -p minibox-core
```

Remove `minibox-oci` and `minibox-client` publish steps.

- [ ] **Step 2: Update ci.yml clippy targets**

Replace any `-p linuxbox` or `-p daemonbox` clippy targets with
`-p minibox`. Remove `-p minibox-oci` and `-p minibox-client` if
listed individually.

- [ ] **Step 3: Update CLAUDE.md**

Update the workspace structure documentation, crate list, and
architecture overview to reflect 8-crate workspace. Key sections:
- "Architecture Overview / Workspace Structure"
- "12 crates in cargo workspace" -> "8 crates"
- linuxbox/daemonbox references -> minibox
- minibox-oci references -> minibox-core
- minibox-client references -> minibox-core::client

- [ ] **Step 4: Update xtask**

Check `crates/xtask/src/` for hardcoded crate names (`linuxbox`,
`daemonbox`, `minibox-oci`, `minibox-client`). Update to new names.

- [ ] **Step 5: Verify full CI locally**

Run: `cargo xtask pre-commit`
Run: `cargo xtask prepush` (on Linux/VPS)
Expected: all gates pass

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore: update CI, docs, and xtask for \
consolidated workspace"
```

---

### Phase 7: Dry-run publish and tag

- [ ] **Step 1: Verify CARGO_REGISTRY_TOKEN is set**

```bash
gh secret list -R 89jobrien/minibox | grep CARGO_REGISTRY_TOKEN
```

- [ ] **Step 2: Dry-run publish**

```bash
cargo publish -p minibox-macros --dry-run
cargo publish -p minibox-core --dry-run
```

Both must succeed. Common failures: missing `description`, missing
license file, unpublished dependency.

- [ ] **Step 3: Promote main -> next -> stable**

Trigger `phased-deployment.yml` or:
```bash
git checkout next && git merge main && git push
git checkout stable && git merge next && git push
git checkout main
```

- [ ] **Step 4: Tag and release**

```bash
git tag v0.21.0 stable
git push origin v0.21.0
```

This triggers `release.yml` which publishes to crates.io and creates
the GitHub Release.

---

## Rollback Strategy

Each phase is a single commit. If a phase breaks CI:
1. `git revert <commit>` to restore previous state
2. Fix the issue on a branch
3. Re-apply

The workspace compiles after every phase, so partial progress is safe.

## Risk Register

| Risk | Mitigation |
|------|------------|
| `as_any!`/`adapt!` macro path breakage after linuxbox rename | Macros use `crate::domain::AsAny` — works because minibox re-exports `minibox_core::domain`. Verify in Phase 4 Step 10. |
| Benchmark regressions from crate merge | Run `cargo bench -p minibox` after Phase 4; compare trait_overhead and protocol_codec results. |
| crates.io publish fails (new dep graph) | Phase 6 dry-run catches this before tagging. |
| Test count regression | Compare `cargo nextest run --lib 2>&1 | tail -1` before Phase 1 and after Phase 5. Must be equal or higher. |
| `minibox-testers` has linuxbox/daemonbox deps | Phase 3 Step 1 audits this. If deps exist, defer those modules to after Phase 4. |
