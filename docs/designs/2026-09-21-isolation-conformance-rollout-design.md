---
source_sha: f9aa9a79348227ca3c517fe7ea86b3bc6eb16829
status: proposed
sources:
  - crates/minibox-testsuite/Cargo.toml
  - crates/minibox-testsuite/src/adapters/runtime.rs
  - crates/minibox/tests/native_adapter_isolation_tests.rs
  - crates/minibox/tests/security_regression.rs
  - crates/minibox-core/tests/protocol_evolution.rs
  - crates/minibox-core/tests/property_roundtrip.rs
  - crates/minibox-core/tests/proptest_roundtrip.rs
  - xtask/src/protocol_drift.rs
  - xtask/src/gates.rs
  - docs/designs/2026-09-21-isolation-profile-contract-design.md
  - docs/designs/2026-09-21-isolation-admission-lifecycle-design.md
  - docs/designs/2026-09-21-native-standard-v1-enforcement-design.md
  - docs/designs/2026-09-21-isolation-adapter-qualification-design.md
  - docs/designs/2026-09-21-isolation-protocol-migration-design.md
generated: 2026-09-21
---

# Design: Isolation Conformance And Rollout

## Goal

Prove the global fail-closed contract through deterministic fixtures, failure injection,
real-backend evidence, and qualification gates before any adapter advertises `standard-v1`.

## Ownership

- **Shared conformance:** `minibox-testsuite`
- **Domain/property fixtures:** `minibox-domain` and `minibox-core`
- **Native security regressions:** `minibox/tests`
- **Workspace gates:** `xtask`
- **Real backend suites:** owning adapter crates and CI/VPS jobs

## Conformance Outcomes

Every adapter/profile case ends in exactly one outcome:

1. **Qualified:** all required controls planned, staged, evidenced, validated, released, and cleaned.
2. **Rejected:** typed rejection before first per-container mutation.
3. **Compatibility authorized:** all controls are enforced or explicitly degraded and bound to an
   operator authorization ID.

Warning-only or partial standard execution is a test failure.

## Shared Adapter Suite

The proposed `crates/minibox-testsuite/src/adapters/isolation.rs` verifies:

- Capability report is generation-bound and advisory.
- `plan()` is deterministic and mutation-free.
- Unsupported profiles reject before mutation.
- Standard plans contain all nine controls and no degradations.
- Compatibility plans partition all nine controls into enforced/degraded sets.
- Stage evidence binds runtime, generation, workload, policy, plan, subject, and artifacts.
- Release consumes validated typestate and targets the same subject.
- Stage, validation, and release failures retain cleanup ownership.
- Stop and cleanup are idempotent.

## Golden Vectors

Version-controlled fixtures pin:

- Profile IDs and control names.
- Docker capability baseline numeric IDs.
- Capability name/number parsing.
- Every digest domain and canonical JSON projection.
- Manifest v1 digest before schema changes.
- Manifest v2 bindings and status transitions.
- Standard and compatibility syscall artifact digests.
- Protocol old/new request and response shapes.

Unknown versions and malformed canonical encodings must fail closed.

## Property Tests

- Map/set insertion order cannot change digests.
- Display-name table changes cannot change capability digest projections.
- Drop-wins capability resolution is deterministic.
- Operator policy overrides are monotonic: they may remove authority but never add it.
- Profile resolution always yields exactly nine controls.
- Standard evidence has exactly nine observations and zero degradations.
- Compatibility enforced/degraded sets are disjoint and complete.
- Cleanup retries converge without losing handles.

## Failure Injection

Inject failure before and after every operation:

- Directory/rootfs allocation.
- Cgroup creation and limit writes.
- Network allocation.
- VM/guest creation.
- Process clone and namespace mapping.
- Mount/pivot.
- Capability application.
- `no_new_privileges` and seccomp installation.
- Evidence readback and persistence.
- Release-latch delivery.
- Stop and cleanup.

Assertions:

- Planning failures perform zero per-container mutation.
- Stage failure either proves rollback complete or persists cleanup ownership.
- Workload code never executes after failed validation.
- Cleanup evidence and digest persist for every attempt.

## Compile-Fail Tests

Use compile-fail fixtures to prove:

- `StagedIsolation` cannot be passed to `release()`.
- Callers cannot construct `ValidatedStagedIsolation`.
- Resolved profiles and evidence cannot be manufactured through public fields.
- Raw handler dependencies cannot access mutation ports after aggregate-port migration.

## Native Linux Qualification

Real-root tests inspect:

- UID/GID maps and namespace inodes.
- Process identity and start generation.
- Effective, permitted, inheritable, ambient, and bounding capabilities.
- `NoNewPrivs` and seccomp mode/program digest.
- Mountinfo and filesystem restrictions.
- Network mode/namespace/configuration.
- Cgroup resource values.
- Capability regain attempts and descendants.

Mocks and source-shape tests supplement but never replace these tests.

## Non-Native Qualification

Each qualified adapter requires a real backend job:

- SmolVM guest-agent evidence.
- krun guest/hypervisor evidence.
- VZ host/proxy/guest binding.
- Colima create-before-start inspection.
- GKE substrate/proot equivalence evidence.
- Winbox approved Windows semantic mapping and evidence.

Until that job passes, the adapter's conformance expectation is typed pre-mutation rejection.

## Protocol And Client Qualification

- New-client/old-daemon handshake rejection.
- Old-client/new-daemon standard enforcement.
- CLI, MCP, Crux, VZ proxy, and pipeline handshake parity.
- Terminal-response table coverage.
- No missing isolation summary after a successful handshake.
- Cleanup failure terminal/status behavior.

## Rollout Phases

1. Land contract/digest fixtures with no runtime support claim.
2. Land admission lifecycle and rejecting aggregate adapters.
3. Qualify native Linux.
4. Enable global standard default where a qualified adapter exists.
5. Keep other adapters fail-closed; optionally permit explicit compatibility.
6. Qualify adapters one backend family at a time.
7. Remove deprecated privileged boolean inputs after the documented window.
8. Evaluate promotion to signed/environment-attested authority.

## Required Commands

- `cargo fmt --all`
- `cargo clippy --workspace -- -D warnings`
- `cargo nextest run --workspace`
- `cargo xtask architecture`
- `cargo xtask verify`
- `taskit protocol drift`
- Linux root/VPS isolation suite
- Real adapter qualification job for any support-claim change

Commands that update protocol locks or golden fixtures run only when the corresponding reviewed
contract intentionally changes; check-mode gates never mutate them.

## Qualification Report

The generated report lists, per adapter:

- Available/compiled state.
- Supported profile versions.
- Qualified controls.
- Evidence authorities.
- Real-backend test revision and timestamp.
- Rejection behavior for unsupported profiles.
- Compatibility degradations.

Doctor and documentation consume this report rather than hand-maintained capability tables.

## Exit Criteria

The umbrella design can move from proposed to approved when all child designs are approved. An
adapter can advertise `standard-v1` only when its conformance and real-backend gates pass. The
global rollout is complete when every selectable adapter either qualifies or deterministically
rejects before mutation.

## Out Of Scope

- Implementation scheduling beyond dependency order.
- Cryptographic attestation validation.
- Performance optimization before correctness gates pass.
