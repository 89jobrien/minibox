# Issue Chain Ordering — Stabilization Sprint

**Date**: 2026-05-14
**Branch**: develop
**Status**: active

Governing constraint: #127 — freeze net-new surface area until fundamentals stabilize. All chains
below serve the stabilization sprint. Feature work (Chain I) is explicitly gated behind Chain E.

---

## Dependency Graph

Key implicit blocking relationships (beyond GitHub `blocked` labels):

```
#156 (remove stale crate refs)
  └── #155 (reconcile docs with 8-crate arch)
        └── #120 (fix docs/feature-status contradictions)
              └── #159 (downgrade platform support claims)
                    └── #115 (publish capability matrix)

#114 (document state persistence model)
  └── #160 (persisted state reconciliation on restart)

#123 (security gotchas -> regression tests)
  └── #157 (security regression suite)
        └── #158 (raise handler/lifecycle error-path coverage)
              └── #129 / #116 (coverage thresholds — parallel)

#117 (define support tiers)
  └── #127 (freeze net-new surface — gates all Chain I features)

#131 (FEATURE_MATRIX as single source of truth)
  └── #132 (add code citations)
        └── #135 (automate Last-updated stamp)
              └── #136 (clarify mandatory vs advisory gates)
                    └── #133 (CI enforcement)
                          └── #127 (formal freeze)
                                └── #117 (support tiers)

#71 / #67 / #62 (conformance suite build/commit/push)
  └── #142–#146 (handler conformance tests — parallel)
        └── #79 (validate on macOS + Linux CI)
              └── #77 (emit Markdown/JSON reports)
                    └── #80 (rootfs metadata + mac adapter regression tests)
                          └── #74 (Tier 3 Linux-only adapter tests)

#161 (centralize adapter registration)
  └── #168 (internalize adapter selection from start-daemon scripts)
```

---

## Chains

### Chain A — Docs Reconciliation

Pure markdown. Zero compile risk. Unblocks all other chains that rely on accurate
architecture claims.

| Step | Issue | Title |
|------|-------|-------|
| 1 | #156 | Audit and remove stale historical crate/binary references |
| 2 | #155 | Reconcile docs with current 8-crate consolidated architecture |
| 3 | #120 | Fix docs/feature-status contradictions between CLAUDE.md and README |
| 4 | #159 | Clarify platform support matrix and downgrade over-broad claims |
| 5 | #115 | Publish platform/backend capability matrix |

**Start condition**: none — begin immediately.
**End condition**: no reference to removed crates outside archive/changelog; README platform
claims match actual implementation; capability matrix published and accurate.

---

### Chain B — Security Regression Suite

Converts prose security gotchas into enforceable regression tests. Feeds coverage chains.

| Step | Issue | Title |
|------|-------|-------|
| 1 | #123 | Turn security gotchas into regression tests and safer abstractions |
| 2 | #157 | Add security regression suite (tar/path/socket/container-init invariants) |
| 3 | #158 | Raise daemon handler/lifecycle error-path test coverage |
| 4a | #129 | Increase daemonbox/handler.rs coverage to 80% function threshold |
| 4b | #116 | Raise coverage on handler.rs and lifecycle/error paths |

Steps 4a and 4b are parallel; both depend on #158.

**Start condition**: #123 can begin immediately. #157 requires #123 complete.
**End condition**: handler.rs >= 80% function coverage; every security invariant in
`docs/SECURITY_INVARIANTS.md` maps to a test.

---

### Chain C — State Model and Restart Correctness

Short chain, high correctness value. Must complete before any work touches restart semantics.

| Step | Issue | Title |
|------|-------|-------|
| 1 | #114 | Clarify and document state persistence model |
| 2 | #160 | Improve persisted state reconciliation on daemon restart |

**Start condition**: none — begin immediately.
**End condition**: `docs/STATE_MODEL.mbx.md` reflects actual `DaemonState` behavior;
restart reconciliation implemented and tested.

---

### Chain D — Conformance Suite Completion

Quality gate for adapter contracts across platforms.

| Step | Issue | Title |
|------|-------|-------|
| 1 | #71 | Build conformance tests |
| 2 | #67 | Commit conformance tests |
| 3 | #62 | Push conformance tests |
| 4 | #142 | pause/resume handler conformance tests |
| 4 | #143 | handle_list conformance tests |
| 4 | #144 | policy rejection conformance tests |
| 4 | #145 | ContainerId validation edge cases |
| 4 | #146 | handle_logs conformance tests |
| 5 | #79 | Validate conformance suite on macOS Colima and Linux CI |
| 6 | #77 | Emit Markdown/JSON conformance reports |
| 7 | #80 | Add regression tests for rootfs metadata and mac adapter wiring |
| 8 | #74 | Tier 3 Linux-only adapter isolation and lifecycle failure tests |

Step 4 issues (#142–#146) are parallel; all depend on steps 1–3 and all block step 5.

**Start condition**: conformance suite crate must exist (verify before starting #71).
**End condition**: conformance suite passes on Linux native and macOS Colima; Markdown +
JSON reports emitted to `artifacts/conformance/` on every CI run.

---

### Chain E — Stability Gate Infrastructure

Creates governance structure that makes #127 operational rather than aspirational.

| Step | Issue | Title |
|------|-------|-------|
| 1 | #131 | Make FEATURE_MATRIX.md single source of truth |
| 2 | #132 | Add code citations for implementation-status claims |
| 3 | #135 | Replace manual Last-updated stamp with automation |
| 4 | #136 | Clarify STABILITY_CHECKLIST.md: mandatory gates vs advisory items |
| 5 | #133 | Add CI enforcement for stability checklist gates |
| 6 | #127 | Formally declare net-new surface freeze |
| 7 | #117 | Define support tiers for crates and tools |

**Start condition**: Chain A should be at step 3+ before step 1 begins (accurate architecture
docs are required to write accurate FEATURE_MATRIX entries).
**End condition**: CI fails on stability checklist violations; support tiers documented;
#127 formally resolved and referenced in CONTRIBUTING.md.

---

### Chain F — Developer UX and Adapter Registration

| Step | Issue | Title |
|------|-------|-------|
| 1 | #162 | Simplify developer tooling entrypoints; document canonical workflow |
| 2 | #161 | Centralize adapter registration; validate adapter selection UX |
| 3 | #168 | Internalize adapter selection from start-daemon scripts into miniboxd |
| 4 | #167 | Add `mbx diagnose <container_id>` subcommand |
| 5 | #166 | Fold preflight.nu checks into `mbx doctor` / `cargo xtask` |

**Start condition**: #162 is docs-only, begin immediately. #161 and #168 require #162 to
avoid writing docs against a moving target.
**End condition**: single canonical workflow documented in DEVELOPMENT.md; `miniboxd`
handles adapter selection internally; `mbx doctor` covers all preflight checks.

---

### Chain G — Parallel Pull Spec

Fully self-contained in `minibox-core/src/image/`. No shared state with any other chain.
Run in parallel with everything else.

| Step | Issue | Title |
|------|-------|-------|
| 1 | #149 | Specify layer storage state machine for parallel pulls |
| 2 | #150 | Formalize LimitedStream contract for layer size enforcement |
| 3 | #151 | Carry layer digest through task failures in parallel pulls |
| 4 | #148 | Define end-to-end failure model for parallel layer pulls |
| 5 | #152 | Expand test plan for parallel layer pull implementation |

**Start condition**: none — begin immediately alongside Chain A.
**End condition**: layer storage state machine documented; LimitedStream semantics specified;
digest propagated through all error paths; full test plan approved.

---

### Chain H — xtask/Scripts Consolidation

Standalone maintenance. Parallel-safe (only touches `crates/xtask/`).

| Step | Issue | Title |
|------|-------|-------|
| 1 | #171 | Remove redundant run-cgroup-tests.{sh,nu} scripts |
| 2 | #170 | Add `cargo xtask check-protocol-sites` |
| 3 | #169 | Add `cargo xtask collect-metrics` |
| 4 | #172 | Add `cargo xtask demo [--adapter smolvm]` |

**Start condition**: none — begin any time. Avoid overlapping with Chain D if both are
actively touching xtask test runners simultaneously (merge conflict risk only).
**End condition**: `scripts/` contains only scripts with no xtask equivalent; all CI guards
run through xtask.

---

### Chain I — Feature Work (GATED)

Do not begin until Chain E resolves #127 and formally lifts the freeze per-feature.

- #94 / #20 — Container networking (veth/bridge)
- #83 — PTY/stdio piping for interactive containers
- #21 — Shared OCI image-pulling library
- #138–#141 — minibox-agent/LLM (multi-turn infer, Ollama, agentic loop, ai-review wiring)
- #76 / #81 / #75 / #41 / #42 / #43 — VZ adapter stack
- #147 / #45 — winbox (Windows Hyper-V/WSL2)

---

## Parallel Execution Slots

| Slot | Chains | Crate/Area Overlap Risk |
|------|--------|------------------------|
| 1 (immediate) | A + G | None — A is docs-only; G is minibox-core/image |
| 2 (after A step 3+) | B + C + E | Low — B: handler.rs tests; C: state.rs; E: docs/CI |
| 3 | D + F | Low — D: conformance crate; F: miniboxd/mbx entrypoints |
| 4 (any time) | H | None — xtask is standalone |
| 5 (post Chain E) | I | High — serialize within slot; shared handler.rs |

---

## Risk Table

| Risk | Issue(s) | Consequence |
|------|----------|-------------|
| Starting Chain I before Chain E | #94, #83, #138–#141, etc. | Feature work expands the surface #127 intends to freeze; gate cannot be enforced retroactively |
| Chain D (#79) before Chain A | #79 | Conformance reports assert platform support that contradicts the uncorrected matrix |
| Chain B (#157) before #123 | #157 | Security test suite misses uncatalogued gotchas; false sense of coverage |
| Chain E (#133) before #131/#132 | #133 | CI enforces a checklist that references inaccurate FEATURE_MATRIX entries |
| #160 before #114 | #160 | Restart reconciliation implemented against an undocumented state model; diverges from whatever #114 specifies |
| Chain H and Chain D simultaneously | #170–#172, #71 | Merge conflicts in xtask test runner wiring only — coordinate branch order |

---

## Starting Sequence

```
Day 1: start Chain A (#156) + Chain G (#149) in parallel
Day 1: start Chain C (#114) independently

When Chain A reaches step 3 (#120):
  start Chain E (#131)

When Chain A completes:
  start Chain B (#123)

When Chain D prerequisites exist:
  start Chain D (#71)

Any time:
  Chain H (#171)

After Chain E resolves #127:
  ungate Chain I — plan separately
```
