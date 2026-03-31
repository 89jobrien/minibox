> **Note (2026-03-20):** This analysis is from 2026-03-17 and is partially stale.
> Several critical issues have been resolved since: handler/conformance tests now compile
> (daemonbox crate, `handler_tests.rs` + `conformance_tests.rs` moved there), structured
> logging/tracing contract finalized (CLAUDE.md), and setuid stripping was audited.
> The security findings in VULN-001 through VULN-011 remain the authoritative vulnerability list
> but some may have been addressed — verify against current code before acting on them.

# Minibox Codebase Analysis Report

**Date**: 2026-03-17
**Scope**: Full codebase analysis across security, architecture, testing, performance, logging, and code quality

## Executive Summary

Minibox demonstrates **strong architectural foundations** -- the hexagonal architecture with trait-based adapters is genuinely well-executed, and the security posture shows deliberate defense-in-depth thinking. However, the analysis reveals **4 critical security vulnerabilities**, **significant test coverage gaps** in security-critical code paths, and **performance bottlenecks** in image pulling that would impact production readiness.

**Key numbers**: 97 tests exist but 31 don't compile on macOS. Zero tests cover path traversal validation or tar extraction security. 11 security findings, 9 performance issues, and systematic logging gaps across the codebase.

---

## Critical Issues (Immediate Action Required)

### 1. SECURITY: Setuid Bit Stripping Is a No-Op

**Files**: `crates/mbx/src/image/layer.rs:99-101`, `crates/mbx/src/adapters/gke.rs:210-215`

The tar extraction code computes safe modes but never applies them -- the `tar` crate doesn't expose `set_mode()` before extraction, and **umask does NOT strip setuid bits**. The GKE `CopyFilesystem` adapter actively _re-applies_ original permissions including setuid/setgid. Container images can deliver setuid root binaries.

**Fix**: Strip setuid/setgid bits post-extraction with `set_permissions(mode & 0o777)`.

### 2. SECURITY: No User Namespace -- Container Root = Host Root

**File**: `crates/mbx/src/container/namespace.rs:17-63`

`CLONE_NEWUSER` is absent from the namespace configuration. Any container escape grants immediate full host root. This is the single biggest architectural security risk.

**Fix**: Add `CLONE_NEWUSER` with UID/GID mapping (map container UID 0 to unprivileged host UID).

### 3. SECURITY: Symlink TOCTOU Race in Tar Extraction

**File**: `crates/mbx/src/image/layer.rs:150-162`

The path validation canonicalizes the parent only when it already exists. A crafted tar archive can create a directory, replace it with a symlink to `/etc`, then extract files through the symlink. Relative symlink targets with `..` components are also not validated.

**Fix**: Post-extraction path verification + reject relative traversal in symlink targets.

### 4. SECURITY: Overlayfs Mount Option String Injection

**File**: `crates/mbx/src/container/filesystem.rs:114-125`

Layer paths containing commas or colons can inject additional mount options into the overlayfs mount string, potentially bypassing `MS_NOSUID | MS_NODEV` flags.

**Fix**: Validate that no path contains `,` or `:` before building mount options.

### 5. TESTS: Handler/Conformance Tests Don't Compile

**Files**: `crates/miniboxd/tests/handler_tests.rs`, `crates/miniboxd/tests/conformance_tests.rs`, `crates/miniboxd/tests/integration_tests.rs`

31 tests across 3 files fail to compile: `HandlerDependencies` is constructed without required `containers_base` and `run_containers_base` fields, plus type inference errors. These tests were written against an older version of the struct.

### 6. TESTS: Zero Coverage on Security-Critical Code

**Files**: `crates/mbx/src/container/filesystem.rs:validate_layer_path`, `crates/mbx/src/image/layer.rs:validate_tar_entry_path`, `crates/mbx/src/image/layer.rs:verify_digest`

The primary defenses against path traversal, Zip Slip attacks, and image tampering have no tests. These are explicitly flagged as "TODO (security-critical)" in CLAUDE.md.

---

## High Priority Improvements

| #   | Category     | Issue                                                              | File(s)                                              |
| --- | ------------ | ------------------------------------------------------------------ | ---------------------------------------------------- |
| 7   | Security     | Container ID not validated before cgroup path construction         | `crates/mbx/src/container/cgroups.rs:45`     |
| 8   | Security     | `devtmpfs` mount exposes all host devices (`/dev/sda`, `/dev/mem`) | `crates/mbx/src/container/filesystem.rs:217` |
| 9   | Security     | `MAX_REQUEST_SIZE` enforced after full read into memory (DoS)      | `crates/miniboxd/src/server.rs:68-96`                |
| 10  | Security     | FD leak window between `clone()` and `close_extra_fds()`           | `crates/mbx/src/container/process.rs:92-94`  |
| 11  | Security     | PID reuse race in `handle_stop` could kill wrong process           | `crates/miniboxd/src/handler.rs:327-348`             |
| 12  | Performance  | Sequential layer downloads (5-layer image = 5x latency)            | `crates/mbx/src/image/registry.rs:351`       |
| 13  | Performance  | `spawn_blocking` contention between spawns and extraction          | `crates/mbx/src/adapters/runtime.rs:125`     |
| 14  | Architecture | `run_inner` has 10 responsibilities in one function                | `crates/miniboxd/src/handler.rs:106-264`             |
| 15  | Tests        | `DaemonState` has zero unit tests for state transitions            | `crates/miniboxd/src/state.rs`                       |
| 16  | Tests        | `server.rs` connection handler completely untested                 | `crates/miniboxd/src/server.rs`                      |
| 17  | Logging      | Raw request content logged -- future secret leakage risk           | `crates/miniboxd/src/server.rs:107,111`              |
| 18  | Docs         | `MAX_LAYER_SIZE` is 10GB but CLAUDE.md says 1GB                    | `crates/mbx/src/image/registry.rs:43`        |

---

## Medium Priority Improvements

| #   | Category     | Issue                                                                       | File(s)                                                                                      |
| --- | ------------ | --------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| 19  | Architecture | Dual error hierarchy (`MiniboxError` + `DomainError`) with `anyhow` erasure | `crates/mbx/src/error.rs`, `crates/mbx/src/domain.rs`                        |
| 20  | Architecture | `AsAny` trait leaks test infrastructure into domain interface               | `crates/mbx/src/domain.rs:75-77`                                                     |
| 21  | Architecture | PID stored in both `ContainerRecord.pid` and `.info.pid`                    | `crates/miniboxd/src/state.rs:22-32`                                                         |
| 22  | Architecture | 5 speculative domain ports with no implementations                          | `crates/mbx/src/domain/extensions.rs`, `crates/mbx/src/domain/networking.rs` |
| 23  | Performance  | `canonicalize(dest)` called per tar entry (~1400 syscalls/layer)            | `crates/mbx/src/image/layer.rs:146`                                                  |
| 24  | Performance  | `CopyFilesystem` blocks async executor for ~600ms                           | `crates/mbx/src/adapters/gke.rs:143`                                                 |
| 25  | Performance  | Stop polling with 250ms sleep instead of notification                       | `crates/miniboxd/src/handler.rs:335`                                                         |
| 26  | Logging      | Zero tracing spans -- no request correlation possible                       | All files                                                                                    |
| 27  | Logging      | All logs use unstructured string interpolation                              | All files                                                                                    |
| 28  | Logging      | Duplicate INFO logs for same PID spawn event (3x)                           | `namespace.rs`, `process.rs`, `handler.rs`                                                   |
| 29  | Code         | `has_parent_dir_component` duplicated in two files                          | `filesystem.rs:23`, `layer.rs:14`                                                            |
| 30  | Code         | Container state uses magic string literals, not typed enum                  | `crates/miniboxd/src/state.rs`                                                               |
| 31  | Code         | Cross-platform adapters compile on Linux without `#[cfg]` guards            | `crates/mbx/src/adapters/mod.rs`                                                     |

---

## Quick Wins (< 1 hour each)

1. **Strip setuid in `CopyFilesystem`** -- Add `mode & 0o6000` after `set_permissions` (30 min)
2. **Fix test compilation** -- Add missing struct fields + type annotations (30 min)
3. **Validate container IDs at handler boundary** -- Use existing `ContainerId::new()` (15 min)
4. **Hoist `canonicalize(dest)` above tar extraction loop** -- 50% fewer syscalls (15 min)
5. **Sanitize request logging** -- Log request type, not full content (15 min)
6. **Deduplicate `has_parent_dir_component`** -- Move to shared module (15 min)
7. **Remove duplicate PID spawn INFO logs** -- Lower 2 of 3 to DEBUG (10 min)
8. **Log PID file write failures** -- Replace `let _ =` with `if let Err` (5 min)

---

## Long-Term Improvements

| Priority | Item                                                        | Effort    |
| -------- | ----------------------------------------------------------- | --------- |
| High     | Add `CLONE_NEWUSER` with UID mapping                        | 1-2 weeks |
| High     | Replace `devtmpfs` with minimal device whitelist            | 2-3 days  |
| High     | Implement state persistence (`StateStore` trait exists)     | 1 week    |
| Medium   | Concurrent layer downloads with `spawn_blocking` extraction | 3-4 hours |
| Medium   | Adopt `pidfd` for PID-reuse-immune process tracking         | 2-3 days  |
| Medium   | Extract container orchestration from `run_inner`            | 1 day     |
| Medium   | Add structured tracing spans to key operations              | 2-3 days  |
| Low      | Replace `waitpid` runtime bridge with channel pattern       | 30 min    |
| Low      | Add JSON log format option for production                   | 1 hour    |
| Low      | Registry auth token caching                                 | 2 hours   |

---

## Performance Targets

| Operation                     | Current Estimate  | Target | Ceiling       |
| ----------------------------- | ----------------- | ------ | ------------- |
| Container start (cached)      | 65-240ms          | 45ms   | 100ms         |
| Container stop (graceful)     | 0-250ms avg 125ms | <10ms  | 500ms         |
| Image pull (5 layers, alpine) | ~2.1s             | ~700ms | 5s            |
| `minibox ps` (100 containers) | ~1ms              | ~1ms   | 10ms          |
| Concurrent starts             | ~10/s             | 100/s  | 50/s minimum  |
| GKE container start (alpine)  | ~700ms            | ~700ms | 2s (inherent) |

---

## Test Coverage Assessment

| Module                    | Coverage           | Priority     |
| ------------------------- | ------------------ | ------------ |
| `protocol.rs`             | High (21 tests)    | Low          |
| `adapters/mocks.rs`       | Adequate (6 tests) | Low          |
| `adapters/gke.rs`         | Good (14 tests)    | Medium       |
| `adapters/registry.rs`    | Minimal (3 tests)  | Medium       |
| `image/layer.rs`          | None               | **Critical** |
| `container/filesystem.rs` | None               | **Critical** |
| `container/cgroups.rs`    | None               | **High**     |
| `image/manifest.rs`       | None               | High         |
| `miniboxd/state.rs`       | None               | High         |
| `miniboxd/server.rs`      | None               | High         |
| `miniboxd/handler.rs`     | Partial (broken)   | **Critical** |
| `minibox-cli/`            | None               | Medium       |

---

## Architecture Compliance

| Pattern                | Status                 | Notes                                                    |
| ---------------------- | ---------------------- | -------------------------------------------------------- |
| Hexagonal Architecture | Compliant              | Domain traits correctly defined, adapters inject cleanly |
| Composition Root       | Compliant              | `main.rs` is the only place concrete types are named     |
| Single Responsibility  | Partial violation      | `run_inner` has ten distinct steps                       |
| Open/Closed            | Compliant              | New adapters addable without domain/handler changes      |
| Dependency Inversion   | Compliant              | `HandlerDependencies` holds only trait objects           |
| Liskov Substitution    | Minor violation        | `AsAny` leaks test infrastructure into domain traits     |
| Interface Segregation  | Partial violation      | 5 speculative ports with no implementations              |
| Async/Sync Boundary    | Functional but fragile | `daemon_wait_for_exit` guesses runtime context           |
| Error Hierarchy        | Inconsistent           | Dual typed error systems with `anyhow` erasure           |

---

## Strengths Worth Preserving

- **Hexagonal architecture** is genuinely well-executed -- `HandlerDependencies` holds only trait objects, composition root is clean
- **Digest verification** occurs before storage (prevents TOCTOU on cached layers)
- **Mount flags** (`MS_NOSUID | MS_NODEV`) correctly applied to overlay and pseudo-filesystem mounts
- **SO_PEERCRED authentication** is the right approach for Unix socket authorization
- **Streaming size enforcement** checks running totals during download, not just `Content-Length` headers
- **Mock adapter design** with builder pattern and call counters enables thorough testing without root/network
- **Environment isolation** -- container processes get explicit minimal `PATH`/`TERM`, not inherited env

---

## Security Vulnerability Summary

| ID       | Severity | Component       | Title                                                   |
| -------- | -------- | --------------- | ------------------------------------------------------- |
| VULN-001 | HIGH     | `layer.rs`      | Symlink attack in tar extraction (TOCTOU)               |
| VULN-002 | HIGH     | `namespace.rs`  | No user namespace -- container root is host root        |
| VULN-003 | HIGH     | `gke.rs`        | CopyFilesystem propagates setuid bits                   |
| VULN-004 | HIGH     | `cgroups.rs`    | Container ID path injection in cgroup path              |
| VULN-005 | MEDIUM   | `server.rs`     | MAX_REQUEST_SIZE enforced after read (DoS)              |
| VULN-006 | MEDIUM   | `registry.rs`   | Registry-controlled digest used in URL construction     |
| VULN-007 | MEDIUM   | `process.rs`    | FD leak window between clone and close_extra_fds        |
| VULN-008 | MEDIUM   | `handler.rs`    | PID reuse race in handle_stop                           |
| VULN-009 | LOW      | `cgroups.rs`    | Hardcoded device major:minor renders io.max ineffective |
| VULN-010 | LOW      | `filesystem.rs` | devtmpfs exposes all host devices                       |
| VULN-011 | LOW      | `gke.rs`        | proot command not restricted in non-root-auth mode      |

---

## Logging Assessment

### Current State

- All logs use **unstructured string interpolation** (no structured fields)
- **Zero tracing spans** across the entire codebase
- **Raw request content** logged at INFO level (security risk)
- Critical operations missing logging (directory creation, semaphore acquisition, state transitions)
- Duplicate INFO logs for the same events
- CLI crate has **zero logging** in command modules

### Recommended Improvements

1. Sanitize raw request logging to prevent future secret leakage
2. Adopt structured fields (`info!(container_id = %id, pid = pid, "container started")`)
3. Add tracing spans to `handle_connection`, `run_inner`, `pull_image`, `stop_inner`
4. Move per-tar-entry logging from DEBUG to TRACE
5. Add non-blocking log output for production (`tracing-appender`)
6. Add request ID correlation for concurrent request debugging
