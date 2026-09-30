---
source_sha: f9aa9a79348227ca3c517fe7ea86b3bc6eb16829
status: proposed
sources:
  - crates/minibox/src/adapters/gke.rs
  - crates/minibox/src/adapters/colima.rs
  - crates/minibox/src/adapters/smolvm.rs
  - crates/miniboxd/src/adapter_registry.rs
  - crates/miniboxd/src/main.rs
  - crates/macbox/src/krun/runtime.rs
  - crates/macbox/src/vz/adapter.rs
  - crates/macbox/src/vz/proxy.rs
  - crates/macbox/src/lib.rs
  - crates/smolbox/src/lib.rs
  - crates/winbox/src/lib.rs
  - docs/designs/2026-09-21-isolation-profile-contract-design.md
  - docs/designs/2026-09-21-isolation-admission-lifecycle-design.md
generated: 2026-09-21
---

# Design: Isolation Adapter Qualification

## Goal

Define how every selectable runtime proves complete `standard-v1` equivalence or rejects before its
first per-workload mutation.

## Ownership

- `minibox`: native, SmolVM, Colima, and GKE aggregates.
- `macbox`: krun and VZ aggregates.
- `winbox`: Windows equivalent or explicit rejection.
- `miniboxd`: profile-aware selection and composition.
- `smolbox`: facade only; no independent enforcement authority.

## Capability Reporting

`IsolationRuntimeCapabilities` is advisory and generation-bound. It reports:

- Adapter ID and implementation version.
- Environment-generation digest.
- Supported profile versions.
- Supported controls.
- Accepted evidence authorities.
- Linux `cap_last_cap` where applicable.

`plan()` remains authoritative for the exact workload. A capabilities report never grants support
by itself.

## Qualification Rule

An adapter qualifies `standard-v1` only when:

1. It can plan all nine controls without degradation.
2. It stages a blocked subject before workload execution.
3. It returns profile-approved evidence authority for every control.
4. Evidence binds workload, profile, policy, plan, runtime generation, and subject.
5. Release targets the exact staged subject.
6. Failure retains cleanup ownership.
7. Real backend tests pass; mocks alone are insufficient.

Otherwise `plan()` returns `UnsupportedProfile` or `UnsupportedControl` before mutation.

## Evidence Authorities

| Authority                         | Accepted Use                                                                |
| --------------------------------- | --------------------------------------------------------------------------- |
| `LocalKernel`                     | Native kernel readback                                                      |
| `GuestAgent`                      | Guest-kernel readback bound to host session                                 |
| `Hypervisor`                      | VM identity, launch, and boundary evidence                                  |
| `OuterSandbox`                    | Container-engine or cluster substrate evidence when semantically equivalent |
| `OperatorAuthorizedCompatibility` | Compatibility degradation only; never standard qualification                |

The profile owns the authority matrix. An adapter cannot qualify itself by advertising a weaker
authority.

## Adapter Requirements

### SmolVM

Initial behavior: reject `standard-v1` before VM launch.

Qualification requires a guest init/agent that receives the plan over an authenticated session,
applies guest controls, stages the command, returns guest evidence, and waits for release. Host
evidence binds VM image, VM instance, guest agent version, and session nonce.

### krun

Initial behavior: reject before VM launch.

Qualification requires the same guest staged-release protocol as SmolVM plus krun/hypervisor
identity evidence. Host-only process isolation is insufficient to assert guest capability or seccomp
state.

### VZ

Initial behavior: reject before VM launch.

The host/proxy protocol carries profile version, workload/plan digests, runtime generation, and
release token. The VZ guest agent returns control evidence and subject ID. Proxy reconnect cannot
change the bound profile or subject.

### Colima

Initial behavior: reject before engine mutation.

Qualification maps controls to exact engine flags, inspects the created-but-not-started container,
and returns engine plus guest-kernel evidence before start. Unsupported engine versions reject;
command-line acceptance alone is not evidence.

### GKE/proot

Initial behavior: reject before copy, pod, or proot mutation.

Qualification requires evidence that the pod security context and outer cluster sandbox provide
equal or stronger semantics for all nine controls. Proot cannot manufacture Linux capabilities;
explicit capability additions therefore reject unless the substrate can provide and prove them.

### Winbox

Initial behavior: reject `standard-v1`.

Qualification requires a separately versioned Windows semantic mapping for identity, process/job,
filesystem, network, privilege, syscall-equivalent, and resource controls. Linux capability names
remain unsupported rather than silently ignored. The mapping must be approved before Winbox can
advertise the profile.

## Profile-Aware Selection

Adapter selection separates availability from qualification:

1. Discover compiled/available adapters.
2. Probe generation-bound profile capabilities.
3. Filter adapters that cannot plan the requested profile.
4. Apply operator preference among qualified adapters.
5. Re-run authoritative planning after every fallback decision.

If no adapter qualifies, run admission fails. The daemon may remain available for image/status
operations.

## Compatibility Mode

An operator-authorized adapter may run `compatibility-v1` if it records each of the nine controls as
enforced or degraded. Explicit capability add/drop requests still require exact enforcement.
Compatibility does not qualify the adapter for standard selection.

## Guest Protocol Invariants

- Host and guest bind the same workload, policy, plan, and environment-generation digests.
- Session nonces prevent evidence replay.
- Guest evidence identifies agent version and boot/VM generation.
- Release is one-shot and subject-specific.
- Disconnect before release triggers cleanup, not workload execution.
- Unknown guest protocol versions fail closed.

## Conformance Registration

Each adapter registers one conformance factory with:

- Supported/rejected profiles.
- Expected evidence authorities.
- Real-backend test availability.
- Mutation probes for rejection tests.
- Cleanup probes.

## Out Of Scope

- Native Linux control mechanics.
- Generic protocol/client migration.
- Cryptographic remote attestation; session binding here is local structural evidence.
