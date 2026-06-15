# Minibox-in-Minibox (DinD) Analysis

Last updated: 2026-06-15

---

## Current State: Working (manual setup)

Minibox-in-minibox works end-to-end on the native Linux adapter today. The
existing integration test (`crates/miniboxd/tests/system_tests.rs:504`,
`test_e2e_dind_pull_and_run`) proves the full lifecycle:

1. Outer miniboxd starts and pulls alpine
2. Inner miniboxd binary + CLI bind-mounted into a privileged container
3. Host cgroup slice bind-mounted; inner cgroup manually delegated
4. Inner daemon starts, pulls alpine, runs `echo hello-from-dind`
5. Output verified, cleanup performed

### What works

| Component                  | Status | Notes                                    |
| -------------------------- | ------ | ---------------------------------------- |
| Privileged mode            | Done   | `capset(2)` grants curated cap set       |
| Cap exclusion list         | Done   | SYS_MODULE, SYS_BOOT, MAC_OVERRIDE/ADMIN |
| Bind mounts into container | Done   | `-v host:container[:ro]` syntax          |
| Cgroup delegation (manual) | Done   | `MINIBOX_CGROUP_ROOT` env override       |
| Overlay avoidance          | Done   | Inner data dir on host tmpfs via bind    |
| DinD integration test      | Done   | Serial, requires root + cgroups v2       |
| Preflight cgroup probe     | Done   | `cgroup_subtree_delegatable` check       |

### What's limited

| Component           | Status      | Detail                             |
| ------------------- | ----------- | ---------------------------------- |
| `/dev` population   | Minimal     | Basic `/dev` from mount namespace; |
|                     |             | no `mknod` for full device set     |
| Cgroup delegation   | Manual only | User must `mkdir` + write          |
|                     |             | `subtree_control` themselves       |
| Overlay-on-overlay  | Avoided     | Workaround: host tmpfs bind mount; |
|                     |             | no native nested overlay support   |
| Non-native adapters | N/A         | smolvm/krun already run full Linux |
|                     |             | VM; nesting is the architecture    |
| Port forwarding     | Missing     | No way to expose inner sockets to  |
|                     |             | outer host                         |
| Proc mount          | Implicit    | Works via pivot_root flow; no      |
|                     |             | explicit `mount_proc` function     |

### Platform support

Only the `native` adapter supports privileged mode and bind mounts per the
feature matrix. The VM-based adapters (smolvm, krun) inherently provide a
full Linux kernel, so "nesting" there means running miniboxd inside the VM
-- which is already the default architecture, just not user-facing.

---

## Architecture of the DinD Test

```
Host (Linux, root, cgroups v2)
  |
  +-- outer miniboxd (host socket)
        |
        +-- alpine container (--privileged)
              |  Bind mounts:
              |    miniboxd binary -> /usr/local/bin/miniboxd
              |    mbx binary     -> /usr/local/bin/minibox
              |    /sys/fs/cgroup -> /sys/fs/cgroup
              |    tmpfs data dir -> /minibox-data
              |    tmpfs run dir  -> /minibox-run
              |
              +-- inner miniboxd (MINIBOX_CGROUP_ROOT=delegated slice)
                    |
                    +-- alpine container
                          `echo hello-from-dind`
```

Key design decision: inner daemon's data dir is on host tmpfs (via bind
mount), not the outer container's overlay. This avoids the kernel's
overlay-on-overlay limitation entirely.

---

## Next Steps

### P0 -- Automatic cgroup delegation

Currently the DinD test script manually runs:

```sh
mkdir -p /sys/fs/cgroup/{slice}
echo '+memory +cpu +pids' > /sys/fs/cgroup/{slice}/cgroup.subtree_control
```

The daemon should do this automatically when `--privileged` is set and the
container needs nested cgroup control. Options:

- **A)** Add a `--cgroup-parent` flag to `minibox run` that creates and
  delegates the cgroup slice before exec. Similar to Docker's
  `--cgroup-parent`.
- **B)** Auto-detect when the container process is miniboxd and delegate
  a child cgroup. More magic, less explicit.
- **C)** Document the manual approach and ship a helper script.

Recommendation: **A** -- explicit, composable, no special-casing.

### P1 -- `/dev` population for complex workloads

Alpine busybox works without explicit device nodes. Heavier workloads
(apt, systemd, anything touching `/dev/null` or `/dev/urandom`) will fail.

Options:

- Populate `/dev/{null,zero,urandom,random,tty,console}` via bind mounts
  from the host before pivot_root when `--privileged` is set.
- Gate behind a `--init-dev` flag to avoid changing default behavior.

### P2 -- User-facing documentation

No guide exists for running minibox-in-minibox outside the test suite.
Write a `docs/NESTING.md` covering:

- Prerequisites (Linux, root, cgroups v2, overlay)
- Manual setup steps
- Example `minibox run` invocation
- Troubleshooting (overlay-on-overlay, cgroup delegation failures)

### P3 -- Port forwarding / socket exposure

Inner containers have no way to expose services to the outer host. This
blocks real DinD use cases like running inner miniboxd as a build service.
Requires the bridge networking feature to mature first.

### P4 -- CI coverage

The DinD test runs with `#[serial]` and requires root + cgroups v2 +
network. Verify it runs on the self-hosted runner (`jobrien-vm`). Add it
to the `test-e2e` just target if not already included.

---

## Risk Assessment

| Risk                          | Likelihood | Impact | Mitigation             |
| ----------------------------- | ---------- | ------ | ---------------------- |
| Overlay-on-overlay kernel bug | Low        | High   | tmpfs bind mount trick |
| Cgroup controller missing     | Medium     | Medium | Preflight probe exists |
| Inner daemon orphaned on      | Medium     | Medium | PID namespace + kill   |
| outer crash                   |            |        | in cleanup             |
| Privilege escalation via      | Low        | High   | Cap exclusion list;    |
| nested privileged container   |            |        | no SYS_MODULE/SYS_BOOT |
| `/dev` absence causes silent  | Medium     | Low    | Explicit error on      |
| failures in inner container   |            |        | missing device access  |
