# Containerized CI Execution Design

**Date:** 2026-03-19
**Status:** Draft

## Overview

A phased migration from native execution to minibox-containerized execution for all CI workflows — local hooks and GHA jobs. Each phase is independently deliverable and gated on specific minibox capability gaps closing.

The end state: every quality gate (fmt, clippy, nextest, coverage, bench) runs inside an ephemeral `minibox-rust-ci` container. Containers spin up, run their task, and are removed. The workspace is bind-mounted in. The host has no Rust toolchain requirement beyond minibox itself.

macOS GHA jobs remain native permanently — minibox is a Linux runtime.

---

## Phase Map

```
Phase 0  local-store-ghcr-adapter spec  Image infra ready. Hooks + GHA run natively.
Phase 1  stdout/stderr streaming        Smoke test: output pipes from container to CLI.
Phase 2  bind mounts                    Workspace mounted into containers. Full ephemeral loop.
Phase 3  GHA runner                     Self-hosted Linux GHA jobs run in minibox containers.
```

### Phase Dependency Matrix

| Phase | Requires                                                                         |
| ----- | -------------------------------------------------------------------------------- |
| 0     | local-store-ghcr-adapter spec (GhcrRegistry + ImageRef + local store)            |
| 1     | Phase 0 + stdout/stderr streaming + exit code propagation (protocol streaming spec in local-store-ghcr-adapter) |
| 2     | Phase 1 working + bind mount protocol support (`RunContainer.mounts` + `MS_BIND` in `filesystem.rs`) |
| 3     | Phase 2 working + self-hosted runner host provisioned with `miniboxd`            |

Each phase ships only after its prerequisite minibox gaps are *implemented and tested*, not just designed.

---

## Phase 0 — Foundation

**Gap requirements:** local-store-ghcr-adapter spec must land first.

**What changes:**

- `minibox pull ghcr.io/<org>/minibox-rust-ci:stable` works (ghcr adapter + local store)
- `pull-ci-image` xtask added to onboarding
- Hooks and GHA continue to run natively
- Image is cached in `MINIBOX_DATA_DIR/images/ghcr.io/...` and ready for Phase 1

**xtask target:**

```rust
// xtask/src/main.rs
"pull-ci-image" => {
    cmd!("minibox", "pull", "ghcr.io/<org>/minibox-rust-ci:stable").run()?;
}
```

**Value:** Image infrastructure is live. Team can pull and inspect the image. Nothing breaks.

---

## Phase 1 — Stdout Streaming Smoke Test

**Gap requirements:**

- stdout/stderr streaming (`ContainerOutput` / `ContainerStopped` protocol messages — see local-store-ghcr-adapter spec)
- Exit code propagation via `ContainerStopped.exit_code`

**What changes:**

Phase 1 is a validation milestone, not a hook integration. It verifies that `minibox run` streams output to the terminal and propagates exit codes before any hook logic depends on this behavior.

```
# Smoke test
cargo xtask ci-smoke
# → minibox run ghcr.io/<org>/minibox-rust-ci:stable -- rustc --version
# Expected: "rustc 1.XX.X (... ...)" printed to terminal, exit 0
```

**Why not hook integration yet:** Without bind mounts (Phase 2), the container sees the image's empty `/workspace`, not the host workspace. Running `cargo fmt` or `cargo clippy` inside the container would check nothing. Phase 1 confirms the plumbing works; Phase 2 makes it useful.

**Ephemeral containers:** `RunContainer` gains an `ephemeral: bool` field. When `true`, the daemon automatically removes the container (overlay upper dir + state entry) after the process exits. All containerized hook invocations use `ephemeral: true`.

---

## Phase 2 — Workspace Bind Mounts (Full Ephemeral Loop)

**Gap requirements:**

- Bind mount support: `minibox run --mount <host-path>:<container-path>` (Phase 1 complete)
- `RunContainer` protocol gains `mounts: Vec<Mount>` field
- `crates/minibox-lib/src/container/filesystem.rs`: `MS_BIND` mount setup before `pivot_root`

**What changes:**

Hooks mount the host workspace into the container. The container sees live source files.

```rust
// xtask/src/main.rs — Phase 2 pre-commit
"pre-commit" => {
    let workspace = env::current_dir()?;
    let cargo_cache = home_dir()?.join(".mbx/cache/cargo-registry");
    cmd!(
        "minibox", "run",
        "--mount", format!("{}:/workspace", workspace.display()),
        "--mount", format!("{}:/root/.cargo/registry", cargo_cache.display()),
        "ghcr.io/<org>/minibox-rust-ci:stable",
        "--",
        "sh", "-c",
        "cd /workspace && cargo fmt --all --check && \
         cargo clippy -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox -- -D warnings && \
         cargo build --release -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox -p minibox-bench"
    ).run()?;
}
```

**Bind mount path security:** Host paths are validated before mounting:

- `std::fs::canonicalize(host_path)` resolves symlinks and `..`; the canonical path must start with an allowed prefix.
- Allowed prefixes: user's `$HOME` and `/tmp`. Paths outside these are rejected.
- The container path must be absolute and free of `..` components.
- This mirrors the `canonicalize()` + path-escape rejection in `crates/minibox-lib/src/container/filesystem.rs`.

**Cargo cache mount:** `~/.mbx/cache/cargo-registry` is bind-mounted at `/root/.cargo/registry` inside the container. The directory is created on first use if absent. Containers accumulate the registry cache across runs; the overlay upper dir isolates any per-container writes.

**Ephemeral cleanup:** Each `minibox run` with `ephemeral: true` (the default for all xtask invocations) creates a fresh overlay upper dir, removes it on exit, and deletes the state entry. `minibox ps` shows no residual containers after hook completion.

**Build artifact note:** The workspace bind mount is read-write. Container writes to `/workspace/target/` land on the host (intentional — the release binary is available immediately after the hook).

**Container lifecycle per hook invocation:**

```
1. minibox pull (no-op if image cached)
2. minibox run --ephemeral → container created, namespace + overlay + cgroups set up
3. Command executes in container namespaces with mounted workspace
4. Exit code propagated to hook via ContainerStopped.exit_code
5. Container removed automatically (ephemeral), overlay upper dir cleaned up
```

Total overhead vs native: namespace setup + overlay mount (~50-200ms). Acceptable for a pre-push gate.

**Additional xtask targets:**

```rust
// Escape hatch: run any command inside the CI image against the live workspace
"run-ci" => {
    let cmd_args = args; // passed through
    cmd!("minibox", "run", "--mount", ..., "rust-ci", "--", ...cmd_args).run()?;
}

// Non-interactive shell in CI container (TTY not required; output is piped)
"ci-shell" => {
    cmd!("minibox", "run", "--mount", ..., "rust-ci", "--", "/bin/bash").run()?;
}
// Note: interactive TTY (-it) is a future minibox gap; ci-shell is non-interactive.
```

---

## Phase 3 — Self-Hosted GHA Linux Runner

**Gap requirements:**

- All Phase 2 gaps closed
- `miniboxd` stable enough for automated CI workloads
- Self-hosted runner host provisioned

**What changes:**

The self-hosted runner executes GHA job steps inside minibox containers instead of on the bare host.

```yaml
# ci.yml Phase 3 — Linux jobs
test-linux:
  runs-on: [self-hosted, linux, minibox]
  steps:
    - uses: actions/checkout@v4
    - name: Tests
      run: |
        minibox run \
          --mount ${{ github.workspace }}:/workspace \
          ghcr.io/<org>/minibox-rust-ci:stable \
          -- sh -c "cd /workspace && cargo nextest run --workspace --lib"
```

**Runner host requirements:**

- Linux 5.0+, cgroups v2, overlayfs
- `miniboxd` running as root (systemd unit)
- `minibox` CLI in PATH for the runner user
- `MINIBOX_DATA_DIR/images/ghcr.io/.../minibox-rust-ci/stable/` pre-seeded

**Runner provisioning:**

1. Install GHA runner agent; tag with `[self-hosted, linux, minibox]`.
2. Install `miniboxd` systemd service:
   ```ini
   [Unit]
   Description=Minibox Container Daemon
   [Service]
   ExecStart=/usr/local/bin/miniboxd
   Restart=always
   [Install]
   WantedBy=multi-user.target
   ```
3. Run `minibox pull ghcr.io/<org>/minibox-rust-ci:stable` to pre-seed the image cache.
4. Ensure runner user can reach the minibox socket (sudo or socket group).

A provisioning script `scripts/provision-runner.sh` will be created as part of Phase 3 implementation.

**macOS GHA jobs:** Remain native forever. `clippy-macos` and `test-macos` run on `macos-latest` GitHub-hosted runners.

**Nightly integration tests:** In Phase 3, these also run inside minibox containers on the self-hosted runner.

---

## Protocol Changes Required (Minibox Gaps)

| Gap                               | Phase needed | Minibox change                                                                                            |
| --------------------------------- | ------------ | --------------------------------------------------------------------------------------------------------- |
| stdout/stderr pipe                | 1            | `process.rs`: pipe child stdout/stderr; protocol: `ContainerOutput` stream messages (defined in local-store-ghcr-adapter spec) |
| Exit code propagation             | 1            | Protocol: `ContainerStopped { exit_code: i32 }` (defined in local-store-ghcr-adapter spec)               |
| Ephemeral flag                    | 1            | Protocol: `RunContainer` gains `ephemeral: bool`; daemon auto-removes on exit when true                   |
| Bind mount support                | 2            | Protocol: `RunContainer` gains `mounts: Vec<Mount>`; `filesystem.rs`: `MS_BIND` before `pivot_root`      |
| Read-only mount                   | 2            | `Mount` struct gains `readonly: bool`; `MS_BIND | MS_RDONLY` in `filesystem.rs`                          |
| Interactive TTY (-it)             | Future       | Allocate PTY, attach to socket; not required for any Phase 0-3 deliverable                                |

---

## xtask + Justfile Evolution Across Phases

CI task logic lives in `xtask/`. The `Justfile` is a thin shim — every target is one line.

```
# Phase 0 Justfile (thin shim, all logic in xtask)
pre-commit:    cargo xtask pre-commit
prepush:       cargo xtask prepush
pull-ci-image: cargo xtask pull-ci-image

# Phase 1 addition
ci-smoke:      cargo xtask ci-smoke

# Phase 2 additions
run-ci *args:  cargo xtask run-ci {{args}}
ci-shell:      cargo xtask ci-shell
```

The xtask implementation progressively replaces native commands with containerized equivalents. Phase 0 xtask targets call native cargo; Phase 2 targets call `minibox run --mount`. The Justfile never changes between phases.

---

## Files Created/Modified Per Phase

**Phase 0:**
- `xtask/src/main.rs` — `pull-ci-image` target
- `Justfile` — add `pull-ci-image: cargo xtask pull-ci-image`

**Phase 1:** (smoke test)
- `xtask/src/main.rs` — `ci-smoke` target
- `Justfile` — add `ci-smoke: cargo xtask ci-smoke`

**Phase 2:**
- `xtask/src/main.rs` — update `pre-commit`, `prepush` to containerized; add `run-ci`, `ci-shell`
- `Justfile` — no change (already delegates to xtask)

**Phase 3:**
- `.github/workflows/ci.yml` — `test-linux` switches to `[self-hosted, linux, minibox]`
- `scripts/provision-runner.sh` — new file

---

## Success Criteria

**Phase 0:** `just pull-ci-image` downloads `minibox-rust-ci:stable`; image layers present in `MINIBOX_DATA_DIR/images/ghcr.io/...`

**Phase 1:** `just ci-smoke` prints `rustc X.Y.Z` to terminal and exits 0; exit 1 from a failing command propagates to the hook

**Phase 2:**
- `just pre-commit` runs fmt/clippy/build inside a minibox container against the live workspace
- Output visible in terminal as it streams
- `minibox ps` shows no residual containers after completion
- Cargo registry cache in `~/.mbx/cache/cargo-registry` persists across invocations

**Phase 3:**
- `test-linux` GHA job runs inside minibox container on self-hosted runner
- Failure output visible in GHA logs
- Container removed after job step completes
