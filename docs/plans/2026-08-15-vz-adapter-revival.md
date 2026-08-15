# Plan: vz-adapter-revival

## STATUS (2026-08-15, post-completion): all 8 tasks done, but the adapter does not work

All 8 tasks below were completed and committed (`51bbce1d` through `08a72536`). The code
compiles clean, passes clippy, and the isolation/smoke test harnesses run without
crashing. However, a follow-up minimal repro proved `VZLinuxBootLoader` — the boot
mechanism this adapter depends on — still fails with `VZErrorDomain code=1` on this
machine's macOS 26.4, tested against two independent kernel images (including a freshly
downloaded one) to rule out a bad image file. The premise that motivated this plan (the
Tahoe regression is fixed) does not hold for this boot path. See
`docs/designs/2026-08-15-vz-adapter-revival-design.md`'s status update for the full
finding. Treat `vz` as restored-but-non-functional until this is resolved.

## Goal (original)

Restore the Apple Virtualization.framework (`vz`) macOS container adapter — removed in
`00ee4427` due to a Tahoe-beta `VZErrorInternal(1)` regression now confirmed fixed on
macOS 26.4 — as an opt-in, non-default adapter selectable via `MINIBOX_ADAPTER=vz`.

## Architecture

- Crates affected: `macbox` (owner — restores `src/vz/*` module tree, `vz` cargo
  feature, `start_vz`), `miniboxd` (restores `vz_main()` platform dispatch,
  `AdapterSuite::Vz` in `adapter_registry.rs`).
- New traits/types: none new — restores existing `VzRuntime`, `VzRegistry`,
  `VzFilesystem`, `VzLimiter` (`macbox::vz::adapter`), `VzVm`/`VzVmConfig`
  (`macbox::vz::vm`), `VzProxy` (`macbox::vz::proxy`).
- Data flow: `mbx` CLI -> daemon Unix socket -> `start_vz`'s accept loop -> `VzProxy`
  over vsock -> in-VM `miniboxd` agent (real Linux namespaces/cgroups) -> `DaemonResponse`
  streamed back over vsock -> CLI.

See `docs/designs/2026-08-15-vz-adapter-revival-design.md` for the full design.

## Tech Stack

- Rust edition: 2024 (unchanged)
- New dependencies (restored, not net-new to the ecosystem): `block2 0.6`, `objc2 0.6`,
  `objc2-foundation 0.3`, `objc2-virtualization 0.3` — gated behind the `vz` feature

## Tasks

### Task 1: Restore macbox `vz` feature and Cargo dependencies

**Crate**: `macbox`
**File(s)**: `crates/macbox/Cargo.toml`
**Run**: `cargo check -p macbox --features vz`

1. Confirm current state: `cargo check -p macbox --features vz` FAILS (`unknown feature: vz`).
2. Restore the `[features]` block and `block2`/`objc2`/`objc2-foundation`/`objc2-virtualization`
   optional deps from `git show 00ee4427^:crates/macbox/Cargo.toml`, plus the
   `[[test]] name = "vz_isolation_tests" harness = false` entry.
3. `cargo check -p macbox --features vz` — confirm it resolves deps (module files don't
   exist yet, so this step only validates the manifest; full check happens in Task 3).
4. `git commit -m "feat(macbox): restore vz cargo feature and dependencies"`

### Task 2: Restore `macbox::vz` module tree

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/{mod,bindings,vm,proxy,vsock,agent_init,adapter}.rs`
**Run**: `cargo check -p macbox --features vz --lib`

1. Confirm current state: `crates/macbox/src/vz/` does not exist — any reference to
   `macbox::vz::*` FAILS to compile.
2. Restore all seven files verbatim via `git show 00ee4427^:crates/macbox/src/vz/<file>.rs`
   for `mod.rs`, `bindings.rs`, `vm.rs`, `proxy.rs`, `vsock.rs`, `agent_init.rs`,
   `adapter.rs`.
3. `cargo check -p macbox --features vz --lib` — confirm GREEN (module compiles standalone).
4. `git commit -m "feat(macbox): restore vz adapter module tree (VzRuntime/Registry/Filesystem/Limiter)"`

### Task 3: Restore `macbox::vz` public export and `start_vz` wiring

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/lib.rs`
**Run**: `cargo check -p macbox --features vz`

1. Confirm current state: `cargo check -p macbox --features vz` FAILS — `mod vz;` not
   declared, `start_vz()` doesn't exist, nothing wires the four `Vz*` adapters into
   `HandlerDependencies`.
2. Restore `pub mod vz;` and the `#[cfg(feature = "vz")] async fn start_vz(...)` function
   (two-phase `dispatch_sync_f`/GCD main-queue boot, `HandlerDependencies` wiring) from
   `git show 00ee4427^:crates/macbox/src/lib.rs`.
3. `cargo check -p macbox --features vz` — confirm GREEN.
4. `git commit -m "feat(macbox): restore start_vz daemon entry point"`

### Task 4: Restore miniboxd `vz_main` platform dispatch

**Crate**: `miniboxd`
**File(s)**: `crates/miniboxd/Cargo.toml`, `crates/miniboxd/src/main.rs`
**Run**: `cargo check -p miniboxd --features macbox/vz`

1. Confirm current state: `crates/miniboxd/src/main.rs` has no
   `#[cfg(all(target_os = "macos", feature = "vz"))]` branch — building with a `vz`
   feature on `miniboxd` is a no-op today (the feature doesn't exist).
2. Restore the `macbox/vz` optional-feature forwarding in `crates/miniboxd/Cargo.toml`
   and the `vz_main() -> !` function + its call site in `main()` from
   `git show 00ee4427^:crates/miniboxd/Cargo.toml` and `.../src/main.rs`.
3. `cargo check -p miniboxd --features macbox/vz` — confirm GREEN.
4. `git commit -m "feat(miniboxd): restore vz_main platform dispatch"`

### Task 5: Restore and rerun `vz_isolation_tests` — confirm no `VZErrorInternal(1)`

**Crate**: `macbox`
**File(s)**: `crates/macbox/tests/vz_isolation_tests.rs`, `entitlements/vz-test.entitlements`,
`Justfile`
**Run**: `just test-vz-isolation`

1. Confirm current state: `just test-vz-isolation` FAILS — recipe doesn't exist, test file
   doesn't exist.
2. Restore `vz_isolation_tests.rs` (10 tests: `vz_container_can_list_rootfs`,
   `vz_overlay_write_is_ephemeral`, `vz_overlay_image_content_visible`,
   `vz_overlay_write_lands_in_upper_not_lower`, `vz_container_runs_in_cgroup`,
   `vz_container_cgroup_is_minibox_slice`, `vz_pid_namespace_isolated`,
   `vz_uts_namespace_isolated`, `vz_mount_namespace_has_proc`,
   `vz_mount_namespace_has_sys`), `entitlements/vz-test.entitlements`, and the
   `test-vz-isolation` Justfile recipe (codesign + direct binary run, `harness = false`)
   from `git show 00ee4427^:...`.
3. `just test-vz-isolation` — this is the critical check: confirm all 10 tests report
   real pass/fail results, with **no** `VZErrorInternal(1)` anywhere in output (the
   specific regression this plan exists to validate is gone).
4. `git commit -m "test(macbox): restore vz isolation test suite"`

### Task 6: Restore `vz_adapter_smoke` test

**Crate**: `macbox`
**File(s)**: `crates/macbox/tests/vz_adapter_smoke.rs`
**Run**: `cargo nextest run -p macbox --features vz --test vz_adapter_smoke`

1. Confirm current state: test file doesn't exist — command FAILS (`no test target`).
2. Restore `vz_adapter_smoke.rs` (`vz_smoke_list_containers_returns_empty` +
   `boot_vm`/`vm_dir`/`vm_image_available` helpers) from
   `git show 00ee4427^:crates/macbox/tests/vz_adapter_smoke.rs`.
3. `cargo nextest run -p macbox --features vz --test vz_adapter_smoke` — confirm GREEN.
4. `git commit -m "test(macbox): restore vz_adapter_smoke test"`

### Task 7: Wire `AdapterSuite::Vz` into the adapter registry

**Crate**: `miniboxd`
**File(s)**: `crates/miniboxd/src/adapter_registry.rs`
**Run**: `cargo nextest run -p miniboxd --features macbox/vz adapter_registry`

1. Write a failing test in `adapter_registry.rs`'s existing test module asserting
   `parse_adapter("vz")` returns `Ok(AdapterSuite::Vz)` and `VALID_ADAPTERS` contains
   `"vz"`. Confirm FAIL (`Vz` variant doesn't exist yet).
2. Add `AdapterSuite::Vz` (macOS-only availability, `as_str() -> "vz"`), add `"vz"` to
   `VALID_ADAPTERS`, extend `parse_adapter`/`all_adapters` to cover it — mirroring how
   `Krun`/`Colima` are already handled, but gated on
   `cfg(all(target_os = "macos", feature = "macbox/vz"))` so it's absent from
   non-macOS or non-`vz`-feature builds.
3. `cargo nextest run -p miniboxd --features macbox/vz adapter_registry` — confirm GREEN.
4. `cargo clippy -p miniboxd --features macbox/vz -- -D warnings` — zero warnings.
5. `git commit -m "feat(miniboxd): wire AdapterSuite::Vz into adapter registry"`

### Task 8: Update capability matrix and design doc status

**Crate**: docs (no crate)
**File(s)**: `docs/core/FEATURE_MATRIX.mbx.md`, `docs/designs/2026-08-15-vz-adapter-revival-design.md`
**Run**: `cargo xtask verify` (docs lint)

1. Confirm current state: `FEATURE_MATRIX.mbx.md` has no `vz` column — stale relative to
   the newly wired adapter.
2. Add a `vz` column to the Adapter Suites and Capability Matrix tables (status:
   Experimental; same VM-provided isolation row values as `krun`, since both boot a
   guest kernel — verify by reading `VzRuntime::capabilities()` rather than assuming);
   append a `## Notes` entry describing the vsock/GCD-main-queue architecture, mirroring
   the existing `smolvm`/`krun`/`colima` notes. Mark the design doc's Validation Plan
   steps 1-4 as complete with a one-line pointer to the commits from Tasks 1-6.
3. `cargo xtask verify` — confirm docs lint passes (no broken refs / stale format).
4. `git commit -m "docs: add vz adapter to feature matrix, close out design doc validation plan"`

## Notes

- `vz` stays **opt-in only** for this plan — no task makes it the default adapter or
  changes `smolvm`/`krun`/`colima` fallback behavior. Promoting it to default (or even
  fallback) is explicitly out of scope per the design doc and would need its own
  follow-up plan after real-world soak time.
- Every "restore verbatim" task pulls from `00ee4427^` (the commit _before_ the removal),
  not from memory or re-derivation — this avoids reintroducing the GCD/dispatch-queue
  bugs that were already solved and are documented in commits `b53c7c68`, `db4caf04`,
  `97e99eed`, `d9491053`.
- If Task 5's `just test-vz-isolation` run reproduces `VZErrorInternal(1)` (or any hang),
  STOP — that means the Apple regression is not actually fixed for minibox's exact usage
  pattern despite the Lima repro succeeding, and this plan should not proceed past that
  point. Re-open the investigation rather than working around it.
