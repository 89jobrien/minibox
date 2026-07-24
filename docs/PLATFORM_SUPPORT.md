# Platform Support

This document summarizes which minibox adapters work on which platforms and what
capabilities each adapter provides. It is written for end users of minibox.

Last updated: 2026-07-24

---

## Quick Reference

| Adapter  | macOS | Linux | Windows | Status       | Use when...                          |
| -------- | :---: | :---: | :-----: | ------------ | ------------------------------------ |
| `smolvm` |  Yes  |  Yes  |   No    | Experimental | Default on all platforms             |
| `krun`   |  Yes  |  Yes  |   No    | Experimental | Fallback when `smolvm` is not on PATH|
| `native` |   No  |  Yes  |   No    | Production   | Linux with root, full isolation      |
| `gke`    |   No  |  Yes  |   No    | Production   | Unprivileged GKE pods                |
| `colima` |  Yes  |  Yes  |   No    | Experimental | Colima/Lima VM environment           |
| `winbox` |   No  |   No  |  (stub) | Stub         | Not yet functional                   |

---

## Adapter Selection

miniboxd selects an adapter automatically at startup:

1. If `MINIBOX_ADAPTER=<name>` is set, that adapter is used with no fallback.
2. Otherwise, `smolvm` is used if the `smolvm` binary is present on PATH.
3. If `smolvm` is absent, the fallback is:
   - `native` on Linux
   - `krun` on macOS

To pin an adapter explicitly:

```sh
MINIBOX_ADAPTER=native miniboxd
MINIBOX_ADAPTER=krun   miniboxd
MINIBOX_ADAPTER=colima miniboxd
```

---

## Prerequisites by Adapter

### `smolvm` (default)

- **Platforms:** macOS, Linux
- **Requirements:**
  - `smolvm` binary on PATH (subsecond-boot lightweight VM manager)
  - No root required on macOS
  - On Linux, root or appropriate capabilities may be required depending on host setup
- **Isolation:** provided by the smolvm VM, not Linux namespaces

### `krun` (macOS fallback)

- **Platforms:** macOS, Linux
- **Requirements:**
  - `libkrun` installed; the `krun` feature must be compiled in
  - No root required on macOS
- **Isolation:** provided by the krun VM (libkrun)

### `native` (Linux production)

- **Platforms:** Linux only (x86_64 and arm64)
- **Requirements:**
  - Root (or `CAP_SYS_ADMIN`, `CAP_NET_ADMIN`) for namespace and cgroup operations
  - cgroups v2 mounted at `/sys/fs/cgroup`
  - Overlay filesystem support in the kernel
- **Isolation:** Linux PID, mount, network, UTS, and IPC namespaces + cgroups v2

### `gke`

- **Platforms:** Linux (GKE pods)
- **Requirements:**
  - No root required; designed for unprivileged pod environments
  - Uses `proot` for filesystem isolation
- **Isolation:** proot (no kernel namespaces or cgroups — not available in unprivileged pods)

### `colima`

- **Platforms:** macOS, Linux (requires Colima + Lima installed)
- **Requirements:**
  - Colima running with a Lima VM (`colima start`)
  - `nerdctl` and `limactl` available
- **Isolation:** provided by the Lima VM

---

## Capability Matrix

Legend:
- **Yes** — implemented and tested
- **No** — not implemented for this adapter
- **Limited** — partially working, known gaps
- **VM** — isolation is provided by the underlying VM, not minibox namespaces
- **Copy** — uses copy-based layer strategy instead of overlay

### Container Lifecycle

| Operation    | smolvm | krun | native | gke | colima |
| ------------ | :----: | :--: | :----: | :-: | :----: |
| pull         |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| run          |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| stop         |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| rm           |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| ps           |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| restart      |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| pause/resume |   No   |  No  |  Yes   |  No |   No   |
| exec (-it)   |   No   |  No  |  Yes   |  No | Limited|
| logs         |   No   |  No  |  Yes   |  No | Limited|
| events       |   No   |  No  |  Yes   | Yes |   No   |

Note: `pause`/`resume` requires cgroups v2 (`native` adapter, Linux only). `exec`
into a running container requires `native` on Linux — not available on VM-based
adapters.

### Image Management

| Operation           | smolvm | krun | native | gke | colima |
| ------------------- | :----: | :--: | :----: | :-: | :----: |
| Docker Hub pull     |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| ghcr.io pull        |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| Parallel layer pull |  Yes   | Yes  |  Yes   | Yes |  Yes   |
| prune / rmi         |   No   |  No  |  Yes   |  No |   No   |
| push (experimental) |   No   |  No  |  Yes   | Yes |  Yes   |
| commit (experimental)|  No   |  No  |  Yes   |  No |  Yes   |
| build (experimental)|  Yes   |  No  |  Yes   |  No |  Yes   |

### Isolation

| Mechanism         | smolvm | krun | native | gke    | colima  |
| ----------------- | :----: | :--: | :----: | :----: | :-----: |
| PID namespace     |   VM   |  VM  |  Yes   |   No   | Lima VM |
| Mount namespace   |   VM   |  VM  |  Yes   |   No   | Lima VM |
| Network namespace |   VM   |  VM  |  Yes   |   No   | Lima VM |
| UTS namespace     |   VM   |  VM  |  Yes   |   No   | Lima VM |
| IPC namespace     |   VM   |  VM  |  Yes   |   No   | Lima VM |
| cgroups v2        |   VM   |  No  |  Yes   |   No   | Lima VM |
| Overlay FS        |   No   |  No  |  Yes   |  Copy  | nerdctl |

### Networking

| Feature             | smolvm | krun | native | gke | colima |
| ------------------- | :----: | :--: | :----: | :-: | :----: |
| Bridge (experimental)|  No   |  No  |  Yes   |  No |   No   |
| Port forwarding     |   No   |  No  |   No   |  No |   No   |
| DNS                 |   No   |  No  |   No   |  No |   No   |

Bridge networking is experimental and Linux-only (`native` adapter).
Port forwarding and DNS are not yet implemented on any adapter.

### Mounts and Privileges

| Feature         | smolvm | krun | native | gke | colima |
| --------------- | :----: | :--: | :----: | :-: | :----: |
| Bind mounts (-v)|   No   |  No  |  Yes   |  No |   No   |
| Privileged mode |   No   |  No  |  Yes   |  No |   No   |

Bind mounts and privileged mode are only available on the `native` Linux adapter
and require root.

---

## Security

All adapters enforce the following protections regardless of platform:

- **Tar path validation** — rejects `..` components and absolute paths in image layers
- **Setuid stripping** — removes setuid/setgid bits when extracting layers
- **Device node rejection** — block and character device entries in layers are rejected
- **Unix socket auth** — the daemon rejects requests from non-root callers via
  `SO_PEERCRED` (Linux/macOS)

The following are available on all adapters except the Windows stub:

- Layer digest verification
- Request frame size limits
- Environment variable redaction in logs (values stored as SHA-256 digests, never plaintext)
- Execution manifest persistence (recorded before container start)
- Admission policy gate (configurable image allowlist, network restrictions,
  privileged gate, memory cap)

---

## Execution Manifest and Policy

Every container run produces an execution manifest persisted to disk before the
process starts. The manifest records the image reference, layer digests, command,
environment (as digests), mounts, resource limits, network mode, and privileged
flag.

You can inspect and verify manifests with the `mbx` CLI:

```sh
mbx manifest <container-id>                # print manifest as JSON
mbx verify   <container-id> --policy <file> # evaluate admission policy (exit 0 = allow)
```

Execution manifests are supported on all adapters that implement `run`.

---

## Observability

Structured tracing and OTLP export are available on all adapters (except `winbox`).

| Feature              | Env variable              | Notes                              |
| -------------------- | ------------------------- | ---------------------------------- |
| OTLP trace export    | `MINIBOX_OTLP_ENDPOINT`   | Requires the `otel` feature        |
| Prometheus metrics   | `MINIBOX_METRICS_ADDR`    | e.g. `0.0.0.0:9090`; `metrics` feature required |

---

## Windows

The `winbox` adapter is a stub. It returns an error for all operations. Named Pipe
server support and HCS/WSL2 wiring have not been implemented. Windows is not a
supported platform in the current release.

---

## Further Reading

- [Architecture reference](ARCHITECTURE.mbx.md) — crate layout, adapter wiring,
  domain traits, protocol overview
- [Feature matrix (developer)](FEATURE_MATRIX.mbx.md) — detailed capability matrix
  with source references
- [Security invariants](SECURITY_INVARIANTS.mbx.md) — security rules that must be
  preserved across changes
- [Verifiable execution](verifiable-execution.mbx.md) — execution manifest format
  and attestation path
