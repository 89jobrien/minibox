# Minibox — Agent Handoff

Central orientation document for AI agents starting a new session. Update the
**"Current state"** and **"Next up"** sections at the end of each session.

**Last updated:** 2026-03-27
**Current version:** 0.1.0 (workspace Cargo.toml)
**Changelog:** `docs/PRERELEASE_CHANGELOG.md` (v0.0.1 – v0.0.14)

---

## What is minibox?

A Docker-like container runtime in Rust: daemon (`miniboxd`) + CLI (`minibox`).
OCI image pulling, Linux namespace isolation, cgroups v2 resource limits, overlay
filesystem. Daemon/client architecture over a Unix socket (JSON-over-newline protocol).

---

## Crate layout

```
crates/
  minibox-core/    — cross-platform shared types: protocol, domain traits, errors, image mgmt
  linuxbox/        — Linux-specific adapters (compiles everywhere, re-exports minibox-core)
  minibox-macros/  — proc macros: as_any!, default_new!, adapt!
  daemonbox/       — handler/state/server (Unix-safe; macOS/Linux)
  miniboxd/        — unified daemon binary; dispatches by platform
                     Linux  → native handler via daemonbox
                     macOS  → macbox::start()
                     Windows → winbox::start()
  macbox/          — macOS daemon: Colima preflight, adapter wiring, start()
  winbox/          — Windows daemon stub: Named Pipe paths, start() stub
  minibox-cli/     — CLI client (platform-aware socket/pipe path)
  minibox-client/  — shared client library (socket connection, error types)
  minibox-llm/     — multi-provider LLM client with structured output and fallback chains
  minibox-secrets/ — typed credential store (env, keyring, 1Password, Bitwarden adapters)
  minibox-bench/   — benchmark harness (linuxbox only)
  xtask/           — dev tool: pre-commit, test-unit, e2e-suite, coverage
mbxctl/            — axum-based control plane (WIP)
```

**Dependency graph:**

```
miniboxd  ──[linux]──► linuxbox ──► minibox-core
          ──[macos]──► macbox ──► daemonbox ──► linuxbox
          ──[win]────► winbox ──► daemonbox ──► linuxbox
minibox-cli ──► minibox-client ──► linuxbox
minibox-bench ──────────────────► linuxbox
minibox-llm (standalone)
minibox-secrets (standalone)
mbxctl (standalone, axum)
```

---

## Current test counts

| Suite | Count | Platform |
|---|---|---|
| linuxbox unit | 88 (+1 ignored) | any |
| minibox-cli | 36 | any |
| daemonbox lib | 27 (1 failing — see below) | any |
| daemonbox integration (handler + conformance + proptest) | 108 (+3 ignored) | any |
| minibox-llm | 27 | any |
| minibox-secrets | 32 | any |
| cgroup integration | 16 | Linux + root |
| e2e daemon+CLI | 14 | Linux + root |
| existing integration | 8 | Linux + root |

**Known test failure:** `daemonbox::server::tests::test_is_terminal_response_for_each_variant` —
likely needs updating after a protocol change (new response variant not marked terminal/non-terminal).

Coverage snapshot (2026-03-25, `cargo xtask prepush`):

| File | fn% | line% | Notes |
|---|---|---|---|
| `daemonbox/src/handler.rs` | 67.5% | 55% | Biggest gap — error paths in run/pull/stop/rm |
| `linuxbox/src/adapters/ghcr.rs` | 74.5% | 89.7% | New; 4 wiremock tests added this session |
| `daemonbox/src/server.rs` | 100% | 90.6% | Healthy |
| `linuxbox/src/adapters/registry.rs` | 89.5% | 85.8% | Good |

Test files for handler/conformance live in `crates/daemonbox/tests/` (moved from
`crates/miniboxd/tests/` during daemonbox extraction, 2026-03-18).

---

## Git Workflow (3-tier stability pipeline)

**Spec:** `docs/superpowers/specs/2026-03-26-git-workflow-design.md`
**Status:** Implemented. Branches `next` and `stable` exist. `phased-deployment.yml` is live.

```
main (develop) ──auto──► next (validated) ──manual──► stable (release) ──► v* tag
```

- `main`: Active R&D. Must compile. Direct push.
- `next`: Auto-promoted from `main` on green CI. Full test + audit gates.
- `stable`: Manual promote. Maestro-consumable. Tagged releases cut here.

**Remote:** `origin` → `git@github.com:89jobrien/minibox.git` (migration from Gitea complete).

## CI

Workflows: `ci.yml`, `phased-deployment.yml`, `release.yml`, `nightly.yml`

Current `ci.yml` jobs (branch-conditional — see CLAUDE.md for full matrix):
- All branches: `cargo check --workspace` + `cargo fmt --all --check` + clippy
- `next` + `stable`: above + `cargo xtask test-unit` + audit/deny/machete
- `stable` only: above + `cargo geiger`

`phased-deployment.yml`: auto-promote `main→next` on green CI; manual `next→stable` via `workflow_dispatch`; hotfix backmerge.

E2e CI job restored (commit `0b862ad`, 2026-03-27) after streaming regression fix.

**CI phases for self-hosted runner:**
- Phase 1 (now): non-compile gates (audit/deny/machete/geiger)
- Phase 2 (future): compile + test inside minibox containers (dogfooding)

**jobrien-vm status (2026-03-26):** SSH unreachable (100.105.75.7 timeout).

---

## Quality gates (macOS, run before committing)

```bash
cargo fmt --all --check
cargo clippy -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -p minibox-secrets -- -D warnings
cargo xtask test-unit

# Full pre-commit gate:
cargo xtask pre-commit
```

**Never use `--workspace`** for clippy, test, or check — `miniboxd` has
platform-gated code that fails on non-target platforms. Always use `-p` flags.

---

## Key architectural decisions

| Decision | Rationale |
|---|---|
| `daemonbox` is Unix-only (no Windows dep on it) | Windows uses Named Pipe proxy (`winboxd`), not a daemonbox consumer. Avoids large conditional-compilation surface. |
| `miniboxd/src/lib.rs` is a re-export shim | Backward compat after daemonbox extraction; let existing tests compile without surgery. |
| `ServerListener` + `PeerCreds` traits in daemonbox | Generic `run_server<L, F>` accept loop; `UnixServerListener` is the Linux/macOS impl; future `NamedPipeListener` for Windows. |
| `MINIBOX_ADAPTER` env var selects adapter suite | `native` (Linux namespaces) or `gke` (proot, unprivileged). |
| `ImageRef` routes to registry | `[REGISTRY/]NAMESPACE/NAME[:TAG]` — Docker Hub default, ghcr.io if registry prefix is `ghcr.io`. |
| CLI streaming via `ephemeral: true` | `ContainerOutput` / `ContainerStopped` messages stream stdout/stderr; CLI exits with container exit code. |

---

## What's done

All items below are merged to `main`:

- [x] Hexagonal architecture (domain traits, adapters, mocks)
- [x] Security hardening (path traversal, tar Zip Slip, SO_PEERCRED, size limits)
- [x] cgroups v2 resource limits + supervisor leaf cgroup pattern
- [x] E2E test infrastructure (`DaemonFixture`, `just test-e2e-suite`)
- [x] Conformance tests (cross-adapter contract verification)
- [x] Structured tracing contract (CLAUDE.md Tracing Contract section)
- [x] Regression tests (protocol, path validation, manifest parsing)
- [x] PRERELEASE_CHANGELOG (v0.0.1–v0.0.14)
- [x] `daemonbox` crate extracted from `miniboxd`
- [x] CLI streaming (`ContainerOutput` / `ContainerStopped`, `ephemeral` flag)
- [x] Parallel layer pulls
- [x] `GhcrRegistry` adapter + `ImageRef` routing
- [x] `macbox` crate (Colima adapter wiring, macOS `start()`)
- [x] `winbox` crate (Windows stub, Named Pipe paths)
- [x] `ServerListener` / `PeerCreds` in daemonbox
- [x] Cross-platform `miniboxd` dispatch (Linux / macOS / Windows)
- [x] macOS CI job
- [x] `close_range(2)` fast path in `close_extra_fds()` (QEMU-inspired, 2026-03-21)
- [x] `ci-setup` Justfile recipe made resilient (`rm || true`)
- [x] GHCR adapter hardening (2026-03-25, commit `5b89dc1`):
  - P0: fix double-prefix cache key bug (`has_image`/`get_image_layers` now use caller-supplied fully-qualified name)
  - P1: stream layer blobs via `SyncIoBridge` + `spawn_blocking`; no more full-blob buffering
  - P2: `authenticate()` probes actual tag instead of hardcoded `latest`
  - P2: 4 wiremock behavioural tests (cache hit/miss, versioned-tag auth, streaming storage)
  - P3: `GHCR_ORG_ALLOWLIST` env var to restrict which org/repos a shared PAT can pull
  - P3: data/runtime dirs created with `mode(0o700)` to protect layer contents on shared hosts
- [x] 3-tier git workflow: `next`/`stable` branches, `phased-deployment.yml`, remote swap to GitHub
- [x] E2e streaming regression fix + CI job restored (2026-03-27, commit `0b862ad`)
- [x] E2e harness hardened: `serial_test` + polling instead of fixed sleeps (commit `bd41fdf`)
- [x] Bind mounts + privileged mode end-to-end (CLI `--privileged`, `-v`/`--volume`, `--mount` → protocol → handler → process)
- [x] Samply/cargo-flamegraph wired into bench pipeline (commit `9c503f9`)
- [x] `minibox-client` crate (shared client library)
- [x] `minibox-llm` crate (multi-provider LLM client)
- [x] `minibox-secrets` crate (typed credential store)
- [x] `mbxctl` axum control plane skeleton

---

## Next up

### ✅ DONE (2026-03-27, session started same day) - Daemonbox test failure fix + handler coverage

**Fixed test failure** (commit f57b5c9):
- `test_is_terminal_response_for_each_variant` was missing Success, ContainerCreated, ContainerList variants
- Updated `is_terminal_response()` to include all terminal variants (all except ContainerOutput)

**Started handler.rs coverage improvement** (commit 7475cf6):
- Added 2 new error path tests: `test_run_empty_image_no_layers`, `test_pull_registry_failure_with_tag`
- Test count increased 51→52 handler tests
- Documented handler testing patterns in CLAUDE.md for future sessions (commit f910dbd)

`daemonbox/src/handler.rs` is still at ~67.5% function / 55% line coverage. Additional opportunities:
- Directory creation failure paths
- Cleanup-after-error (filesystem/cgroup rollback)
- Network setup edge cases

More error scenarios can be added incrementally using existing mock builder patterns.

### QEMU osdep hardening (from QEMU `util/` audit, 2026-03-21)

Patterns borrowed from QEMU's OS-dependency layer, adapted to Rust/hexagonal architecture.

| Item | Priority | Plan / Notes |
|---|---|---|
| Audit CLOEXEC on daemon listener socket | 1 | Quick security check — verify `daemonbox/src/server.rs` listener sets CLOEXEC. Rust's `UnixListener` does this by default on Linux but confirm. |
| Race-safe PID file for miniboxd | 2 | open + fstat + fcntl(F_SETLK) + stat-verify-inode + ftruncate + write PID. Reference: QEMU `oslib-posix.c:qemu_write_pidfile()`. |
| Systemd socket activation | 3 | Read `LISTEN_PID`/`LISTEN_FDS`, set CLOEXEC on passed FDs, clear env. ~30 lines. Reference: QEMU `systemd.c:check_socket_activation()`. |
| Human-readable size parsing for CLI | 3 | Parse "512M", "2G", "1.5T" for `--memory` flags. Reference: QEMU `cutils.c:qemu_strtosz()`. |

### Ready to execute (no blockers)

| Item | Plan / Notes |
|---|---|
| Linux CI job (self-hosted runner) | Use `mbx:minibox-ci` skill; runner is on jobrien-vm (currently unreachable — SSH timeout as of 2026-03-26) |
| `WslRuntime` executor injection seam | Add `Arc<dyn Fn(&[&str]) -> Result<String>>` to WSL2/Docker Desktop adapters (same pattern as Colima `LimaExecutor`) so they can be unit-tested without real WSL |
| Compile-time tracing field enforcement | Macros/wrappers that enforce canonical field names at compile time; contract is documented in CLAUDE.md |

### Blocked on hardware

| Item | Blocked on |
|---|---|
| `macboxd` e2e tests (`MacboxFixture`) | macOS + Colima machine |
| `winboxd` Named Pipe accept loop (Phase 2) | Windows machine with WSL2 |

### Future / not started

| Item | Notes |
|---|---|
| State persistence | `StateStore` trait exists; HashMap in `state.rs` is current impl |
| `exec` into running container | Needs `setns(2)` + output streaming; blocks maestro integration |
| Container log capture | Stdout/stderr discarded post-`execvp`; needed for `maestro-minibox` Phase 1 |
| Named containers | `ContainerName` field on `RunContainer`; needed for maestro integration |
| Networking (bridge/veth) | No networking setup; containers get isolated net namespace only |
| `minibox-orch` agent orchestrator | See `docs/minibox-orch-design.md`; needs exec/logs/named containers first |
| Native Windows backend | `winboxd` WSL2 proxy is a stepping stone; no plan yet |

---

## Known limitations (don't try to fix without a plan)

- No user namespace remapping — container root = host root (VULN-002 in `docs/CODEBASE_ANALYSIS.md`)
- No networking setup — containers are network-isolated with no bridge/veth
- No `exec` command — cannot run commands in existing containers
- No persistent state — daemon restart loses all container records
- No Dockerfile support — OCI image-only workflow
- `docker_desktop` and `wsl2` adapters exist in `linuxbox` but are **not wired** into `miniboxd`

---

## Runtime paths

| Path | Purpose |
|---|---|
| `/run/minibox/miniboxd.sock` | Unix socket (Linux/macOS) |
| `\\.\pipe\miniboxd` | Named Pipe (Windows, future) |
| `/var/lib/minibox/images/` | Image layer storage (root) |
| `~/.mbx/cache/` | Image layer storage (non-root) |
| `/sys/fs/cgroup/minibox.slice/miniboxd.service/` | Container cgroup root |

Override with: `MINIBOX_SOCKET_PATH`, `MINIBOX_DATA_DIR`, `MINIBOX_RUN_DIR`, `MINIBOX_CGROUP_ROOT`

---

## Docs map

| Doc | Status | Purpose |
|---|---|---|
| `CLAUDE.md` | Current | Primary agent instructions, architecture, tracing contract |
| `HANDOFF.md` | Current (update each session) | Agent orientation — this file |
| `docs/PRERELEASE_CHANGELOG.md` | Current | Per-version change history |
| `docs/TESTING.md` | Current | Test strategy and layer reference |
| `docs/SECURITY.md` | Current | Threat model, disclosure process |
| `docs/SECURITY_FIXES.md` | Historical | Record of 2026-03-15 security hardening |
| `docs/SECURITY_TESTING.md` | Current | Security test procedures |
| `docs/cgroup-findings.md` | Historical | Debug record for cgroup supervisor leaf fix |
| `docs/CODEBASE_ANALYSIS.md` | Partial (2026-03-17, some issues resolved) | Full audit findings |
| `docs/vps-usage.md` | Current | systemd deploy guide |
| `docs/diagrams/` | Current | Crate graph, hexagonal arch, lifecycle diagrams |
| `docs/superpowers/plans/` | All have status frontmatter | Implementation plans |
| `docs/plans/` | All have status frontmatter | Feature plans |
| `docs/minibox-orch-design.md` | `status: future` | Agent orchestrator design |
| `docs/minibox-orch-handoff.md` | `status: future` | Agent orchestrator impl spec |
| `docs/handoff-2026-03-18.md` | `status: superseded` | Historical session handoff |
| `docs/archive/` | Archived | Stale docs (TEST_RESULTS, ZOMBIENET_PATTERNS) |
