---
status: future
note: Phase 1 (exec, logs, named containers) not started
---

# Maestro-Minibox Integration Opportunities

1. Minibox as a ContainerProvider in maestro-cli (Most Natural)

Maestro already has a ContainerProvider trait abstraction with Docker and Kubernetes implementations. Minibox could be a third provider — a lightweight Linux-native option that doesn't require Docker daemon.

Where: maestro-cli/src/providers/ — implement ContainerProvider backed by minibox's Unix socket API.

What it gets you: Sessions on Linux without Docker Desktop or Docker Engine installed. Useful for CI, constrained VMs, or bare metal.

Blockers today (minibox limitations that maestro depends on):

### Gap Analysis: Minibox vs. What Maestro Needs

```text
| Feature                          | Minibox today           | Gap                                                |
| -------------------------------- | ----------------------- | -------------------------------------------------- |
| `exists` / `is_running` / `list` | `DaemonRequest::List` ✓ | Trivial — filter list by name                      |
| `stop` / `cleanup`               | `Stop` + `Remove` ✓     | Named container lookup needed                      |
| `create_container`               | `Run` ✓ (partially)     | No named containers, no `ContainerName` field      |
| `architecture`                   | Not in protocol         | Easy — read `/proc/cpuinfo` or `uname`             |
| `logs`                           | **Missing**             | Stdout/stderr are discarded post-`execvp`          |
| **`exec`**                       | **Missing**             | Core blocker — needs `setns(2)` + output streaming |
| **TTY/stdio**                    | **Missing**             | Core blocker — needs PTY allocation                |
| **Networking**                   | **Missing**             | Hard blocker for proxy-based terminal              |
```

Full provider support requires closing those gaps first.

---

2. Minibox as the runtime inside maestro-session pods (K8s path)

Instead of running containers via Docker inside the pod, maestro-session's maestro-runtime init could use minibox to launch sub-processes with stronger namespace/cgroup isolation — turning each Claude Code session into a miniboxed workload.

Where: maestro-runtime/src/supervisor.rs + middleware lifecycle — replace raw subprocess spawn with minibox RunContainer.

Value: Per-session cgroup limits (memory.max, cpu.weight), filesystem isolation via overlay, without needing a Docker daemon sidecar.

Blockers: Same TTY/network gaps, plus minibox daemon would need to run inside the K8s pod (requires privileged pod or user namespace support).

---

3. Shared OCI image-pulling library (Low-hanging fruit)

Minibox has a clean Docker Hub v2 client in minibox-lib/src/image/registry.rs. Maestro currently delegates image pulls to Docker daemon. On a minibox provider path, you'd reuse minibox's registry client directly.

- This is the most immediately shippable piece — it's just a library dependency.

---

4. MINIBOX_ADAPTER=gke as the bridge

Minibox's gke adapter (proot, copy FS, no-op limiter, unprivileged) was designed for Kubernetes-like environments. This is closest to what maestro runs on GKE. The gke adapter could be the first target for maestro-runtime integration since it doesn't require root.
