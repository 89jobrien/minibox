# Design: VZ.framework Adapter Revival

## Goal

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

1. `git revert 00ee4427` (or cherry-pick the pre-removal tree) onto `develop`.
2. `cargo build -p macbox --features vz` — confirm it still compiles against current
   `objc2`/`objc2-virtualization` crate versions (may have advanced since removal; check
   for breaking upstream API changes independently of the Apple OS bug).
3. Re-run `just test-vz-isolation` (or `cargo xtask` equivalent) — confirm
   `vz_isolation_tests.rs` passes without the `VZErrorInternal(1)` failure, using the
   `dispatch_main()` + entitlements harness from `b53c7c68`.
4. Re-run `vz_adapter_smoke.rs` against a live daemon with `MINIBOX_ADAPTER=vz`.
5. Only after 2-4 pass: wire `AdapterSuite::Vz` into `adapter_registry.rs`
   `VALID_ADAPTERS`/`parse_adapter`, and add `build_vz_handler_dependencies` call site in
   `crates/miniboxd/src/main.rs`, gated the same way `krun`/`colima` are today.
