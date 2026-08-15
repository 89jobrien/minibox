# Design: VZ.framework Adapter Revival

## STATUS UPDATE (2026-08-15, post-implementation): regression NOT actually fixed

The Goal statement below is **wrong** and is kept only for historical record of what was
believed when this design was approved. A Lima `--vm-type=vz` boot repro was taken as proof
the Tahoe `VZErrorInternal(1)` regression was fixed on macOS 26.4 — but Lima's successful
boot used `VZEFIBootLoader` (disk-image based), not `VZLinuxBootLoader` (raw kernel +
initramfs), which is what this adapter actually needs.

A minimal, standalone repro isolating `VZLinuxBootLoader` from all of minibox's own
configuration (no virtiofs, no vsock, no serial port — just boot loader + memory + cpu)
reproduces the exact same failure the code once shipped against:

```
VM start FAILED: Internal Virtualization error. The virtual machine failed to start.
(domain=VZErrorDomain code=1)
```

Tested against **two different kernel/initramfs pairs** to rule out a stale/bad image:
the existing `~/.minibox/vm/boot/` files (~4 months old) and a freshly downloaded
official Alpine v3.22 aarch64 `netboot` kernel+initramfs — **identical failure both
times**. This rules out "stale kernel file" as the cause.

**Conclusion**: `VZLinuxBootLoader` itself is still broken on macOS 26.4, independent of
minibox's code and independent of which kernel is used. The adapter code restored by
this plan is real, compiles clean, and is architecturally sound (see the rest of this
doc and `docs/plans/2026-08-15-vz-adapter-revival.md`'s 8 completed tasks) — but it
cannot actually boot a VM on this machine's OS build today. `vz` should be treated as
**not functional**, not merely "unvalidated," until Apple fixes `VZLinuxBootLoader`
specifically, or minibox switches to `VZEFIBootLoader` (a materially different,
disk-image-based boot mechanism — out of scope for this design).

## Goal (original, superseded by the status update above)

Restore the Apple Virtualization.framework (`vz`) macOS container adapter, since the
Tahoe-beta `VZErrorInternal(1)` regression that forced its removal is confirmed fixed on
macOS 26.4 (verified via a Lima `--vm-type=vz` boot repro: clean `Starting` -> `running`
transition, no error, guest reachable in ~25s).

## Approved Approach

Revert commit `00ee4427` (`drop(vz): remove VZ adapter and all associated code`, issue
#305, 2026-05-07) to restore the prior working implementation verbatim, then re-validate
its isolation test suite and GCD main-queue dispatch workarounds against the current OS
build before wiring it back in as a selectable (non-default) adapter — rewriting from
scratch is explicitly rejected: the removed code was functionally complete (10 isolation
tests in `vz_isolation_tests.rs` plus a smoke test in `vz_adapter_smoke.rs`, passing up
until the Apple regression hit) and re-deriving it would re-introduce the same
GCD/dispatch-queue bugs that were already solved and documented in commits `b53c7c68`,
`db4caf04`, `97e99eed`, `d9491053`.

## Crate Ownership

- **Owner crate**: `macbox` — already owns all macOS-only adapter code (`krun`, `paths`,
  `preflight`); `vz` is restored as a sibling module, feature-gated behind `vz` (mirrors
  the original scheme, not additive).
- **Affected crates**: `miniboxd` (composition root wiring, `AdapterSuite` enum,
  `adapter_registry.rs`), `xtask`/`Justfile` (restore `test-vz-isolation` recipe).

## Public API

Restored verbatim from `00ee4427^` — no new names invented, no signature changes.

### Traits

No new traits. `Vz*` types implement the existing domain ports:
`ContainerRuntime`, `ImageRegistry`, `FilesystemProvider`, `ResourceLimiter`
(`minibox_core::domain`).

### Types

```rust
// crates/macbox/src/vz/adapter.rs
pub struct VzRuntime { /* Arc<VzVm> */ }
pub struct VzRegistry { /* Arc<VzVm> */ }
pub struct VzFilesystem { /* Arc<VzVm> */ }
pub struct VzLimiter { /* Arc<VzVm> */ }

// crates/macbox/src/vz/vm.rs
pub struct VzVm { /* ... */ }

// crates/macbox/src/vz/proxy.rs
pub struct VzProxy { /* wraps vsock stream */ }
```

### Functions

```rust
// crates/macbox/src/vz/vsock.rs
pub async fn connect_to_agent(vm: &VzVm, timeout_secs: u64) -> anyhow::Result<VsockStream>;

// crates/macbox/src/lib.rs — restored verbatim; NOT the
// build_*_handler_dependencies() pattern colima/krun use. vz instead owns its
// own accept-loop entry point, because VZVirtualMachine must be created and
// polled on the GCD main queue (see the two-phase dispatch_sync_f/dispatch_main
// dance in the removed code) — a shape the other adapters don't need.
#[cfg(feature = "vz")]
async fn start_vz(
    socket_path: std::path::PathBuf,
    images_dir: std::path::PathBuf,
    containers_dir: std::path::PathBuf,
    run_containers_dir: std::path::PathBuf,
    state: Arc<minibox::daemon::state::DaemonState>,
) -> anyhow::Result<()>;

// crates/miniboxd/src/main.rs — restored verbatim
#[cfg(all(target_os = "macos", feature = "vz"))]
fn vz_main() -> !; // calls start_vz() then dispatch_main() (divergent)

// crates/miniboxd/src/adapter_registry.rs
// AdapterSuite gains one variant:
pub enum AdapterSuite {
    Native,
    Gke,
    Colima,
    SmolVm,
    Krun,
    Vz, // new
}
```

## Data Flow

1. Source: `mbx` CLI request -> daemon Unix socket -> `HandlerDependencies` dispatch.
2. Transform: handler builds a `DaemonRequest`, opens a vsock connection to the running
   `VzVm` via `connect_to_agent`, sends the request through `VzProxy`.
3. Sink: in-VM `miniboxd` agent processes the request natively (real Linux namespaces/
   cgroups inside the VM) and streams `DaemonResponse` values back over vsock; the host
   adapter forwards the terminal response to the CLI.

## Hexagonal Boundaries

- **Port** (trait): `ContainerRuntime`, `ImageRegistry`, `FilesystemProvider`,
  `ResourceLimiter` in `minibox_core::domain` — unchanged, already exist.
- **Adapter** (impl): `VzRuntime`/`VzRegistry`/`VzFilesystem`/`VzLimiter` in
  `crates/macbox/src/vz/adapter.rs` — restored, forward to `VzVm` over vsock exactly as
  `KrunRuntime`/etc. forward to libkrun.

## Out of Scope

- Making `vz` the default macOS adapter — stays opt-in via `MINIBOX_ADAPTER=vz` until it
  has a track record across more than one machine/OS build.
- Any change to `smolvm`/`krun`/`colima` selection or fallback logic.
- Windows support.
- Fixing the separate `docker load`-in-guest / CAP_SYS_ADMIN issue found on `smolvm` this
  session — unrelated adapter, unrelated bug.
- New capabilities beyond what the removed implementation had (e.g. no attempt to add
  `exec`/`logs` support beyond whatever the original `Vz*` adapters already provided).

## Risk

- [ ] Breaking API changes: **no** — pure restoration, existing adapters/selection
      behavior for `native`/`gke`/`colima`/`smolvm`/`krun` untouched; `vz` is additive and
      opt-in.
- [ ] New external dependency: **yes** — `block2`, `objc2`, `objc2-foundation`,
      `objc2-virtualization` (0.3/0.6-series, per the removed `Cargo.toml`). Justified as
      restoring exactly what was already vetted and working before the Apple regression;
      no new crates beyond what `00ee4427` removed.
- [ ] Feature flag required: **yes** — `vz` cargo feature on `macbox`, matching the prior
      scheme (`vz = ["dep:block2", "dep:objc2", "dep:objc2-foundation",
"dep:objc2-virtualization"]`).

## Validation Plan (pre-merge, not part of the API surface)

1. [x] Restore `crates/macbox/src/vz/*` and `start_vz`/`vz_main` from `00ee4427^`,
       adapted to ~3.5 months of protocol/domain drift (`51bbce1d`, `5469ff25`, `50ba968e`).
2. [x] `cargo build -p macbox --features vz` — compiles against current
       `objc2 0.6.4`/`objc2-virtualization 0.3.2` with no upstream breaking changes; required
       fixing a real `!Send` future bug in `vsock.rs`'s completion-handler poll loop
       (`spawn_blocking`, mirroring `VzVm::wait_for_running`'s existing pattern) — see `5469ff25`.
3. [x] Re-ran `just test-vz-isolation` — the `dispatch_main()` + entitlements harness
       builds, codesigns, and runs cleanly end-to-end with **no `VZErrorInternal(1)`, no hang**
       (`9c3766cc`). Caveat: all 10 tests report `SKIP` because no VM image exists at
       `~/.minibox/vm/` — `cargo xtask build-vm-image`, referenced by the test's own skip
       message, does not exist in this codebase and never has. This confirms the harness is
       sound; it does **not** confirm `VZErrorInternal(1)` is gone against a real minibox VM
       boot, only against Lima's separate `vz` driver (see Goal) and the harness plumbing
       itself. Building a real kernel/rootfs image for `VZLinuxBootLoader` is out of scope here.
4. [x] Re-ran `vz_adapter_smoke.rs` (`1b0bb5ea`) — passes, same VM-image-absent caveat as
       step 3.
5. [x] Wired `AdapterSuite::Vz` into `adapter_registry.rs` `VALID_ADAPTERS`/`parse_adapter`
       (`e577f873`) — scope narrowed from what this line originally assumed:
       `build_vz_handler_dependencies` was never added, because `vz` doesn't go through
       `build_handler_deps` at all. `main()` diverts to `vz_main()` before `run_daemon()` ever
       runs, since VM boot needs the OS main thread for GCD callbacks — a requirement that
       predates, and remains incompatible with, the unified `AdapterSuite`/`build_handler_deps`
       dispatch every other adapter now goes through.
