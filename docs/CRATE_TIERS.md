# Crate Support Tiers

This document classifies every crate in the minibox workspace by support tier,
defines ownership, and sets the stabilization policy that governs adding new
crates and wiring new adapter suites.

Last updated: 2026-04-26

---

## Tier Definitions

| Tier             | Meaning                                                                                                                          |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| **Core**         | Stable API contract. Breaking changes require a semver bump and a migration note. Highest test coverage expectation.             |
| **Platform**     | Adapter suites for specific execution environments. APIs may evolve; no cross-platform compatibility guarantee between adapters. |
| **Experimental** | Unstable. APIs may change or modules may move without notice. Not suitable for external consumers.                               |
| **Internal**     | Dev tooling. Never shipped as a library or binary in releases.                                                                   |
| **External**     | Non-Rust module. Governed by its own toolchain and release process.                                                              |

---

## Core Tier

These crates define the stable runtime contract. Any API change in a Core crate
that breaks callers outside the workspace is a semver-major event.

| Crate          | Path                  | Role                                                                                                                                                                       |
| -------------- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `minibox-core` | `crates/minibox-core` | Cross-platform shared types: protocol, domain traits, error types, OCI image types, `ImageStore`, `RegistryClient`, `DaemonClient`, preflight. Single source of truth for `DaemonRequest`/`DaemonResponse`. |
| `minibox`      | `crates/minibox`      | Linux container primitives (namespaces, cgroups v2, overlay FS, process init) + daemon handler/server/state. Re-exports `minibox-core` for macro compatibility.            |
| `miniboxd`     | `crates/miniboxd`     | Async daemon entry point. Dispatches to the appropriate platform adapter suite at startup.                                                                                 |
| `mbx`          | `crates/mbx`          | User-facing CLI binary. Command set and flag schema are the public UX contract.                                                                                            |

**Stability expectations for Core crates:**

- `cargo xtask pre-commit` must pass before merging any PR that touches these crates.
- Handler coverage in `minibox/src/daemon/` must stay at or above the current baseline;
  the target is >= 80% function coverage.
- Protocol wire format is pinned by snapshot tests in `minibox-core`. Do not remove or
  rename existing variants without a deprecation period.

---

## Platform Tier

Adapter suites for specific host environments. Each `{platform}box` crate implements
the domain traits from `minibox-core` for its target platform.

| Crate       | Path               | Role                                                                                                                                                        |
| ----------- | ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `macbox`    | `crates/macbox`    | macOS adapter suite: Colima (nerdctl/limactl) and Virtualization.framework (VZ) backends.                                                                   |
| `winbox`    | `crates/winbox`    | Windows adapter suite. Currently a stub — `winbox::start()` returns an error unconditionally. Phase 2 (Named Pipe server, HCS/WSL2 wiring) has not started. |
| `dockerbox` | `crates/dockerbox` | Docker API shim: HTTP-over-Unix-socket bridge that translates Docker API calls to the minibox protocol. Ships the `dockerboxd` binary. (Not yet created.)   |
| `tailbox`   | `crates/tailbox`   | Tailscale/tailnet adapter: auth, config, experimental network experiments. (Not yet created.)                                                               |

**Stability expectations for Platform crates:**

- Each wired adapter (`MINIBOX_ADAPTER` value accepted by the daemon) must have at least
  one integration test exercising the real adapter path before it leaves experimental
  status.
- Platform crates may have platform-conditional compilation (`#[cfg(target_os = ...)]`).
  CI gates clippy on Linux and macOS; Windows clippy is best-effort.

---

## Experimental Tier

Unstable crates. APIs may change or crates may be merged, split, or removed. Do not
take a public dependency on these crates from outside the workspace.

| Crate             | Path                     | Role                                                                                                                       |
| ----------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `minibox-agent`   | `crates/minibox-agent`   | AI agent runtime: error types, LLM step wiring, crux-agentic integration.                                                  |
| `minibox-llm`     | `crates/minibox-llm`     | Multi-provider LLM client with structured output and fallback chains.                                                      |
| `minibox-secrets` | `crates/minibox-secrets` | Typed credential store: env, OS keyring, 1Password, Bitwarden adapters. SHA-256 audit hashes, expiry-aware provider chain. |
| `mbxctl`          | `crates/mbxctl`          | Alternative management CLI (axum-based). WIP — not a shipping binary in the current release.                               |
| `dashbox`         | `crates/dashbox`         | Ratatui TUI dashboard with 6 tabs (Agents, Bench, History, Git, Todos, CI). Run via `just dash`.                           |
| `minibox-bench`   | `crates/minibox-bench`   | Benchmark harness binary: codec and adapter-overhead suites.                                                               |

**Stability expectations for Experimental crates:**

- No API stability guarantee. Breaking changes are acceptable without a version bump
  while the crate remains in this tier.
- Must compile as part of `cargo check --workspace`. Build failures in Experimental
  crates block CI like any other crate.
- May be promoted to Core or Platform tier only after meeting all gates in
  `docs/STABILITY_CHECKLIST.md`.

---

## Internal Tier

Dev tooling. These crates are workspace members but are never published or included
in release binaries.

| Crate            | Path                    | Role                                                                                                               |
| ---------------- | ----------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `xtask`          | `crates/xtask`          | Cargo xtask runner: `pre-commit`, `prepush`, `test-unit`, `test-conformance`, `bench`, `build-vm-image`, and more. |
| `minibox-macros` | `crates/minibox-macros` | Proc-macro crate: `as_any!` and `adapt!` derive macros used by `minibox`.                                          |

**Stability expectations for Internal crates:**

- No external API contract. Can be restructured freely.
- Must not appear in `[dependencies]` of any crate outside the workspace.

---

## External Tier

Non-Rust modules with their own toolchain and release lifecycle.

| Module     | Path        | Language | Role                                                                                                                                                                     |
| ---------- | ----------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `agentbox` | `agentbox/` | Go       | AI agent tooling: `cmd/agentbox/` (council, meta-agent CLI), `cmd/mbx-commit-msg/` (commit message generator). Build: `just agentbox-build`. Test: `just agentbox-test`. |

---

## Stabilization Policy

### No new Core or Platform crates until stabilization gates are met

The workspace currently has 8 shipped crates across all tiers. Adding more crates before
the core runtime is hardened increases maintenance surface without shipping value.

**A new crate MAY be added to the Core or Platform tier only when ALL of the
following gates in `docs/STABILITY_CHECKLIST.md` are green:**

1. Protocol types have a single source of truth (currently met — minibox-core #122/#128).
2. Handler coverage >= 80% function coverage in `minibox/src/daemon/handler.rs`.
3. All wired adapters have at least one integration test.
4. `cargo xtask pre-commit` passes on macOS (fmt + clippy + release build).
5. `cargo xtask test-unit` passes (~300+ tests).
6. `cargo deny check` passes (license + advisory audit).

Until these gates are met:

- New Experimental crates may be added when a clearly scoped capability cannot be
  cleanly housed in an existing crate. The PR must include a rationale comment
  referencing this document.
- New Internal crates (tooling only) may be added freely.
- Wiring a new `MINIBOX_ADAPTER` value requires the adapter integration test gate
  (item 3 above) to be met for the new adapter before it lands on `main`.

### Promotion path

```
Experimental ──(gates met)──► Core or Platform
Internal      ──(never)──► any other tier
```

Promotion requires:

1. A PR that demonstrates the gate criteria are met (coverage report, test output).
2. Review and merge to `main`.
3. Update this document's tier table to reflect the new classification.

### Freeze notice (issues #117 and #127)

This document was created as part of a stabilization milestone declared in issues
#117 and #127. The freeze applies to **net-new Core and Platform crates**. Existing
crates in all tiers continue to receive fixes and enhancements.

The freeze lifts when all six stabilization gates above are verified green on the
`next` branch.
