# rustqual Code Quality Sweep

**Date:** 2026-06-06
**Branch:** develop
**Starting score:** 60.3% (1294 findings, 3511 functions)
**Current score:** 82.4% (320 findings, 2284 scanned functions)

## Completed (this session)

### Config tuning (no code changes)

| Change | Findings removed | Score delta |
|--------|-----------------|-------------|
| Exclude `*/tests/*`, `*/benches/*`, `*/examples/*`, mocks from root `rustqual.toml` | 937 | 60.3% -> 81.9% |
| Remove 24 stale `// qual:allow(complexity)` orphan comments across 16 files | 24 | — (no score change) |
| Disable `detect_wildcard_imports` (all 13 were `use super::*` in `#[cfg(test)]`) | 13 | 81.9% -> 82.4% |
| Exclude `minibox-testsuite/src/*` (test infrastructure) | ~50 | included above |
| Add `qual:allow` for 5 legitimate `.expect()` uses (mutex poison, semaphore, const parse) | 5 | pending re-score |

### Files modified

- `rustqual.toml` — `exclude_files`, `detect_wildcard_imports`
- 16 source files — removed stale `qual:allow` orphan comments
- 5 source files — added justified `qual:allow(complexity)` for `.expect()`

## Remaining findings (320)

### By category

| Category | Count | Weight | Impact | Actionable? |
|----------|-------|--------|--------|-------------|
| TQ_NO_SUT | 57 | 0.10 | 5.7 | Partially — 28 in protocol.rs are false positives (enum variant construction) |
| ERROR_HANDLING | ~6 | 0.18 | 1.1 | 5 suppressed, 1 remaining (`GhcrRegistry::for_test` — test helper) |
| IOSP VIOLATION | 39 | 0.22 | 8.6 | Most are I/O boundary / handler functions — suppress ~30, refactor ~9 |
| BOILERPLATE | 34 | 0.13 | 4.4 | BP-001/002 (3): derive-able. BP-008 (5): clone reduction. BP-009/010 (26): structural |
| FRAGMENT | 24 | 0.13 | 3.1 | Extract shared helpers where genuinely duplicated |
| SRP_MODULE | 17 | 0.18 | 3.1 | Split large files: `run.rs` (995 lines), `server.rs` (561), `state.rs` (521) |
| UNSAFE | 29 | 0.18 | 5.2 | Expected for container runtime — suppress with documented invariants |
| SRP_PARAMS | 16 | 0.18 | 2.9 | Introduce input structs for `build_execution_manifest` (13 params), `build_container_record` (17 params) |
| DUPLICATE | 10 | 0.13 | 1.3 | Extract shared logic from `handle_pause`/`handle_resume`, `preflight` probes |
| MAGIC_NUMBER | 8 | 0.18 | 1.4 | Add to `allowed_magic_numbers` or extract as named constants |
| SRP_STRUCT | 7 | 0.18 | 1.3 | Low LCOM4 cohesion — review struct responsibility boundaries |
| TQ_NO_ASSERT | 5 | 0.10 | 0.5 | Add assertions to signature/smoke tests |
| TQ_UNTESTED | 2 | 0.10 | 0.2 | Add unit tests for 2 production functions |

### By crate

| Crate | Findings | Top categories |
|-------|----------|---------------|
| minibox | ~200 | VIOLATION, FRAGMENT, SRP_MODULE, BOILERPLATE |
| minibox-core | ~60 | TQ_NO_SUT, BOILERPLATE, VIOLATION |
| miniboxd | ~30 | VIOLATION, UNSAFE, TQ_NO_SUT |
| macbox | ~15 | VIOLATION, FRAGMENT |
| mbx | ~10 | BOILERPLATE |
| others | ~5 | — |

## Plan: Next iterations

### Phase 1 — IOSP violations (highest weighted impact)

**Goal:** Reduce 39 violations to ~10 (suppress I/O boundaries, refactor extractable logic).

1. **Suppress handler/dispatch/startup functions** (~25 functions):
   - `handle_*` functions in `daemon/handler/*.rs`
   - `dispatch`, `handle_connection` in `server.rs`
   - `run_daemon`, `build_handler_deps` in `main.rs`
   - `start`, `start_krun` in `macbox/src/lib.rs`
   - Reason: integration roots that inherently mix routing logic with delegation

2. **Refactor extractable logic** (~9 functions):
   - `extract_and_verify_layer` — split verify logic from I/O
   - `RegistryClient::pull_image` — extract manifest resolution from download loop
   - `ImageStore::store_layer_verified` — split verification from store
   - `BridgeNetwork::setup/attach/cleanup/stats` — extract command builders
   - `GhcrRegistry::pull_image` — extract auth flow from pull
   - `TargetPlatform::parse` — extract validation from construction

### Phase 2 — SRP: split large files

| File | Lines | Split strategy |
|------|-------|---------------|
| `daemon/handler/run.rs` | 995 | Extract `prepare_run`, `build_*` helpers into `run/prepare.rs` |
| `daemon/server.rs` | 561 | Extract `dispatch` match arms into `server/dispatch.rs` |
| `daemon/state.rs` | 521 | Extract persistence logic into `state/persistence.rs` |
| `image/registry.rs` (core) | 1100+ | Extract `RegistryClient` methods by concern |

### Phase 3 — SRP: parameter structs

| Function | Params | Fix |
|----------|--------|-----|
| `build_container_record` | 17 | `ContainerRecordInput` struct |
| `build_execution_manifest` | 13 | `ManifestInput` struct |
| `build_spawn_config` | 10 | `SpawnConfigInput` struct |
| `daemon_wait_for_exit` | 9 | `WaitParams` struct |
| `handle_pipeline` | 8 | `PipelineContext` struct |

### Phase 4 — DRY: extract duplicates and fragments

- `handle_pause`/`handle_resume` share identical state-lookup + cgroup logic
  — extract `toggle_freeze(container_id, freeze: bool)`
- `preflight.rs`: `probe_kernel_version`/`parse_kernel_version` duplicated
  — deduplicate
- `format_report` 94% similar — extract shared formatting

### Phase 5 — Config and suppression cleanup

- Add remaining magic numbers to allowlist (8 findings)
- Add `qual:allow` for UNSAFE blocks that already have `// SAFETY:` comments
- Suppress TQ_NO_SUT for `protocol.rs` inline tests (enum variant construction
  is invisible to call-graph tracing)

## Target

| Metric | Current | Target |
|--------|---------|--------|
| Quality score | 82.4% | 92%+ |
| Findings | 320 | <100 |
| IOSP violations | 39 | <10 |
| SRP_MODULE | 17 | <5 |
| SRP_PARAMS | 16 | <5 |

## Risks

- Splitting `run.rs` and `server.rs` touches hot paths — requires full test
  suite pass after each split
- Parameter struct refactors change function signatures — ripple through all
  call sites
- Over-suppressing IOSP violations hides genuine coupling problems

## Doublecheck corrections

During this session, the testing philosophy assessment incorrectly stated
"property tests not in CI coverage gate." Verification shows property tests
ARE in xtask (`test-property` gate) and in the promotion pipeline
(`Tier::Testing`). The plan above does not rely on that incorrect claim.
