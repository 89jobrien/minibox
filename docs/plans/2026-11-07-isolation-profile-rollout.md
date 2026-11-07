# Implementation Plan: Isolation Profile Rollout (`iso-*` cluster)

## Goal

Close out the 13 `iso-*` handoff items by implementing the remaining stages of the
`standard-v1`/`compatibility-v1` isolation-profile initiative: admission/transaction
lifecycle, the aggregate `IsolationRuntime` port, per-adapter qualification (7
runtimes), conformance coverage, and docs. The domain contract and part of the wire
protocol already exist — this plan is a gap-closing plan against 9 existing
`proposed`-status design docs, not a from-scratch design.

**No code changes in this pass.** This document is the PLAN-phase deliverable; ACT
starts only after review, per the project's phased workflow.

## Current State (verified by reading code, 2026-11-07)

| Area                                                                                                                                                                                                       | State           | Evidence                                                                                                                                 |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Domain contract (`IsolationProfileId`, `IsolationControl`, `ResolvedIsolationProfile`, digest)                                                                                                             | **Done**        | `crates/minibox-domain/src/isolation.rs` (1854 lines), implements `docs/designs/2026-09-21-isolation-profile-contract-design.md` in full |
| Wire protocol: `IsolationProfileRequest`, `IsolationServiceCapabilities`, `IsolationRuntimeCapabilities`, `IsolationDenialCode`/`IsolationDenial`, `IsolationExecutionStatus`, `ExecutionIsolationSummary` | **Done**        | `crates/minibox-core/src/protocol.rs:105-260`                                                                                            |
| `DaemonRequest::Run.isolation: Option<Box<IsolationProfileRequest>>`                                                                                                                                       | **Done**        | `crates/minibox-core/src/protocol.rs:376-382` — already carries intent end-to-end on the request path                                    |
| `DaemonRequest::GetIsolationCapabilities` handler                                                                                                                                                          | **Stub**        | `crates/minibox/src/daemon/server.rs:~819-824` — returns `"not yet implemented"`                                                         |
| `EffectiveIsolationWorkload`, `IsolationAdmissionPolicy`, `IsolationConfig`, binding/typestate sequence, aggregate `IsolationRuntime` port                                                                 | **Not started** | No matching types in `crates/minibox-domain/src/runtime.rs` or `crates/minibox/src/daemon/handler/run/` yet                              |
| Per-adapter qualification (native, colima, gke, krun, smolvm, vz, winbox)                                                                                                                                  | **Not started** | No adapter currently implements `plan()`/`stage()`/`release()`; all 7 should reject `standard-v1` until qualified per design             |
| Conformance suite for isolation                                                                                                                                                                            | **Not started** | No `conformance_isolation*.rs` test file exists                                                                                          |
| Docs (`docs/designs/*isolation*` migration/rollout write-up)                                                                                                                                               | **Not started** | Design docs exist; no user-facing/ops doc yet                                                                                            |

This reconciles the 13 handoff items into 3 tiers: already-satisfied (`iso-profile-protocol`
is effectively done as a sub-slice), foundational work with no adapter dependency
(`iso-profile-admission`, `iso-profile-runtime-port`), and per-adapter work that depends on
the foundation (`iso-native-standard-v1`, `iso-colima-profile`, `iso-gke-profile`,
`iso-krun-profile`, `iso-smolvm-profile`, `iso-vz-profile`, `iso-winbox-profile`), plus two
cross-cutting closeout items (`iso-profile-conformance`, `iso-profile-docs`) and one
selection-layer item (`iso-profile-selection`) that depends on qualification reporting.

## Architecture / Stage Ordering

Stages are ordered by dependency, not by handoff-item alphabetical order:

```
Stage 1: iso-profile-protocol (close-out only — wire plumbing already lands)
Stage 2: iso-profile-admission      (domain: EffectiveIsolationWorkload, IsolationAdmissionPolicy, IsolationConfig)
Stage 3: iso-profile-runtime-port   (domain: IsolationRuntime trait, typestate, binding sequence, persisted state machine)
Stage 4: iso-native-standard-v1     (first real qualifier — reference implementation other adapters mirror)
Stage 5: iso-colima-profile, iso-smolvm-profile, iso-krun-profile, iso-vz-profile   (VM/engine adapters — guest staged-release protocol)
Stage 6: iso-gke-profile, iso-winbox-profile   (substrate-equivalence / Windows-semantic adapters — hardest qualification bar)
Stage 7: iso-profile-selection       (adapter filtering by probed capabilities — needs Stage 4-6 capability reporting)
Stage 8: iso-profile-conformance, iso-profile-docs   (closeout)
```

Each adapter stage is independently assignable to a parallel agent/worktree once Stage 3
lands, per `AGENTS.md`'s worktree-isolation rule for parallel subagents touching adapter code.

---

## Stage 1 — `iso-profile-protocol` (close-out)

**Status:** core plumbing already done. Remaining gap: `GetIsolationCapabilities` handler stub.

- `crates/minibox/src/daemon/server.rs` — replace the stub body (~line 819-824) with a call
  into the (Stage 3) aggregate port's `capabilities()` once it exists. Until Stage 3 lands,
  leave the stub but mark the handoff item `blocked-on: iso-profile-runtime-port` rather than
  `pending`, since completing it meaningfully requires a real `IsolationRuntime` to query.
- No new protocol types needed — `IsolationServiceCapabilities`/`IsolationRuntimeCapabilities`
  already exist as the response shape (`crates/minibox-core/src/protocol.rs:141-164`).

## Stage 2 — `iso-profile-admission`

Design: `docs/designs/2026-09-21-isolation-admission-lifecycle-design.md` (lines 47-116).

Files:

- `crates/minibox-domain/src/isolation.rs` — add `EffectiveIsolationWorkload`,
  `ResolvedWorkloadIdentity`, `IsolationAdmissionPolicy`, `resolve_isolation_mode()`. These
  extend the existing contract file rather than creating a new module, since
  `ResolvedIsolationProfile` and `IsolationDigestDomain` already live there and the new types
  compose directly with them.
- `crates/miniboxd/src/config.rs` — add `IsolationConfig` struct with `#[serde(default)]`,
  wire into `DaemonConfig`. Normalize existing `allow_privileged=true` /
  `MINIBOX_ALLOW_PRIVILEGED=true` inputs to a deterministic `OperatorAuthorizationId` per the
  design's migration note (line 113-115) — this is the one place existing behavior must stay
  working during rollout.
- `crates/minibox/src/daemon/handler/run/request.rs` and `run/preparation.rs` — resolve image
  `USER`/caller overrides/profile defaults into `EffectiveIsolationWorkload` before any
  mutation; reject non-empty host lifecycle hooks under `standard-v1` per design.

Tests: unit tests for `resolve_isolation_mode()` covering the default-deny policy (compat,
capability-add, host-mount, legacy-privileged all denied by default) and the
`PolicyOverride`-can-narrow-not-widen invariant.

## Stage 3 — `iso-profile-runtime-port`

Design: `docs/designs/2026-09-21-isolation-admission-lifecycle-design.md` (lines 117-251).

Files:

- `crates/minibox-domain/src/runtime.rs` — add the `IsolationRuntime` async trait
  (`capabilities`/`plan`/`stage`/`release`/`wait`/`stop`/`cleanup`), `IsolationSubject`,
  `StagedIsolation`, `ValidatedStagedIsolation` (typestate — only `validate_staged_isolation()`
  constructs it), `PartialIsolationAllocation`, `IsolationStageFailure`, `RollbackStatus`.
- `crates/minibox-domain/src/execution_manifest.rs` — add `PrePlanManifestBinding`,
  `ExecutionManifestBinding` carrying plan digest + runtime generation, per the 12-step
  binding sequence (design lines 117-132).
- `crates/minibox/src/daemon/state.rs` — add the persisted state machine
  (`planned -> staged -> enforced -> exited -> cleaned`, plus
  `failed-cleanup-pending -> cleaned` and the parallel `legacy-authorized -> legacy-running`
  track). Illegal transitions fail closed.
- `crates/minibox/src/daemon/handler/run.rs` — route `Run` through the 12-step binding
  sequence instead of directly creating per-container state; the aggregate runtime (not
  individual mutation ports) is the only thing the handler calls.

Tests: compile-fail tests proving unvalidated typestate can't be released (design line 248);
mutation-counter tests proving zero mutation before `plan()` returns; repeated-cleanup
idempotence tests.

**This stage has no adapter dependency and is the longest-lead-time item — start it first.**

## Stage 4 — `iso-native-standard-v1`

Design: `docs/designs/2026-09-21-isolation-adapter-qualification-design.md` (lines 166-same
file's native-specific guidance is implicit — native is the reference adapter since it has
direct kernel access and no guest-protocol indirection).

Files:

- `crates/minibox/src/adapters/native.rs` (or wherever the native `ContainerRuntime` impl
  lives — confirm exact path during ACT; not fully enumerated in the docs read so far) —
  implement `IsolationRuntime` for native: plan all nine controls without degradation, stage
  a blocked subject pre-exec, evidence authority `LocalKernel`.
- Reference for evidence/capability binding: `crates/minibox-domain/src/isolation.rs`'s
  `DOCKER_DEFAULT_V1_CAP_IDS` (14 pinned caps) and `IsolationDigestDomain`.

This is the template the other 6 adapters mirror — write it first among adapters, review it,
then fan out.

## Stage 5 — VM/engine adapters: Colima, SmolVM, krun, VZ

Design: `docs/designs/2026-09-21-isolation-adapter-qualification-design.md` (lines 79-110).

All four share the **guest staged-release protocol** (design lines 147-154): host and guest
bind workload/policy/plan/generation digests, session nonces prevent replay, release is
one-shot, disconnect-before-release triggers cleanup not execution, unknown guest protocol
versions fail closed. Initial behavior for all four is **reject `standard-v1` before VM/engine
mutation** until the guest protocol exists — this is explicit in the design and should be the
first commit for each (a safe no-op qualification that just rejects cleanly), with guest-agent
implementation following.

Files (adapter-qualification trait impl, one per crate):

- `crates/minibox/src/adapters/colima.rs` — maps controls to engine flags, inspects
  created-but-not-started container, evidence = engine + guest-kernel, before start.
- `crates/smolbox/src/lib.rs` (or `crates/minibox/src/adapters/smolvm.rs` — confirm exact
  module during ACT) — guest init/agent receiving plan over authenticated session.
- `crates/macbox/src/krun/runtime.rs` — same guest protocol + krun/hypervisor identity
  evidence.
- `crates/macbox/src/vz/adapter.rs` + `crates/macbox/src/vz/proxy.rs` — host/proxy protocol
  carrying profile version/digests/generation/release token; proxy reconnect cannot rebind
  profile or subject.

Each of these 4 can be a separate parallel worktree once Stage 4's pattern is reviewed — cap at
3 concurrent per `AGENTS.md`/`CLAUDE.md` wave rules, so split as 3 + 1 across two waves.

## Stage 6 — Substrate-equivalence adapters: GKE, Winbox

Design: same file, lines 111-126. Hardest qualification bar — explicitly flagged in the design
as likely-reject-only for an extended period:

- `crates/minibox/src/adapters/gke.rs` — proot cannot manufacture Linux capabilities; explicit
  capability additions reject unless the pod security context + cluster sandbox can prove
  equal/stronger semantics for all nine controls.
- `crates/winbox/src/lib.rs` — requires a _separately versioned_ Windows semantic mapping
  (identity, process/job, filesystem, network, privilege, syscall-equivalent, resource); no
  silent fallback to unsupported Linux capability names. The mapping itself needs its own
  review before `iso-winbox-profile` can even start implementation — flag this to the user as
  a sub-decision when Stage 6 begins.

## Stage 7 — `iso-profile-selection`

Design: adapter-qualification doc, lines 128-145.

Files:

- `crates/minibox-core/src/adapter_registry.rs` — extend `AdapterSuite` selection: discover
  compiled adapters, probe generation-bound capabilities (Stage 1's now-real
  `GetIsolationCapabilities`), filter by profile support, apply operator preference, re-run
  authoritative `plan()` after any fallback. If no adapter qualifies, run admission fails but
  daemon stays available for image/status ops.
- `crates/miniboxd/src/main.rs` — update adapter-suite composition call sites (per
  `AGENTS.md`'s "`HandlerDependencies` changes require updating all adapter suite construction
  sites" rule).

Depends on at least one adapter from Stage 4-6 reporting real (non-reject) capabilities to be
testable end-to-end; can be scaffolded earlier against the native adapter alone.

## Stage 8 — Closeout: `iso-profile-conformance`, `iso-profile-docs`

- New file `crates/minibox/tests/conformance_isolation.rs` — one conformance factory per
  adapter per the qualification design's "Conformance Registration" section (lines 156-165):
  supported/rejected profiles, expected evidence authorities, real-backend test availability,
  mutation probes for rejection tests, cleanup probes. Wire into `CAPABILITY_PRODUCER_TARGETS`
  in `xtask/src/context.rs` (same mechanism just extended for `gh-496`) once there's a real test
  to produce evidence from — do not register a target with an empty proof array again.
- `docs/designs/2026-09-21-isolation-protocol-migration-design.md` — read its full body before
  Stage 1 close-out (not yet done in this investigation); it may specify client-side migration
  steps for `minibox-cli`/`mbx` not captured above.
- New `docs/core/` or `docs/plans/` doc (per `iso-profile-docs`): migration guide covering the
  default-deny `IsolationConfig`, the `allow_privileged` compatibility shim, and per-adapter
  qualification status (which adapters currently reject `standard-v1` and why).
- Version/doc-date markers: if any adapter crate's documented capability matrix changes,
  update `docs/core/FEATURE_MATRIX.mbx.md` and the `fact:workspace_version` markers per
  `AGENTS.md`'s branch-protection section, or `cargo xtask verify`'s docs-audit will fail.

---

## Open Items Before ACT

1. `docs/designs/2026-09-21-isolation-protocol-migration-design.md`'s full body is still
   unread — only front-matter was seen. Read it before starting Stage 1 close-out or Stage 7,
   since it may already specify `minibox-cli`/`mbx` client changes this plan doesn't yet cover.
2. Exact adapter module paths for SmolVM (`smolbox` crate vs. `minibox/src/adapters/smolvm.rs`)
   need confirming against current source before Stage 5 starts — the design doc's `sources:`
   list names both `crates/smolbox/src/lib.rs` and implies a minibox-side adapter file; only
   one is likely the real qualification site.
3. Winbox's Windows-semantic mapping (Stage 6) needs its own review/sign-off before
   implementation — not just an engineering task, a design decision on acceptable Windows
   equivalence.
4. Confirm whether `iso-profile-admission`'s `IsolationConfig` migration note (normalizing
   legacy `allow_privileged`/`MINIBOX_ALLOW_PRIVILEGED`) needs a deprecation window or can land
   as a direct behind-the-scenes normalization — ask the user if any external docs/scripts
   reference those flags directly.

## Non-Goals (explicitly out of scope per source design docs)

- Cryptographic remote attestation (session binding is local structural evidence only).
- Generic protocol/client migration beyond what's listed in Stage 1/8.
- Platform-specific control _mechanics_ beyond what's needed to pass qualification (e.g. this
  plan doesn't re-derive seccomp profile internals — that's tracked separately as `gh-494`).
