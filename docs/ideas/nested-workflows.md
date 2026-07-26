---
title: nested-workflows-idea
doctype: idea
project: minibox
status: draft
created: 2026-05-29
updated: 2026-05-29
---

# Nested Container and Image Building Workflows

Future roadmap for nested container execution and image building in
minibox. This document captures architecture, gaps, and priorities for
six scenarios. All items are post-stabilization -- the v0.30.0
freeze is active and no implementation should begin without explicit
scope-change approval.

Current non-goals that overlap with this design: rootless/user-namespace
support, Kubernetes integration. Promoting these to goals is a
prerequisite for scenarios 1-4.

---

## Capability inventory (what exists today)

| Capability                     | Status    | Location                              |
| ------------------------------ | --------- | ------------------------------------- |
| PID/mount/UTS/IPC/net NS       | Yes       | `container/namespace.rs`              |
| User namespace (CLONE_NEWUSER) | **No**    | Flag exists in BackendCapability only |
| cgroup v2 resource limits      | Yes       | `adapters/limiter.rs`                 |
| cgroup delegation              | **No**    | --                                    |
| Overlay filesystem             | Yes       | `adapters/filesystem.rs`              |
| Nested overlay (overlay-on-overlay) | **No** | Kernel 5.11+ supports it            |
| Image build (Dockerfile)       | Partial   | `adapters/builder.rs` (FROM/RUN/ENV/CMD) |
| Container commit               | Yes       | `adapters/commit.rs`                  |
| Image push                     | Yes       | domain trait, native/colima adapters  |
| Bridge networking              | Yes       | `adapters/network/bridge.rs`          |
| Port forwarding                | **No**    | --                                    |
| Bind mounts                    | Yes       | native only                           |
| Privileged mode                | Yes       | native only                           |
| /proc and /sys masking         | Partial   | procfs mounted, no sysfs emulation    |
| Seccomp BPF                    | **No**    | --                                    |
| ID-mapped mounts               | **No**    | Kernel 5.12+ required                 |

---

## Scenario 1: DinM (Docker-in-Minibox)

**Goal:** Run `dockerd` inside a minibox container without
`--privileged`.

### What minibox already supports

- Namespace isolation (PID/mount/UTS/IPC/net).
- Overlay filesystem for the outer container rootfs.
- Bind mounts for providing `/var/lib/docker` storage.

### What's missing

1. **User namespaces.** Docker inside the container needs to believe
   it is root. Today minibox containers run as actual host root.
   Without user namespace remapping, inner dockerd has real root
   privileges on escape -- the entire security model collapses.

2. **Cgroup delegation.** Inner dockerd needs to create child cgroups
   for its containers. Minibox must mark the container's cgroup
   subtree as delegated (`cgroup.subtree_control` + ownership
   transfer to the mapped UID). Without this, inner `docker run`
   fails with EPERM on cgroup writes.

3. **Nested overlay support.** Inner Docker uses overlayfs for its
   containers. Running overlay-on-overlay requires kernel 5.11+ and
   the `metacopy=on` mount option. If the outer overlay does not
   support nesting, inner Docker falls back to vfs (slow, copies
   entire layers).

4. **Writable /proc and /sys.** Inner dockerd mounts procfs and sysfs
   inside its containers. Minibox currently mounts `/proc` read-only
   in the outer container. The inner runtime needs a writable procfs
   within its PID namespace. Minibox must either:
   - Mount a fresh procfs per container (already done).
   - Allow the inner PID namespace to mount its own proc (requires
     mount namespace isolation, which exists, but proc mount options
     need review).

5. **Syscall policy (seccomp).** Without seccomp, inner dockerd has
   access to `clone()`, `mount()`, `pivot_root()` etc. This is
   acceptable if user namespaces scope them, but without userns it
   is a privilege escalation vector.

6. **Dedicated storage per instance.** Each inner dockerd needs its
   own `/var/lib/docker`. Sharing causes corruption. Minibox must
   either auto-provision per-container volumes or enforce the
   constraint via documentation.

### Architectural changes

| Change                        | Crate        | Scope                              |
| ----------------------------- | ------------ | ---------------------------------- |
| Add `new_user` to NamespaceConfig | minibox  | `container/namespace.rs`           |
| UID/GID mapping (newuidmap)   | minibox      | New module `container/userns.rs`   |
| Cgroup delegation logic       | minibox      | `adapters/limiter.rs`              |
| Nested overlay mount options  | minibox      | `adapters/filesystem.rs`           |
| Seccomp BPF profile           | minibox-core | New domain trait + native adapter  |

### Security implications

- User namespace remapping is the single most important security
  control. Without it, DinM is equivalent to `--privileged` Docker.
- UID range allocation: shared ranges (sysbox-CE model) allow
  cross-container file visibility on escape. Exclusive ranges
  (sysbox-EE model) are safer but consume 65536 UIDs per container.
  Minibox should default to exclusive ranges.
- Seccomp profile for the outer container should allowlist only the
  syscalls inner dockerd needs (clone, mount, pivot_root, unshare,
  setns). Block everything else.

### Priority: **Medium**

Useful for CI pipelines that need Docker. But most users can run
Docker alongside minibox rather than inside it. The main value is
CI isolation (each job gets a fresh Docker instance).

---

## Scenario 2: MinM (Minibox-in-Minibox)

**Goal:** Run `miniboxd` inside a minibox container.

### What minibox already supports

- Everything DinM needs, minus user namespaces and cgroup delegation.
- Unix socket communication (inner miniboxd uses its own socket).

### What's missing (beyond DinM requirements)

1. **Everything from DinM** -- user namespaces, cgroup delegation,
   nested overlay, seccomp.

2. **SO_PEERCRED in nested namespaces.** Inner miniboxd uses
   `SO_PEERCRED` to verify the connecting process is root. Inside a
   user namespace, the inner root (UID 0 in the container) maps to
   an unprivileged host UID. `SO_PEERCRED` returns the *namespace*
   UID (0), which is correct for the inner daemon's perspective.
   This should work without changes, but needs integration testing.

3. **Recursive overlay depth.** Each nesting level adds an overlay
   stack. Linux imposes no hard limit on overlay depth, but
   performance degrades. Minibox should cap nesting depth (e.g., 3
   levels) and report the current depth in container metadata.

### Architectural changes

Same as DinM, plus:

| Change                        | Crate        | Scope                       |
| ----------------------------- | ------------ | --------------------------- |
| Nesting depth tracking        | minibox-core | `protocol.rs` (metadata)    |
| Integration test: nested miniboxd | minibox  | New test under `tests/`     |

### Security implications

Same as DinM. The inner miniboxd is no more dangerous than inner
dockerd -- it performs the same namespace/cgroup operations.

### Priority: **Low**

Niche use case. Primarily useful for testing minibox itself.
Real-world users would not nest minibox instances.

---

## Scenario 3: KinM (Kubernetes-in-Minibox)

**Goal:** Run a multi-node K8s cluster where each node is a minibox
container.

### What minibox already supports

- Container-as-compute-unit model (same as sysbox KinD pattern).
- Bridge networking between containers.
- Image pulling from Docker Hub / GHCR.

### What's missing

1. **Everything from DinM** -- kubelet inside each node-container
   needs to run inner containers (via containerd or CRI-O).

2. **Inter-container DNS.** K8s nodes need to resolve each other by
   name. Minibox has no DNS server. Options:
   - Inject `/etc/hosts` entries at container creation (simplest).
   - Run a lightweight DNS forwarder on the bridge network.
   - Delegate to CoreDNS inside the cluster (requires working
     networking first).

3. **Port forwarding.** `kubectl port-forward` and NodePort services
   require the host to forward ports into the bridge network.
   Minibox has bridge networking but no port forwarding (iptables
   DNAT rules).

4. **Systemd or init inside containers.** K8s node images typically
   run systemd for kubelet lifecycle management. Minibox containers
   currently exec a single process. Supporting systemd requires:
   - Mount tmpfs at `/run` and `/run/lock`.
   - Mount cgroup filesystem inside the container.
   - Set container PID 1 to systemd (already possible via command).
   - Ensure the container's cgroup subtree is delegated.

5. **Image preloading.** A 3-node cluster pulls ~2GB of K8s images
   per node. Without preloading (baking images into the node
   container image), cluster startup takes 10+ minutes. This
   requires `minibox build` to support running an inner container
   runtime during build (circular dependency with DinM).

6. **Cluster lifecycle tooling.** A `kindbox`-equivalent CLI wrapper
   that automates: create N node containers, init kubeadm on master,
   join workers, install CNI. This is a separate binary or xtask,
   not core minibox.

### Architectural changes

Everything from DinM, plus:

| Change                     | Crate    | Scope                           |
| -------------------------- | -------- | ------------------------------- |
| Port forwarding (DNAT)     | minibox  | `adapters/network/bridge.rs`    |
| DNS injection or forwarder | minibox  | `container/process.rs` or new   |
| Systemd container support  | minibox  | `container/filesystem.rs`       |
| kindbox-style CLI          | new bin  | Separate crate or xtask         |

### Security implications

- Each K8s node-container runs kubelet + containerd, both of which
  need namespace/cgroup privileges. Same security model as DinM.
- The cluster's pod network (flannel/calico) operates inside the
  bridge network. Pod-to-pod traffic is isolated from the host by
  the bridge + network namespace boundary.
- API server access from the host via `kubectl` requires port
  forwarding or socket proxy.

### Priority: **Low**

Significant engineering effort for a niche use case. KinD (with
Docker) already serves this need well. Minibox would need to match
KinD's developer experience to be competitive. Worth designing but
not worth implementing until DinM is solid.

---

## Scenario 4: DinD via Minibox

**Goal:** Minibox replaces Docker as the outer runtime for
Docker-in-Docker CI workflows.

### What minibox already supports

- Container lifecycle (run/stop/rm/ps).
- Image pulling.
- Bind mounts (for CI workspace sharing).
- Execution manifests (audit trail for CI).

### What's missing

1. **Everything from DinM** -- the inner Docker needs the same
   namespace/cgroup support.

2. **Docker API compatibility.** CI tools (GitHub Actions runners,
   GitLab CI, Jenkins) assume Docker socket at
   `/var/run/docker.sock`. Two options:
   - **Full compatibility:** Implement a Docker API shim that
     translates Docker API calls to minibox daemon requests. Major
     effort, questionable value.
   - **Minibox-native CI:** Provide a minibox-aware CI runner
     configuration. Users configure CI to use `mbx` instead of
     `docker`. Requires CI platform support or wrapper scripts.

3. **`--mount type=volume` support.** CI workflows use Docker
   volumes for caching (node_modules, cargo target, pip cache).
   Minibox has bind mounts but no managed volume abstraction.

4. **Container labels and metadata.** CI runners attach labels to
   containers for identification and cleanup. Minibox containers
   have IDs and image refs but no arbitrary label support.

### Architectural changes

| Change                      | Crate        | Scope                         |
| --------------------------- | ------------ | ----------------------------- |
| Volume management           | minibox-core | New domain trait `VolumeManager` |
| Container labels            | minibox-core | `protocol.rs` (Run request)   |
| Docker API shim (optional)  | new crate    | Compatibility layer           |

### Security implications

- Minibox's execution manifest provides better audit trails than
  Docker for CI. Every container run produces a sealed manifest
  with SHA-256 digests of all inputs.
- Admission policy can gate CI containers: restrict images, network
  modes, mount paths.
- Without the Docker API shim, CI tools cannot accidentally use
  `--privileged` or `--net=host` -- they must go through minibox's
  API which enforces policy.

### Priority: **Medium-High**

This is the most practical scenario for minibox's target users (CI
and developer workflows). The execution manifest + admission policy
give minibox a real advantage over Docker for CI. But it requires
DinM support first, which requires user namespaces.

Incremental path: support minibox-native CI first (no inner Docker),
then add DinM for workflows that require Docker.

---

## Scenario 5: Image Building (`minibox build`)

**Goal:** Build OCI images from Dockerfiles/Containerfiles.

### What minibox already supports

- `MiniboxImageBuilder` in `adapters/builder.rs`:
  - `FROM` (pulls base image if missing).
  - `RUN` (spawns ephemeral container, commits overlay diff as layer).
  - `ENV`, `CMD` (accumulated as metadata).
  - Output streaming during `RUN` steps.
- `ContainerCommitter` for snapshotting writable layers.
- `ImagePusher` for pushing to registries.

### What's missing

1. **Dockerfile instructions.** Currently unimplemented (warned and
   skipped):
   - `COPY` / `ADD` -- copy files from build context into the image.
     Requires copying files into the overlay upper dir before
     committing the layer. `ADD` also supports URL fetching and tar
     auto-extraction.
   - `WORKDIR` -- set the working directory for subsequent
     instructions. Needs to create the directory in the overlay and
     track it as state for `RUN`/`COPY`.
   - `ENTRYPOINT` -- set the default entrypoint. Metadata-only,
     similar to `CMD`.
   - `ARG` -- build-time variables. Needs variable substitution in
     subsequent instructions.
   - `EXPOSE` -- metadata-only (port documentation).
   - `LABEL` -- metadata-only (key-value pairs in image config).
   - `USER` -- switch the UID for subsequent `RUN` steps. Requires
     passing the UID to `spawn_process`.

2. **Multi-stage builds.** `FROM ... AS builder` followed by
   `COPY --from=builder`. The current parser handles `FROM` but not
   `AS` aliases or cross-stage `COPY`. Requires:
   - Tracking named stages and their final layer stacks.
   - `COPY --from=<stage>` reads from a previous stage's overlay.

3. **Build cache.** Each `RUN` step creates a new layer. Rebuilding
   the same Dockerfile re-runs every step. Docker uses content-
   addressed cache keying (instruction hash + parent layer hash).
   Minibox should:
   - Hash each instruction + its parent layer digest.
   - Check the image store for a matching cached layer.
   - Skip the `RUN` step if a cache hit exists.

4. **`.dockerignore` support.** The build context should respect
   `.dockerignore` to exclude files from `COPY`/`ADD`.

5. **BuildKit-style features.** Out of scope for initial
   implementation but worth noting:
   - `RUN --mount=type=cache` (persistent build caches).
   - `RUN --mount=type=secret` (secret injection without layer
     leakage).
   - Parallel stage execution.

### Architectural changes

| Change                     | Crate        | Scope                         |
| -------------------------- | ------------ | ----------------------------- |
| COPY/ADD instruction impl  | minibox      | `adapters/builder.rs`         |
| WORKDIR/ENTRYPOINT/ARG/USER/LABEL/EXPOSE | minibox | `adapters/builder.rs` |
| Multi-stage build tracking | minibox      | `adapters/builder.rs`         |
| Build cache (instruction hash) | minibox  | `adapters/builder.rs` + `image/` |
| .dockerignore parsing      | minibox-core | New module `image/ignore.rs`  |
| Dockerfile parser updates  | minibox      | `image/dockerfile.rs`         |

### Security implications

- `COPY`/`ADD` must validate paths (no `..` traversal, no absolute
  paths escaping the context directory). Reuse
  `validate_layer_path()`.
- `ADD` with URL fetching: must validate the URL scheme (http/https
  only), enforce size limits, and reject redirects to local
  addresses (SSRF).
- `ARG` values should not leak into layer metadata or execution
  manifests. Docker historically exposed build args in image
  history -- minibox should not repeat this.
- `RUN --mount=type=secret` (future): secrets must never be written
  to a layer. They should be bind-mounted into the ephemeral
  container and excluded from the commit.

### Priority: **High**

Image building is the most requested missing feature and the most
immediately useful. It does not require user namespaces or cgroup
delegation -- the current privileged-root execution model works
fine for builds. The existing `MiniboxImageBuilder` provides a
working foundation; the remaining work is Dockerfile instruction
coverage and build cache.

### Incremental implementation order

1. `COPY` (most common missing instruction).
2. `WORKDIR` (required by many Dockerfiles).
3. `ENTRYPOINT`, `LABEL`, `EXPOSE`, `USER` (metadata, low effort).
4. `ARG` (variable substitution adds complexity).
5. `ADD` (URL fetching + tar extraction).
6. `.dockerignore`.
7. Multi-stage builds.
8. Build cache.

---

## Scenario 6: MinD (Minibox-in-Docker)

**Goal:** Run `miniboxd` inside a Docker container for CI
environments where Docker is the available runtime.

### What minibox already supports

- `miniboxd` is a single static binary (musl target).
- Unix socket communication.
- The daemon auto-detects its environment via adapter selection
  (`MINIBOX_ADAPTER` env var or auto-detection in
  `adapter_registry.rs`).

### What's missing

1. **GKE adapter generalization.** The `gke` adapter already runs
   minibox inside unprivileged containers (GKE pods) using proot
   for filesystem isolation and no-op cgroup/network. This is
   conceptually identical to running inside a Docker container.
   The adapter should be renamed or generalized to
   `unprivileged-container` or `proot`.

2. **Proot availability.** The `gke` adapter depends on `proot`
   being in PATH. For MinD, the Docker image must include proot.
   This is a packaging concern, not a code change.

3. **Nested namespace support (optional).** If the Docker container
   is run with `--privileged` or sufficient capabilities
   (`SYS_ADMIN`, `NET_ADMIN`), minibox could use the native adapter
   with real namespace isolation. The adapter selection logic should
   detect available capabilities and choose accordingly:
   - Has `CAP_SYS_ADMIN` + cgroup access -> `native`.
   - No capabilities -> `gke`/`proot` fallback.

4. **Container image.** Need a `Dockerfile` that produces a minimal
   image with `miniboxd`, `mbx`, and `proot`. Published to GHCR.

5. **CI integration docs.** GitHub Actions / GitLab CI examples
   showing how to use minibox as a service container or
   sidecar.

### Architectural changes

| Change                         | Crate    | Scope                            |
| ------------------------------ | -------- | -------------------------------- |
| Rename/generalize gke adapter  | minibox  | `adapters/gke/` -> `adapters/proot/` |
| Capability detection at startup | miniboxd | `adapter_registry.rs`           |
| Dockerfile for minibox image   | repo root | New `Dockerfile`                |
| CI integration examples        | docs     | New doc                          |

### Security implications

- When running inside Docker with `--privileged`, minibox has full
  host capabilities through the Docker container. This is the same
  trust model as running minibox directly on the host.
- When running unprivileged (proot mode), minibox provides weaker
  isolation (proot is ptrace-based, not kernel namespace-based).
  This is acceptable for CI test isolation but not for
  multi-tenant workloads.
- The proot adapter should log a warning at startup indicating
  reduced isolation.

### Priority: **High**

Low implementation effort (gke adapter already exists), high value
for CI adoption. Users who cannot install minibox on their CI hosts
can run it as a Docker container. The main work is packaging
(Dockerfile) and documentation.

---

## Cross-cutting requirements

These capabilities are needed by multiple scenarios and should be
implemented as shared infrastructure.

### User namespace support (scenarios 1-4)

- Add `new_user: bool` to `NamespaceConfig`.
- Implement UID/GID mapping via `newuidmap`/`newgidmap` or direct
  `/proc/<pid>/uid_map` writes.
- Allocate UID ranges: start with a fixed offset (e.g., 100000)
  and 65536 UIDs per container. Store allocations in daemon state
  to prevent overlap.
- ID-mapped mounts (kernel 5.12+) for shared storage between
  user-namespaced containers.

### Cgroup delegation (scenarios 1-4)

- When creating a container's cgroup, write to
  `cgroup.subtree_control` to enable controllers in the subtree.
- `chown` the cgroup directory to the container's mapped UID so
  inner runtimes can create child cgroups.
- Track delegated cgroup paths for cleanup on container removal.

### Seccomp BPF (scenarios 1-4)

- New domain trait: `SeccompProvider` with method
  `load_profile(&self, config: &SeccompConfig) -> Result<()>`.
- Default profile: block dangerous syscalls (kexec_load,
  init_module, etc.) while allowing namespace/mount operations.
- Inner-runtime profile: allowlist clone, mount, pivot_root,
  unshare, setns, plus standard syscalls.

### Port forwarding (scenarios 3-4)

- Extend `NetworkProvider` trait with
  `forward_port(container_ip, container_port, host_port)`.
- Implementation: iptables DNAT rules on the bridge interface.
- Cleanup: remove rules on container stop/remove.

---

## Implementation priority summary

| Priority | Scenario           | Depends on         | Effort |
| -------- | ------------------ | ------------------ | ------ |
| 1        | Image building (5) | Nothing            | Medium |
| 2        | MinD (6)           | Nothing            | Low    |
| 3        | DinD via Minibox (4) | User NS, cgroup  | High   |
| 4        | DinM (1)           | User NS, cgroup    | High   |
| 5        | KinM (3)           | DinM + networking  | Very high |
| 6        | MinM (2)           | DinM               | Low (after DinM) |

Image building and MinD can proceed independently and immediately
after the stabilization freeze lifts. They require no changes to
the non-goals list. Scenarios 1, 3, and 4 require promoting user
namespace support from non-goal to goal.

---

## Open questions

1. **UID range allocation strategy.** Fixed offset (simple, limited
   to ~1000 containers) vs dynamic allocation with persistent state
   (complex, unlimited)? Sysbox-CE uses fixed shared ranges;
   sysbox-EE uses exclusive per-container ranges.

2. **Seccomp profile format.** OCI runtime spec defines a JSON
   seccomp profile format. Should minibox adopt it for
   compatibility, or define its own simpler format?

3. **Build cache invalidation.** Content-addressed (Docker-style,
   hash of instruction + parent layer) or timestamp-based (simpler
   but less correct)?

4. **GKE adapter rename.** Renaming `gke` to `proot` is a breaking
   change for users who set `MINIBOX_ADAPTER=gke`. Keep `gke` as
   an alias?

5. **Nested overlay performance.** How much overhead does
   overlay-on-overlay add? Need benchmarks before committing to
   the approach vs. requiring `vfs` or `fuse-overlayfs` for inner
   runtimes.
