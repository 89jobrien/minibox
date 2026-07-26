# Nested Containers (minibox-in-minibox)

**Date:** 2026-05-26
**Status:** done
**Author:** Joseph O'Brien

## Goal

Enable minibox containers to run miniboxd inside them, which in turn creates
child containers. Primary use case is single-level nesting (outer container
runs inner miniboxd), with recursive nesting supported up to a configurable
max depth (default 4). This unlocks CI-in-minibox, integration testing of
miniboxd itself inside minibox, and agent-spawned sandbox chains.

## Architecture

### Crate scope

All changes live in the `minibox` crate (container init path). No new crates.

Affected modules:

| Module                    | Change                                                   |
| ------------------------- | -------------------------------------------------------- |
| `container/process.rs`    | Nest depth env injection, /proc remount, /dev setup      |
| `container/cgroups.rs`    | Subtree delegation helper                                |
| `container/filesystem.rs` | Overlay-on-overlay detection, tmpfs fallback, /dev setup |
| `preflight.rs`            | Empirical overlay nesting probe                          |
| `container/mod.rs`        | `NestingContext` type                                    |

### New types

```rust
/// Nesting metadata passed through the container init path.
pub struct NestingContext {
    /// Current depth (0 = host, 1 = first container, 2 = nested, ...).
    pub depth: u32,
    /// Maximum allowed depth. Container init fails if depth >= max.
    pub max_depth: u32,
    /// Whether the kernel supports overlay-on-overlay (probed empirically).
    pub overlay_nesting: bool,
}
```

### Data flow

```
Host (depth 0)
  |-- minibox run --privileged alpine
  |     env: MINIBOX_NEST_DEPTH=1
  |     init: delegate cgroup subtree
  |           mount tmpfs at /dev + mknod
  |           remount /proc (if new PID NS)
  |           setup overlay (native)
  |
  +-- Inner container (depth 1)
        |-- miniboxd starts, reads MINIBOX_NEST_DEPTH=1
        |-- minibox run alpine echo hello
        |     env: MINIBOX_NEST_DEPTH=2
        |     init: delegate cgroup subtree (under parent's delegated slice)
        |           mount tmpfs at /dev + mknod
        |           remount /proc
        |           setup overlay (native if probe passes, tmpfs-copy fallback)
        |
        +-- Inner-inner container (depth 2)
              runs "echo hello", exits
```

## Design decisions

### 1. Cgroup delegation (automatic, with override)

When `privileged: true`, container init:

1. Reads its own cgroup from `/proc/self/cgroup`.
2. Creates a child cgroup named `minibox-<container-id>`.
3. Writes `+cpu +memory +pids +io` to `cgroup.subtree_control`.
4. Moves the container process into a `init` leaf under the new subtree
   (a cgroup with `subtree_control` set cannot have direct member processes).

The inner miniboxd discovers its cgroup the same way and creates children
under it. This works recursively without configuration.

If `--cgroup-parent` is explicitly set, it overrides the auto-detected path.
The delegation logic still runs but rooted at the specified parent.

### 2. Overlay-on-overlay with fallback

Nested overlay (using an overlay mount as a lowerdir for another overlay)
works on some kernel versions but not others. The support depends on kernel
version, config options (`CONFIG_OVERLAY_FS_METACOPY`, redirect_dir), and
filesystem flags. There is no single kernel version cutoff that reliably
predicts support.

Detection — empirical probe (same approach as Podman):

```rust
fn supports_nested_overlay() -> bool {
    // Try a real overlay-on-overlay mount in a tmpdir.
    // 1. Mount tmpfs
    // 2. Create lower/upper/work/merged dirs
    // 3. Mount first overlay (lower -> merged1)
    // 4. Attempt second overlay using merged1 as lowerdir
    // 5. If mount succeeds, nested overlay is supported
    // 6. Clean up and cache result (static OnceLock<bool>)
    probe_nested_overlay_mount()
}
```

The result is cached for the process lifetime via `OnceLock<bool>`.

Fallback when probe fails:

1. Create a tmpfs mount.
2. Copy (or reflink where supported) the image layers into the tmpfs.
3. Mount overlay with the tmpfs copies as lowerdirs.

This is slower but correct. The fallback is only triggered at depth >= 2
(the host-level overlay is always fine).

### 3. Nesting depth detection via environment

Container init sets `MINIBOX_NEST_DEPTH=N` where N = parent depth + 1.

- Host miniboxd: depth 0 (env var absent or explicitly 0).
- First container: depth 1.
- Nested container: depth 2, etc.

If `depth >= max_depth`, container creation fails with:

```
Error: nesting depth 4 exceeds maximum (MINIBOX_MAX_NEST_DEPTH=4)
```

`MINIBOX_MAX_NEST_DEPTH` is configurable via env or daemon config. Default 4.

### 4. /dev setup (tmpfs + mknod)

Following the same approach as runc/libcontainer: mount a fresh `tmpfs` at
`/dev` and explicitly create device nodes with `mknod`. This is more
portable than `devtmpfs` (which may fail inside a non-initial mount
namespace or require `CONFIG_DEVTMPFS`) and works reliably at any nesting
depth.

In the child init path (after `CLONE_NEWNS`, before `pivot_root`):

1. Mount `tmpfs` at `<rootfs>/dev` with `mode=0755`.
2. Create device nodes via `mknod`:
    - `/dev/null` (c 1, 3)
    - `/dev/zero` (c 1, 5)
    - `/dev/full` (c 1, 7)
    - `/dev/random` (c 1, 8)
    - `/dev/urandom` (c 1, 9)
    - `/dev/tty` (c 5, 0)
    - `/dev/console` (c 5, 1)
    - `/dev/ptmx` (c 5, 2) -> symlink to `/dev/pts/ptmx`
3. Mount `devpts` at `<rootfs>/dev/pts` with `newinstance,ptmxmode=0666`.
4. Create symlinks:
    - `/dev/fd` -> `/proc/self/fd`
    - `/dev/stdin` -> `/proc/self/fd/0`
    - `/dev/stdout` -> `/proc/self/fd/1`
    - `/dev/stderr` -> `/proc/self/fd/2`
5. Create `/dev/shm` directory (tmpfs mount point for POSIX shared memory).

This runs for all privileged containers, not just nested ones, since it
improves isolation regardless.

### 5. /proc remount

After `CLONE_NEWPID` + `CLONE_NEWNS`, the child must remount `/proc` to
reflect its own PID namespace:

```rust
// In child init, after pivot_root:
mount("proc", "/proc", "proc", MS_NOSUID | MS_NOEXEC | MS_NODEV, None)
```

This is already implicit in the pivot_root path (the old /proc becomes
inaccessible), but we need to explicitly mount a new one inside the new
rootfs for the inner miniboxd to function.

## Security considerations

- **Max depth limit** prevents resource exhaustion from unbounded nesting.
- **Excluded capabilities propagate**: CAP_SYS_MODULE, CAP_SYS_BOOT,
  CAP_MAC_OVERRIDE, CAP_MAC_ADMIN are excluded at every level. The inner
  container cannot escalate beyond what the outer grants.
- **Cgroup delegation is scoped**: the inner miniboxd can only create
  cgroups under its own subtree, not escape to parent slices.
- **Overlay fallback uses tmpfs**: no persistent writes to the outer
  container's filesystem from the inner overlay.

## Out of scope

- **Network nesting** (nested bridge networks, NAT chains) -- the inner
  container inherits the outer's network namespace or gets `--network none`.
  Nested network bridges are a separate feature.
- **Image sharing** between nesting levels -- each miniboxd has its own
  image store. Shared layer caching across nesting levels is future work.
- **User namespace nesting** -- `CLONE_NEWUSER` is not currently used by
  minibox. Adding it for rootless nesting is a separate effort.
- **Windows/macOS nesting** -- this is Linux-only. macOS adapters (smolvm,
  krun) already provide a VM boundary; nesting inside those VMs follows
  the same Linux path.

## Test plan

1. **Unit tests** (in-crate, no root needed):
    - `NestingContext` construction and depth validation.
    - Overlay probe logic (mock mount syscall).
    - Cgroup delegation path generation.
    - Device node list completeness (tmpfs + mknod).

2. **Integration test** (`#[ignore]`, Linux + root):
    - Outer minibox runs Alpine with miniboxd binary bind-mounted.
    - Inner miniboxd starts, pulls `busybox`, runs `echo nested-ok`.
    - Assert stdout contains `nested-ok`.
    - Verify inner cgroup was created under outer's subtree.
    - Verify cleanup: inner cgroup removed, overlay unmounted.

3. **Depth limit test** (`#[ignore]`, Linux + root):
    - Set `MINIBOX_MAX_NEST_DEPTH=2`.
    - Nest to depth 2 (succeeds), attempt depth 3 (fails with error).

4. **Overlay fallback test** (`#[ignore]`, Linux + root):
    - Force overlay probe to return `false` (via test-only override).
    - Verify tmpfs-copy path is taken at depth >= 2.
