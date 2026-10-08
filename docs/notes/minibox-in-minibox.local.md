# Minibox-in-Minibox (DinD) Analysis

Last updated: 2026-10-02
guest run)

---

## Summary

Nested containers **do work on the `native` adapter** on a real Linux host —
that is what `just smoke` (`crux/dev/smoke.crux`) exercises, and it is the only
backend with genuine namespaces. The macOS VM adapters are the constrained ones,
for substrate reasons rather than container-model reasons.

A live run during this pass found and fixed a real bug in the privileged path
(see "Bug found and fixed" below). The remaining blockers are listed under
"Open items", and each is specific rather than general.

---

## Current State

| Backend | privileged | bind mounts | cgroups writable by container | Nesting |
| ------- | ---------- | ----------- | ----------------------------- | ------- |
| `native` (Linux) | yes | yes | parent-side only; delegated subtree works | **yes** |
| `colima` (macOS) | yes (verified) | yes (verified) | yes (runs as VM root) | nested daemon verified; pull blocked on network |
| `smolvm` (macOS) | no-op | no — no host FS via virtiofs | guest is itself a container | not viable |
| `krun` / `vz` | untested | untested | untested | unknown |

### Depth tracking

`crates/minibox/src/nesting.rs`: `MINIBOX_NEST_DEPTH` / `MINIBOX_MAX_NEST_DEPTH`,
default max 4. Enforced — `check_depth()` runs in `handle_run`
(`request.rs:30-31`). The daemon increments the counter in the child env
(`preparation.rs`); `integration_tests.rs:672` asserts a container sees
`DEPTH=1`.

Scope caveat: this only bounds nesting of *minibox daemons*. A container
running Docker or podman directly never increments it, so the limit is advisory
in that case.

---

## Bug found and fixed: privileged cgroup placement (`024233e3`)

`delegate_subtree` creates an `init` leaf cgroup, documented as "the init leaf
**where the container process will live**" (`cgroups.rs:365`). But
`preparation.rs` handed the runtime the *subtree root*:

```rust
cgroup_path: cgroup_dir.clone().into(),   // the parent, not the leaf
```

The container process is then written to `<subtree>/cgroup.procs`
(`process.rs:255-259`). cgroup v2 forbids a cgroup that **has children** from
holding processes of its own, so every privileged run failed:

```
failed to add process 1413142 to cgroup /sys/fs/cgroup/minibox/<id>/cgroup.procs:
Resource busy (os error 16)
```

Fixed by placing the process in `cgroup_dir/init` when `privileged`. Verified
live — the container now gets past cgroup placement, enters the namespace with
`cap_eff=000001ffffffffff`, sets its hostname, and proceeds to filesystem setup.

Non-privileged containers were unaffected because they never call
`delegate_subtree`. That asymmetry is why this survived: the broken path is
exactly the DinD path.

---

## Test environment

### What works

`tests/smolfiles/dind.smolfile` + `tests/smolfiles/dind-smoke.sh` provide a
reproducible Linux guest for this. Create once:

```bash
cargo build -p miniboxd -p minibox-cli --target aarch64-unknown-linux-musl
smolvm machine create --name minibox-dind --smolfile tests/smolfiles/dind.smolfile
smolvm machine start  --name minibox-dind
smolvm machine exec   --name minibox-dind -- bash /workspace/tests/smolfiles/dind-smoke.sh
smolvm machine stop   --name minibox-dind
```

The smoke script mirrors `crux/dev/smoke.crux`: start an outer daemon, run a
privileged container with the daemon bind-mounted, delegate a cgroup subtree
*from inside that container*, start an inner daemon there, and have it pull and
run an image.

### Environment requirements discovered

1. **`MINIBOX_ADAPTER=native` is mandatory** in the guest. A smolvm guest is
   itself a smolvm machine, so the `smolvm` binary is present and gets
   auto-selected; the run then fails with `failed to execute smolvm`.
2. **Do not set `MINIBOX_CGROUP_ROOT` on the outer daemon.** With it set, the
   daemon creates a plain directory whose `subtree_control` was never enabled,
   and the first `pids.max` write EPERMs. Left unset, the daemon resolves its
   own supervisor cgroup correctly.
3. **Controllers must be delegated in the guest root.** A bare VM has an empty
   `/sys/fs/cgroup/cgroup.subtree_control`, so children inherit no controllers
   and `pids.max` writes fail. The smolfile enables
   `+cpu +memory +pids +io`. On a normal Linux host systemd has already done
   this, which is why the native adapter cannot be relied on to do it itself.
4. **`smolvm` volume mounts do work** for provisioning (`volumes = ["./:/workspace"]`
   populated correctly), so the virtiofs limitation that blocks the *minibox
   smolvm adapter's* `-v` flags does not apply to smolvm's own mount mechanism.
   The two are separate code paths.
5. **smolvm guests cannot host the native adapter's cgroup work.** The guest is
   launched as a detached container, so its cgroup namespace is a delegated
   subtree; `/sys/fs/cgroup/cgroup.type` is absent and adding a process to a
   cgroup fails with EBUSY. `colima` (a real VM) does not have this problem and
   is the better substrate — verified `mkdir` + `subtree_control` delegation +
   process move all succeed there.

Cross-compiling for the guest needs the musl linker, and the shell is not
persistent between tool invocations, so the env has to be set in the same call:

```bash
CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc \
  cargo build -p miniboxd -p minibox-cli --target aarch64-unknown-linux-musl
```

`just smoke` expects `target/release/{miniboxd,mbx}`; a `--release` musl build
failed to link here (the macOS linker was selected despite the env var), so the
harness above uses the debug target dir instead.

---

## Open items

### P0 — rootfs ownership vs. the uid range (blocks native here)

`UidRangeMode::Exclusive` (the default) allocates a distinct 65,536-ID host
range per container, so container-uid-0 is **not** host-root. The rootfs is
created by the daemon and owned by host root, and nothing chowns it to the
container's mapped range — `chown` appears only in image/commit paths, never in
the container rootfs setup.

Consequence: the containerized child cannot create directories in a root-owned
rootfs, which fails during `pivot_root`:

```
pivot_root: setup runtime directories
  create .../merged/run
  Permission denied (os error 13)
```

This is not colima-specific — the overlay there is `rw` and writable from a root
shell, so the environment is fine and the mapping is the constraint.

Two ways out, both design decisions rather than bugs to fix blindly:
chown the rootfs to the container's mapped uid range, or create the runtime
directories before entering the user namespace.

Note this also explains the earlier bind-mount failure in the privileged path:
`failed to create parent for bind mount target .../usr/local/bin/miniboxd` is
the same inability, reached through a different code path.

### P1 — Colima container networking

Colima containers get loopback only — `/proc/net/dev` shows just `lo`,
`/etc/resolv.conf` is empty — because the spawn script runs
`unshare --net` with no veth, bridge, or DHCP behind it. The inner daemon
cannot pull an image.

`skip_network_namespace` exists in `ContainerSpawnConfig` but the colima adapter
never reads it; it appears only in two test fixtures. This is unimplemented,
not misconfigured.

### P2 — `native_network_provider` undefined

`crates/miniboxd/src/main.rs:1501,1516` call `native_network_provider` under
`#[cfg(target_os = "linux")]`, but the function is not defined anywhere in the
workspace. miniboxd's binary unit tests therefore fail to compile on Linux:

```
error[E0425]: cannot find function `native_network_provider` in this scope
```

macOS skips those tests via cfg, so a macOS-only dev loop never sees it. It
blocks `cargo test --no-run -p miniboxd --target aarch64-unknown-linux-musl`,
which is the command `test-in-vm` runs, so cross-compiling the suite for Linux
currently fails outright.

### P3 — Port forwarding

Absent on every backend. `minibox-domain/src/capability.rs:30` references
port-forward conformance tests as future work; the live capability matrix reports
`port_forwarding: unsupported` for all backends.

---

## Things that were wrong in earlier revisions of this document

Recorded so the corrections are not re-reintroduced:

- **`justfile:134` → `crux/dev/test_linux.crux` does exist.** An earlier revision
  claimed otherwise. The mistake was looking in `.crux/` when the pipelines live
  in `crux/dev/`.
- **`.crux/promote.crux` is not the env-var source.** `promote.crux:6-9`
  deliberately excludes the container smoke tests because GitHub-hosted runners
  do not delegate cgroups. The DinD smoke test is `crux/dev/smoke.crux`.
- **The policy gate is not why DinD is unrunnable in general.** It matters on a
  Linux host where the native adapter is in play, but adapter capability is the
  broader constraint.
- **"A userns child cannot manage cgroups" was wrong.** `process.rs:238-250` says
  the child cannot add *itself* to a cgroup — which is why the parent does it —
  not that it cannot `mkdir` cgroup directories or write `cgroup.subtree_control`.
  The smoke recipe depends on exactly those and they work. The real limitation is
  narrower and is written up as P0 above.

---

## Verified working

| Capability | Where |
| ---------- | ----- |
| Privileged mode via `capset(2)` | `process.rs:376-457`, excludes SYS_MODULE/SYS_BOOT/MAC_OVERRIDE/MAC_ADMIN |
| Bind mounts `-v host:container[:ro]` | `system_tests.rs:526-530`; colima path verified live |
| Bind target auto-creation (Docker parity) | `colima.rs` `bind_mount_shell_snippet` |
| Automatic cgroup delegation on `--privileged` | `preparation.rs`, `cgroups.rs:354-380` |
| `--cgroup-parent` flag with validation | `cgroups.rs:71-86` |
| `/dev` device set | `colima.rs` `dev_setup_fragment`, from `fs_util::default_device_nodes()` |
| proc / sysfs / cgroup2 / tmpfs mounts | `colima.rs` `namespace_exec_fragment` |
| Writable container rootfs | `colima.rs` `overlay_base_dir` — VM-local upperdir |
| `MINIBOX_CGROUP_ROOT` override | `cgroups.rs:53-65` |
| Preflight `cgroup_subtree_delegatable` probe | `preflight.rs:28,50,92-97` |
| Three-level policy opt-in | env vars, `config.toml`, project `minibox.toml` |
| Capability drift gate | `cargo xtask capabilities verify --backend <name>` |
