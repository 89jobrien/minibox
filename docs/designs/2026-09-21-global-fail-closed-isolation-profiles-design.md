---
source_sha: f9aa9a79348227ca3c517fe7ea86b3bc6eb16829
status: proposed
sources:
  - docs/designs/2026-09-12-guaranteed-security-profiles-design.md
  - docs/designs/2026-09-18-environment-attested-isolation-design.md
  - docs/designs/2026-09-18-isolation-policy-admission-design.md
  - docs/designs/2026-09-18-typed-isolation-policy-contract-design.md
  - crates/minibox-domain/src/runtime.rs
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox-core/src/protocol.rs
  - crates/minibox/src/daemon/handler/run.rs
  - crates/minibox/src/daemon/handler/run/preparation.rs
  - crates/minibox/src/container/process.rs
  - crates/miniboxd/src/adapter_registry.rs
generated: 2026-09-21
---

# Design: Global Fail-Closed Isolation Profiles

## Goal

Make every non-legacy Minibox workload resolve to a versioned isolation profile that the selected
runtime must enforce with complete evidence or reject before per-container mutation.

## Approved Approach

Use a domain-owned isolation profile model with `standard-v1` as the global default, an explicit
operator-authorized `compatibility-v1` escape hatch, and aggregate runtime adapters that plan before
mutation and stage evidence before workload release.

## Approved Constraints

- `standard-v1` is the default for every workload and adapter.
- Linux capabilities begin with the pinned Docker default set and accept `--cap-add` and
  `--cap-drop` modifiers.
- Capability additions require a default-deny operator gate.
- An adapter enforces every required control with evidence or rejects before mutation.
- `compatibility-v1` requires both caller selection and operator authorization.
- Legacy privileged execution remains separate and conflicts with profile controls.
- Profile controls cover identity, process, mount, network, Linux capabilities,
  `no_new_privileges`, syscall filtering, filesystem restrictions, and resources.
- No automatic fallback or warning-only `standard-v1` execution is allowed.

## Decomposition

The contract is intentionally split because its six concerns have different owners and review
criteria. This document is the umbrella decision record; child documents are authoritative for
their assigned concern.

| Order | Child design                                           | Authority                                             |
| ----- | ------------------------------------------------------ | ----------------------------------------------------- |
| 1     | `2026-09-21-isolation-profile-contract-design.md`      | Profile vocabulary, canonical values, and digests     |
| 2     | `2026-09-21-isolation-admission-lifecycle-design.md`   | Admission and plan-stage-validate-release transaction |
| 3     | `2026-09-21-native-standard-v1-enforcement-design.md`  | Native Linux enforcement and evidence                 |
| 4     | `2026-09-21-isolation-adapter-qualification-design.md` | Non-native adapter qualification or rejection         |
| 5     | `2026-09-21-isolation-protocol-migration-design.md`    | Wire protocol, clients, manifests, and migration      |
| 6     | `2026-09-21-isolation-conformance-rollout-design.md`   | Conformance, failure injection, gates, and rollout    |

## Dependency Graph

```text
profile contract
    |
    v
admission and lifecycle
    |--------------------|
    v                    v
native enforcement   adapter qualification
    |                    |
    +---------+----------+
              v
      protocol and migration
              |
              v
      conformance and rollout
```

Protocol primitives may be developed alongside enforcement, but protocol approval depends on the
contract and lifecycle documents. Conformance approval depends on all other child designs.

## Shared Invariants

1. `standard-v1` has no degraded control state.
2. Every compatibility degradation is explicit, control-specific, and bound to operator
   authorization.
3. Planning performs no per-container mutation.
4. Workload code cannot execute before staged evidence validates.
5. Failure retains or durably records cleanup ownership.
6. The selected adapter and environment generation are bound into plan and evidence digests.
7. Unknown profile, manifest, plan, evidence, and protocol versions fail closed.
8. Raw runtime, filesystem, network, and limiter ports remain adapter internals rather than handler
   escape hatches.
9. New clients handshake before sending a workload to prevent old-daemon downgrade.
10. An adapter is never advertised as qualified solely because it compiled successfully.

## Existing Design Relationship

The environment-attested design remains the stated successor of the September 18 typed-policy
documents. This decomposed contract is its local, non-cryptographic implementation slice. The
signed and environment-attested designs remain authoritative for hostile multi-tenant guarantees,
trust roots, signatures, replay protection, remote attestation, and durable authority ledgers.

## Delivery Mapping

| Task                          | Child design                                 |
| ----------------------------- | -------------------------------------------- |
| `iso-profile-contract`        | Profile contract                             |
| `iso-profile-protocol`        | Protocol and migration                       |
| `iso-profile-admission`       | Admission and lifecycle                      |
| `iso-profile-runtime-port`    | Admission and lifecycle                      |
| `gh-492`                      | Native enforcement: capabilities             |
| `gh-493`                      | Native enforcement: identity/user namespaces |
| `gh-494`                      | Native enforcement: seccomp                  |
| `iso-native-standard-v1`      | Native enforcement integration               |
| `iso-*-profile` adapter tasks | Adapter qualification                        |
| `iso-profile-selection`       | Adapter qualification and protocol migration |
| `iso-profile-conformance`     | Conformance and rollout                      |
| `gh-495`                      | Conformance and rollout E2E regressions      |
| `iso-profile-docs`            | Protocol migration and rollout documentation |

## Global Out Of Scope

- Caller-authored profiles or raw capability, BPF, VM, or HCS payloads.
- Cryptographic policy signatures, trust stores, replay prevention, and remote attestation.
- Automatic compatibility fallback.
- Changing the four capabilities currently withheld from legacy privileged mode.
- Implementation code during design approval.

## Approval

The umbrella is approved only when all six child designs are explicitly approved. The next
planning phase must reference child design paths rather than inventing cross-cutting APIs here.
