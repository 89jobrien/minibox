---
title: test-linux dogfood — run full Linux test suite via minibox on macOS
status: approved
date: 2026-04-01
---

# test-linux: Run Linux Tests via Minibox on macOS

## Goal

`cargo xtask test-linux` runs the complete minibox Linux test suite (cgroup,
e2e, integration, sandbox) on a macOS developer machine using minibox itself as
the execution layer. No SSH, no manual VM steps. Minibox all the way down.

## End-to-End Flow

```
cargo xtask test-linux
  1. cargo xtask build-test-image   (cached; skips if image is fresh)
  2. minibox load ~/.mbx/test-image/mbx-tester.tar
  3. minibox run --privileged mbx-tester /run-tests.sh
     └─ streams stdout/stderr back to terminal
     └─ exits with container's exit code
```

The outer `miniboxd` runs on macOS with `MINIBOX_ADAPTER=colima`.
The inner `miniboxd` (inside the container) runs with `MINIBOX_ADAPTER=native`
and has full access to cgroups v2, overlayfs, and Linux namespaces inside the
Colima VM.

## New Components

### 1. `cargo xtask build-test-image`

New xtask command implemented in `crates/xtask/src/test_image.rs`.

**What it does:**

1. Cross-compiles the following for `aarch64-unknown-linux-musl`:
   - `miniboxd` binary
   - `minibox` (CLI) binary
   - `cgroup_tests` test binary (`cargo test -p miniboxd --test cgroup_tests --no-run`)
   - `e2e_tests` test binary
   - `integration_tests` test binary
   - `sandbox_tests` test binary
2. Pulls an Alpine musl base layer (reuses `vm_image.rs` fetch patterns).
3. Assembles an OCI image tarball at `~/.mbx/test-image/mbx-tester.tar`:
   - Layer 0: Alpine base (for `/bin/sh`, coreutils)
   - Layer 1: minibox binaries + test binaries + entrypoint script
4. Writes an OCI `index.json` + `manifest.json` compatible with the
   `LoadImage` handler.

**Cache logic:** skips rebuild if `mbx-tester.tar` mtime is newer than the
newest `.rs` source file in the workspace. `--force` bypasses the cache.

**Entrypoint — `/run-tests.sh`:**

```sh
#!/bin/sh
set -e
MINIBOX_ADAPTER=native

echo "=== cgroup_tests ==="
/usr/local/bin/cgroup_tests --test-threads=1 --nocapture

echo "=== integration_tests ==="
/usr/local/bin/integration_tests --test-threads=1 --ignored --nocapture

echo "=== e2e_tests ==="
MINIBOX_TEST_BIN_DIR=/usr/local/bin /usr/local/bin/e2e_tests --test-threads=1 --nocapture

echo "=== sandbox_tests ==="
MINIBOX_TEST_BIN_DIR=/usr/local/bin /usr/local/bin/sandbox_tests --test-threads=1 --ignored --nocapture

echo "=== all Linux tests passed ==="
```

**Image layout:**

```
mbx-tester:latest
├── /usr/local/bin/miniboxd
├── /usr/local/bin/minibox
├── /usr/local/bin/cgroup_tests
├── /usr/local/bin/e2e_tests
├── /usr/local/bin/integration_tests
├── /usr/local/bin/sandbox_tests
└── /run-tests.sh
```

### 2. `LoadImage` — new protocol command

New `DaemonRequest` variant in both `minibox-core/src/protocol.rs` and
`mbx/src/protocol.rs`:

```rust
LoadImage {
    /// Absolute path to a local OCI image tarball.
    path: String,
}
```

New `DaemonResponse` variant:

```rust
ImageLoaded {
    /// The image name registered in the store (e.g. "mbx-tester:latest").
    name: String,
}
```

**Handler (`daemonbox/src/handler.rs`):**

- Reads the OCI tarball at `path`
- Extracts layers into `ImageStore` (reuses existing layer extraction logic)
- Parses `index.json` → `manifest.json` to get image name/tag and layer digests
- Registers the image so subsequent `RunContainer` requests can find it

**Colima adapter:** delegates to `nerdctl load -i <path>` (or
`ctr images import <path>`) inside the Lima VM via `LimaExecutor`. The image
then lives in the Colima containerd store, accessible to `ColimaRuntime`.

**Native adapter (Linux):** loads directly into `ImageStore` on disk — no
subprocess needed.

**CLI (`minibox-cli`):**

```
minibox load <path>
```

New subcommand wired up in `minibox-cli/src/commands/`.

### 3. `cargo xtask test-linux`

New entry in `crates/xtask/src/gates.rs`:

```rust
pub fn test_linux(sh: &Shell) -> Result<()> {
    // 1. build image (cached)
    build_test_image(sh, false)?;

    // 2. load into minibox
    cmd!(sh, "minibox load {image_path}").run()?;

    // 3. run — privileged, ephemeral, stream output
    cmd!(sh, "minibox run --privileged mbx-tester /run-tests.sh").run()?;

    Ok(())
}
```

Wired in `crates/xtask/src/main.rs` as `cargo xtask test-linux` and
`cargo xtask build-test-image`.

Also wired in `Justfile` as `just test-linux`.

## Existing Code Reused

| Existing | Reused for |
|---|---|
| `vm_image.rs` Alpine fetch + musl cross-compile | `build-test-image` layer assembly |
| `mbx/src/image/layer.rs` tar extraction | `LoadImage` handler |
| `LimaExecutor` | Colima `load` delegation |
| `DaemonFixture` | unchanged — works inside container |
| `--privileged` flag (already wired) | container gets full Linux capabilities |

## Sequence: First Run

```
$ cargo xtask test-linux
[build-test-image] cross-compiling for aarch64-unknown-linux-musl...
[build-test-image] fetching Alpine base layer...
[build-test-image] assembling mbx-tester.tar...
[load] importing mbx-tester:latest into image store...
[run] starting mbx-tester (privileged)...
=== cgroup_tests ===
test cgroup_create ... ok
...
=== e2e_tests ===
test test_e2e_pull_alpine ... ok
...
=== all Linux tests passed ===
```

## Error Handling

- `build-test-image` fails fast if the musl linker is not found; prints install hint.
- `LoadImage` rejects paths that don't exist or aren't valid OCI tarballs.
- `minibox run` exits with the container's exit code; non-zero propagates through xtask.
- If Colima is not running, `minibox load` fails with the existing `NoBackendAvailable` error.

## Out of Scope

- Windows / VZ paths (VZ is blocked by Apple OS bug; Windows has no Colima)
- Pushing `mbx-tester` to a registry (local-only tarball is sufficient)
- Running unit tests (macOS-native) inside the container — those stay on the host via `cargo xtask test-unit`
