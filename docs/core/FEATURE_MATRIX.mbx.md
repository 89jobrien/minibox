---
source_sha: f75ef70c764b570554053f964cc1ba2deb85cb25
sources:
  - crates/minibox-domain/src/capability_matrix.rs
  - crates/miniboxd/src/adapter_registry.rs
  - crates/miniboxd/src/main.rs
  - crates/minibox/src/daemon/handler
  - crates/minibox/src/adapters/runtime.rs
  - crates/minibox/src/adapters/gke.rs
  - crates/minibox/src/adapters/colima.rs
  - crates/minibox/src/adapters/smolvm.rs
  - crates/minibox/src/adapters/network/bridge.rs
  - crates/minibox/src/container/namespace.rs
  - crates/minibox/src/daemon/server.rs
  - crates/minibox/src/daemon/state.rs
  - crates/minibox-core/src/image/layer.rs
  - crates/minibox-core/src/image/registry.rs
  - crates/macbox/src/lib.rs
  - crates/macbox/src/krun/runtime.rs
  - crates/macbox/src/vz/adapter.rs
  - crates/winbox/src/lib.rs
generated: 2026-09-01
---

# Feature Matrix

Canonical, code-cited capability declarations for minibox adapters.

Last updated: 2026-09-01

## How to read this matrix

The daemon and CLI consume the typed matrix in
`crates/minibox-domain/src/capability_matrix.rs:318-374`. The table below is the
single human-readable copy. Each row citation covers all seven status cells in
that row and points to the exact typed declaration. The support enum and display
labels are defined at `crates/minibox-domain/src/capability_matrix.rs:244-267`.

Backend order is fixed at `crates/minibox-domain/src/capability_matrix.rs:26-36`.
Query the same data without parsing Markdown:

```console
mbx capabilities
mbx capabilities --json
```

### Legend

- **Yes** — declared supported.
- **No** — declared unsupported.
- **Limited** — declared partially supported.
- **VM** / **Lima VM** — provided by the underlying VM.
- **Copy** / **nerdctl** — provided by the named filesystem mechanism.

## Adapter availability

This table intentionally reports source-backed availability and wiring, not an
uncodified production/experimental maturity label.

| Adapter | Availability and wiring | Citation |
| --- | --- | --- |
| `native` | Linux-only registered suite; requires root at startup | `crates/miniboxd/src/adapter_registry.rs:95-100`; `crates/miniboxd/src/main.rs:437-455` |
| `gke` | Linux-only registered suite | `crates/miniboxd/src/adapter_registry.rs:101-106`; `crates/miniboxd/src/main.rs:732-740` |
| `colima` | Unix registered suite | `crates/miniboxd/src/adapter_registry.rs:107-112`; `crates/miniboxd/src/main.rs:741-747` |
| `smolvm` | Unix registered suite and default when its binary is available | `crates/miniboxd/src/adapter_registry.rs:113-118`; `crates/miniboxd/src/adapter_registry.rs:227-261` |
| `krun` | Registered suite and fallback when `smolvm` is unavailable | `crates/miniboxd/src/adapter_registry.rs:119-124`; `crates/miniboxd/src/adapter_registry.rs:217-249` |
| `vz` | macOS + `vz` feature; separately dispatched before the normal adapter builder | `crates/miniboxd/src/adapter_registry.rs:125-130`; `crates/miniboxd/src/main.rs:79-90` |
| `winbox` | Windows daemon stub; startup returns an error | `crates/winbox/src/lib.rs:1-9`; `crates/winbox/src/lib.rs:42-55` |

## Capability status

<!-- capability-status-matrix -->
| Group | Feature | native | gke | colima | smolvm | krun | vz | winbox | Citation |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Container lifecycle | pull | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:329` |
| Container lifecycle | run | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:330` |
| Container lifecycle | stop | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:331` |
| Container lifecycle | rm | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:332` |
| Container lifecycle | ps | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:333` |
| Container lifecycle | pause/resume | Yes | No | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:334` |
| Container lifecycle | restart | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:335` |
| Container lifecycle | exec (-it) | Yes | No | Limited | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:336` |
| Container lifecycle | logs | Yes | No | Limited | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:337` |
| Container lifecycle | events | Yes | Yes | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:338` |
| Image management | Docker Hub v2 | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:339` |
| Image management | ghcr.io | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:340` |
| Image management | Parallel layer pull | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:341` |
| Image management | prune / rmi | Yes | No | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:342` |
| Image management | push (exp) | Yes | Yes | Yes | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:343` |
| Image management | commit (exp) | Yes | No | Yes | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:344` |
| Image management | build (exp) | Yes | No | Yes | Yes | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:345` |
| Isolation | PID namespace | Yes | No | Lima VM | VM | VM | VM | No | `crates/minibox-domain/src/capability_matrix.rs:346` |
| Isolation | Mount namespace | Yes | No | Lima VM | VM | VM | VM | No | `crates/minibox-domain/src/capability_matrix.rs:347` |
| Isolation | Network namespace | Yes | No | Lima VM | VM | VM | VM | No | `crates/minibox-domain/src/capability_matrix.rs:348` |
| Isolation | UTS namespace | Yes | No | Lima VM | VM | VM | VM | No | `crates/minibox-domain/src/capability_matrix.rs:349` |
| Isolation | IPC namespace | Yes | No | Lima VM | VM | VM | VM | No | `crates/minibox-domain/src/capability_matrix.rs:350` |
| Isolation | cgroups v2 | Yes | No | Lima VM | VM | No | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:351` |
| Isolation | Overlay FS | Yes | Copy | nerdctl | No | No | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:352` |
| Networking | Bridge (exp) | Yes | No | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:353` |
| Networking | Port forwarding | No | No | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:354` |
| Networking | DNS | No | No | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:355` |
| Mounts & privileges | Bind mounts (-v) | Yes | No | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:356` |
| Mounts & privileges | Privileged mode | Yes | No | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:357` |
| Security | SO_PEERCRED auth | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:358` |
| Security | Tar path validation | Yes | Yes | Yes | Yes | Yes | Yes | Yes | `crates/minibox-domain/src/capability_matrix.rs:359` |
| Security | Setuid stripping | Yes | Yes | Yes | Yes | Yes | Yes | Yes | `crates/minibox-domain/src/capability_matrix.rs:360` |
| Security | Device node rejection | Yes | Yes | Yes | Yes | Yes | Yes | Yes | `crates/minibox-domain/src/capability_matrix.rs:361` |
| Security | Layer digest verify | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:362` |
| Security | Request frame limits | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:363` |
| Security | Env redaction in logs | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:364` |
| Execution integrity | Execution manifest | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:365` |
| Execution integrity | manifest get/verify | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:366` |
| Execution integrity | Admission policy gate | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:367` |
| State persistence | Records survive restart | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:368` |
| State persistence | PID reconciliation | Yes | No | No | No | No | No | No | `crates/minibox-domain/src/capability_matrix.rs:369` |
| Observability | Structured tracing | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:370` |
| Observability | OTLP export (opt-in) | Yes | Yes | Yes | Yes | Yes | Yes | No | `crates/minibox-domain/src/capability_matrix.rs:371` |
<!-- /capability-status-matrix -->

## Implementation evidence

The row citations above identify the exact status declarations. These additional
sites show the principal implementation and absence evidence behind them:

| Area | Evidence |
| --- | --- |
| Adapter-specific optional exec, push, commit, and build wiring | `crates/miniboxd/src/main.rs:876-910`; `crates/miniboxd/src/main.rs:944-976`; `crates/macbox/src/lib.rs:99-135`; `crates/miniboxd/src/main.rs:1064-1096`; `crates/miniboxd/src/main.rs:1121-1156`; `crates/macbox/src/lib.rs:554-593` |
| Native namespaces | `crates/minibox/src/adapters/runtime.rs:127-176`; `crates/minibox/src/container/namespace.rs:40-72` |
| GKE no-op cgroups, copy filesystem, and proot isolation limits | `crates/minibox/src/adapters/gke.rs:46-77`; `crates/minibox/src/adapters/gke.rs:83-161`; `crates/minibox/src/adapters/gke.rs:287-397` |
| Colima VM namespaces and output behavior | `crates/minibox/src/adapters/colima.rs:867-892`; `crates/minibox/src/adapters/colima.rs:925-1007` |
| SmolVM runtime, VM filesystem, and limiter delegation | `crates/minibox/src/adapters/smolvm.rs:435-562`; `crates/minibox/src/adapters/smolvm.rs:568-667` |
| krun runtime isolation declarations and process spawn | `crates/macbox/src/krun/runtime.rs:335-420` |
| VZ runtime and in-VM filesystem/cgroup delegation | `crates/macbox/src/vz/adapter.rs:152-217`; `crates/macbox/src/vz/adapter.rs:223-344` |
| Native bridge and DNAT implementation | `crates/minibox/src/adapters/network/bridge.rs:82-164`; `crates/minibox/src/adapters/network/bridge.rs:167-221` |
| Tar path, device-node, and permission handling | `crates/minibox-core/src/image/layer.rs:200-230`; `crates/minibox-core/src/image/layer.rs:256-304`; `crates/minibox-core/src/image/layer.rs:319-390` |
| Layer digest verification during pull | `crates/minibox-core/src/image/registry.rs:240-318` |
| Request-size and peer-credential guards | `crates/minibox/src/daemon/server.rs:21`; `crates/minibox/src/daemon/server.rs:132-139`; `crates/minibox/src/daemon/server.rs:190-204` |
| State load and startup reconciliation | `crates/minibox/src/daemon/state.rs:362-424`; `crates/minibox/src/daemon/state.rs:426-461` |
| Windows unsupported status | `crates/winbox/src/lib.rs:1-15`; `crates/winbox/src/lib.rs:42-55` |
