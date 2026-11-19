# Plan: Snapshot Lineage and the VZ Checkpoint Backend

Design: `docs/designs/2026-11-16-snapshot-lineage-design.md` (approved 2026-11-16).
Closes `TODO(feature-idea-31)` and `TODO(feature-idea-29)`.

## Goal

Give minibox versioned, immutable, tamper-evident VM snapshots with parent lineage, and ship the
first real `VmCheckpoint` implementation (macOS Virtualization.framework) so the port stops being
a stub.

## Architecture

- **Crates affected**: `minibox-domain`, `minibox`, `minibox-core`, `mbx`, `minibox-testsuite`,
  `macbox`, `miniboxd` (tests only). Replacing the port signature forces every consumer to change
  in one compile-clean step, so Phase 2 task 2.5 is deliberately one large task.
- **New types** (`minibox-domain`, new module `snapshot`): `VmId`, `SnapshotLabel`, `ShareTag`,
  `SnapshotId`, `PayloadKind`, `PayloadEntry`, `SnapshotManifest`, `SnapshotInfo` (replaces the
  old one), `SnapshotError`.
- **New store** (`minibox`, `adapters/snapshot_store.rs`): `SnapshotStore`, `SnapshotDraft`,
  `StagingDir`, `VerifiedSnapshot`.
- **New VZ code** (`macbox`): `vz/identity.rs`, `vz/main_queue.rs`, `vz/clone.rs`,
  `vz/checkpoint.rs`, plus `VzVm` primitives.
- **Data flow**: request -> handler validates identifiers -> `VmCheckpoint` port (in
  `spawn_blocking`) -> `VzCheckpoint` pauses the VM, writes state, clones shares into a staging
  directory -> `SnapshotStore::publish` hashes payloads, writes the manifest last, renames staging
  to the content address.

## Tech Stack

- Rust edition 2024. No new external crates. `minibox-domain` gains `miette` (already a workspace
  dependency). `macbox` already has `libc` (which exposes `clonefile` on macOS), `uuid`, `sha2`
  and `serde_json`.
- `nutype` is used for validated identifiers, matching `ContainerId` in
  `crates/minibox-domain/src/ids.rs`.

## Refinements Over the Approved Design

These were found while mapping the code. None change an approved decision.

1. `PayloadKind::ShareClone` carries a validated `ShareTag`, not a bare `String`. A manifest is
   read back from disk, so an unvalidated tag (`../x`) would reach `payload_path` and escape the
   snapshot directory.
2. `SnapshotStore::publish_at(staging, draft, created_at)` is exposed so tests can publish with a
   fixed timestamp. `publish` calls it with the current time. Without it the idempotent
   same-ID branch cannot be tested, because `created_at` is part of the content address.
3. The VZ primitives (`VmIdentity`, config builder, main-queue helper, pause/resume/save/restore)
   move from Phase 3 into Phase 0, because the spike needs pinned identity and a real save/restore
   to answer its questions.
4. Capability declaration: the only place a backend's capabilities are declared is
   `descriptor_for_active_adapter` in `crates/minibox-testsuite/src/showcase/context.rs`, which
   has no VZ arm. Phase 3 adds one.

## Running Tests and Gates

- Red/green iteration uses a single-crate filter: `cargo nextest run -p <crate> <filter>`. The
  workspace delegation hook may block direct `cargo`/`nextest` runs; if it does, delegate the run
  to the `joe-tester` agent with the crate and filter as the concrete target.
- Do not run clippy, fmt or workspace tests by hand before committing. `cargo xtask pre-commit`
  runs on every commit, and `cargo xtask verify` is the canonical gate.
- Linux-gated code is verified with
  `rust-script scripts/qemu-linux-tests/run.rs --filter <module>`. Nothing in this plan is
  Linux-gated; the `SnapshotStore` is portable and runs on Linux CI.
- **Commits.** Every task ends with a commit checkpoint. Per `AGENTS.md`, commit only when the user
  asks. When they do: run `git branch --show-current` (must be `develop`), stage explicit paths,
  read `git diff --cached --stat`, never use `--no-verify`. Push only with explicit confirmation.
  If 1Password signing fails, ask the user to unlock it.

## Phase Overview

| Phase | Tasks                                               | Gate                                                        |
| ----- | --------------------------------------------------- | ----------------------------------------------------------- |
| 0     | 0.1 - 0.5, VZ primitives and the spike (macOS only) | Result decides Phase 3. Stop and report if the spike fails. |
| 1     | 1.1 - 1.3, path hardening                           | Independent. Ships the fix for the live traversal defect.   |
| 2     | 2.1 - 2.6, lineage foundation                       | Independent of Phase 0.                                     |
| 3     | 3.1 - 3.8, VZ backend                               | Requires Phase 0 success and Phase 2.                       |

Phases 0, 1 and 2 are independent and may run in any order or in parallel in isolated worktrees
(`git worktree add ../minibox-<name> -b <name>/<task> develop`). Do not run parallel cargo
invocations against one `target/` directory.

---

# Phase 0: VZ Primitives and Spike (macOS only, crate `macbox`)

All Phase 0 code is behind `#[cfg(all(target_os = "macos", feature = "vz"))]` unless stated.
Verify on macOS only; a green Linux run proves nothing for these files.

### Task 0.1: `VmIdentity` (pinned machine identifier and MAC)

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/identity.rs` (new), `crates/macbox/src/vz/mod.rs`
**Run**: `cargo nextest run -p macbox identity`

The persistence logic is portable and tested everywhere; only generation touches Objective-C.

1. Register the module. In `crates/macbox/src/vz/mod.rs` add `pub mod identity;` next to the
   other `pub mod` lines (check the file for whether the others are cfg-gated and match them).
2. Write failing tests first. Create `crates/macbox/src/vz/identity.rs`:

   ```rust
   //! Pinned VM identity.
   //!
   //! Virtualization.framework requires a restored VM's configuration to match the configuration
   //! it was saved from. The generic platform's machine identifier and the NAT device's MAC
   //! address are random on every build unless they are set explicitly, so both are generated
   //! once and persisted in the VM directory.

   use anyhow::{Context, Result};
   use std::path::Path;

   const MACHINE_ID_FILE: &str = "machine-identifier.bin";
   const MAC_FILE: &str = "mac-address.txt";

   /// Persisted platform machine identifier and NAT MAC address.
   #[derive(Debug, Clone, PartialEq, Eq)]
   pub struct VmIdentity {
       machine_identifier: Vec<u8>,
       mac_address: String,
   }

   impl VmIdentity {
       /// Build an identity from explicit parts.
       #[must_use]
       pub const fn from_parts(machine_identifier: Vec<u8>, mac_address: String) -> Self {
           Self {
               machine_identifier,
               mac_address,
           }
       }

       /// Serialized `VZGenericMachineIdentifier` (`dataRepresentation`).
       #[must_use]
       pub fn machine_identifier(&self) -> &[u8] {
           &self.machine_identifier
       }

       /// MAC address in the `aa:bb:cc:dd:ee:ff` form `VZMACAddress` accepts.
       #[must_use]
       pub fn mac_address(&self) -> &str {
           &self.mac_address
       }

       /// Bytes folded into the snapshot `vm_config_digest`.
       #[must_use]
       pub fn digest_input(&self) -> Vec<u8> {
           let mut out = self.machine_identifier.clone();
           out.push(0);
           out.extend_from_slice(self.mac_address.as_bytes());
           out
       }

       /// Load the identity from `vm_dir`, creating it with `generate` if either file is missing.
       ///
       /// # Errors
       ///
       /// Returns an error if the files cannot be read or written, or `generate` fails.
       pub fn load_or_create_with(
           vm_dir: &Path,
           generate: impl FnOnce() -> Result<Self>,
       ) -> Result<Self> {
           let id_path = vm_dir.join(MACHINE_ID_FILE);
           let mac_path = vm_dir.join(MAC_FILE);
           if id_path.exists() && mac_path.exists() {
               let machine_identifier = std::fs::read(&id_path)
                   .with_context(|| format!("read {}", id_path.display()))?;
               let mac_address = std::fs::read_to_string(&mac_path)
                   .with_context(|| format!("read {}", mac_path.display()))?
                   .trim()
                   .to_string();
               return Ok(Self::from_parts(machine_identifier, mac_address));
           }
           let identity = generate()?;
           std::fs::create_dir_all(vm_dir)
               .with_context(|| format!("create {}", vm_dir.display()))?;
           std::fs::write(&id_path, &identity.machine_identifier)
               .with_context(|| format!("write {}", id_path.display()))?;
           std::fs::write(&mac_path, &identity.mac_address)
               .with_context(|| format!("write {}", mac_path.display()))?;
           Ok(identity)
       }
   }

   #[cfg(test)]
   #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   mod tests {
       use super::*;

       fn sample() -> VmIdentity {
           VmIdentity::from_parts(vec![1, 2, 3], "02:00:00:00:00:01".to_string())
       }

       #[test]
       fn first_call_generates_and_persists() {
           let tmp = tempfile::tempdir().expect("tempdir");
           let got = VmIdentity::load_or_create_with(tmp.path(), || Ok(sample())).expect("create");
           assert_eq!(got, sample());
           assert!(tmp.path().join(MACHINE_ID_FILE).exists());
           assert!(tmp.path().join(MAC_FILE).exists());
       }

       #[test]
       fn second_call_reuses_persisted_values_without_generating() {
           let tmp = tempfile::tempdir().expect("tempdir");
           VmIdentity::load_or_create_with(tmp.path(), || Ok(sample())).expect("create");
           let again = VmIdentity::load_or_create_with(tmp.path(), || {
               anyhow::bail!("generator must not run when the identity is already persisted")
           })
           .expect("reload");
           assert_eq!(again, sample());
       }

       #[test]
       fn digest_input_changes_when_either_part_changes() {
           let a = sample();
           let b = VmIdentity::from_parts(vec![1, 2, 4], "02:00:00:00:00:01".to_string());
           let c = VmIdentity::from_parts(vec![1, 2, 3], "02:00:00:00:00:02".to_string());
           assert_ne!(a.digest_input(), b.digest_input());
           assert_ne!(a.digest_input(), c.digest_input());
       }
   }
   ```

   If `tempfile` is not a `macbox` dev-dependency, add `tempfile.workspace = true` under
   `[dev-dependencies]` (it is a workspace dependency).

3. Run the tests: expected FAIL before step 2's file exists, PASS after.
4. Add the Objective-C generator, macOS-only, in the same file (below the `impl` block, above
   `mod tests`):

   ```rust
   #[cfg(all(target_os = "macos", feature = "vz"))]
   impl VmIdentity {
       /// Load the persisted identity from `vm_dir`, generating it on first use.
       ///
       /// # Errors
       ///
       /// Returns an error if the identity files cannot be read or written.
       pub fn load_or_create(vm_dir: &Path) -> Result<Self> {
           Self::load_or_create_with(vm_dir, Self::generate)
       }

       fn generate() -> Result<Self> {
           use objc2_virtualization::{VZGenericMachineIdentifier, VZMACAddress};
           // SAFETY: `new` follows the Objective-C `+new` convention and returns a retained
           // object; `dataRepresentation` and `string` are plain getters on those objects.
           let (data, mac) = unsafe {
               let machine_id = VZGenericMachineIdentifier::new();
               let mac = VZMACAddress::randomLocallyAdministeredAddress();
               (machine_id.dataRepresentation().to_vec(), mac.string().to_string())
           };
           Ok(Self::from_parts(data, mac))
       }
   }
   ```

   Add `"objc2-virtualization/VZGenericMachineIdentifier"` and
   `"objc2-virtualization/VZMACAddress"` to the `vz` feature list in
   `crates/macbox/Cargo.toml` if they are not already enabled (the build error names the missing
   feature). `NSData::to_vec` needs no extra objc2-foundation feature beyond `NSData`; add
   `"NSData"` to the `objc2-foundation` features list if the compiler asks for it.

5. Verify: tests pass; `cargo build -p macbox --features vz` compiles on macOS.
6. Commit checkpoint: `feat(macbox): add persisted VmIdentity for restorable VM configs`.

### Task 0.2: Extract the VM configuration builder and pin identity

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/vm.rs`
**Run**: `cargo build -p macbox --features vz`

Pure refactor plus one new parameter. The existing boot path must behave exactly as before.

1. In `crates/macbox/src/vz/vm.rs`, inside `mod imp`, add before `impl VzVm`:

   ```rust
   /// Build and validate the `VZVirtualMachineConfiguration` for `config`.
   ///
   /// When `identity` is `Some`, the platform machine identifier and the NAT MAC address are
   /// pinned to it; when `None` they are random, which is the legacy boot behaviour.
   ///
   /// # Safety
   ///
   /// Must be called on the GCD main queue; every VZ object constructor used is a standard
   /// Objective-C init.
   unsafe fn build_vm_configuration(
       config: &VzVmConfig,
       identity: Option<&crate::vz::identity::VmIdentity>,
   ) -> Result<Retained<VZVirtualMachineConfiguration>> {
       // (body: move lines 248-317 of the current `prepare_on_main_queue` here unchanged,
       //  then apply the two identity edits below, and end with `Ok(vm_config)`)
   }
   ```

   Move the existing statements from `let vm_config = unsafe { VZVirtualMachineConfiguration::new() };`
   through the `validateWithError` call into this function, replacing `config.` field accesses
   with the `config` parameter as-is (it is already named `config`).

2. Apply the identity edits inside the moved body:
   - After `unsafe { vm_config.setPlatform(&platform) };` add:

     ```rust
     if let Some(identity) = identity {
         // SAFETY: initWithDataRepresentation: returns nil for invalid data, which the
         // Option check below turns into an error; `platform` is a valid generic platform.
         let machine_id = unsafe {
             objc2_virtualization::VZGenericMachineIdentifier::initWithDataRepresentation(
                 objc2_virtualization::VZGenericMachineIdentifier::alloc(),
                 &objc2_foundation::NSData::with_bytes(identity.machine_identifier()),
             )
         }
         .context("persisted machine identifier is not a valid VZGenericMachineIdentifier")?;
         // SAFETY: setMachineIdentifier: is a standard setter on the platform configuration.
         unsafe { platform.setMachineIdentifier(&machine_id) };
     }
     ```

     If `initWithDataRepresentation` is not nullable in the generated binding (it returns
     `Retained<Self>`, not `Option`), drop the `.context(...)?` and the comment about nil.

   - Before `let network_devices ...`, after `let net_dev = ...`, add:

     ```rust
     if let Some(identity) = identity {
         // SAFETY: initWithString: returns nil for a malformed address, which the Option check
         // turns into an error.
         let mac = unsafe {
             objc2_virtualization::VZMACAddress::initWithString(
                 objc2_virtualization::VZMACAddress::alloc(),
                 &NSString::from_str(identity.mac_address()),
             )
         }
         .context("persisted MAC address is not a valid VZMACAddress")?;
         // SAFETY: setMACAddress: is a standard setter on the network device configuration.
         unsafe { net_dev.setMACAddress(&mac) };
     }
     ```

     Same nullability note as above.

3. In `prepare_on_main_queue`, replace the moved statements with:

   ```rust
   let vm_config = unsafe { build_vm_configuration(&config, None) }
       .context("build VM configuration")?;
   ```

   Keep `validate_boot_mode_env()` and `load_vz_framework()` before it, and the
   `VZVirtualMachine::initWithConfiguration` and start-signal code after it, unchanged.

4. Add the `VZGenericMachineIdentifier` and `VZMACAddress` features to `Cargo.toml` as in 0.1
   if not already present.
5. Verify: `cargo build -p macbox --features vz` compiles. Behavioural check: the existing
   `vz_isolation_tests` boot path still reaches Running (run it per `DEVELOPMENT.md`; it needs the
   rebuilt guest image).
6. Commit checkpoint: `refactor(macbox): extract vm configuration builder, accept pinned identity`.

### Task 0.3: Main-queue and completion helpers

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/main_queue.rs` (new), `crates/macbox/src/vz/mod.rs`
**Run**: `cargo build -p macbox --features vz`

`crates/macbox/src/lib.rs` already inlines a `dispatch_sync_f` trampoline for the boot path.
These helpers generalize it so pause, resume, save and restore can reuse it.

1. Register the module in `crates/macbox/src/vz/mod.rs` with the same cfg as the other
   objc2-dependent modules: `pub mod main_queue;`.
2. Create `crates/macbox/src/vz/main_queue.rs`:

   ```rust
   //! Run closures on the GCD main queue and wait for VZ completion handlers.
   //!
   //! `VZVirtualMachine` calls must run on the queue the VM was created on, which is the main
   //! queue here (kept alive by `dispatch_main()` in `miniboxd`). Completion handlers also fire
   //! on the main queue, so waiting for them must happen on a different thread.

   use anyhow::{Result, bail};
   use std::sync::{Arc, Mutex};
   use std::time::{Duration, Instant};

   #[link(name = "System", kind = "dylib")]
   unsafe extern "C" {
       static _dispatch_main_q: std::ffi::c_void;
       fn dispatch_sync_f(
           queue: *const std::ffi::c_void,
           context: *mut std::ffi::c_void,
           work: unsafe extern "C" fn(*mut std::ffi::c_void),
       );
   }

   struct Job<F, R> {
       work: Option<F>,
       result: Option<R>,
   }

   unsafe extern "C" fn trampoline<F: FnOnce() -> R, R>(ctx: *mut std::ffi::c_void) {
       // SAFETY: `ctx` is the `&mut Job<F, R>` created in `run_on_main_queue`;
       // `dispatch_sync_f` guarantees it outlives this call and that this runs exactly once.
       let job = unsafe { &mut *ctx.cast::<Job<F, R>>() };
       if let Some(work) = job.work.take() {
           job.result = Some(work());
       }
   }

   /// Run `work` on the GCD main queue and return its result.
   ///
   /// Blocks the calling thread. Must **not** be called from the main queue (it would deadlock);
   /// call it from a `spawn_blocking` worker.
   ///
   /// # Errors
   ///
   /// Returns an error if the work closure did not run.
   pub fn run_on_main_queue<F, R>(work: F) -> Result<R>
   where
       F: FnOnce() -> R,
   {
       let mut job = Job {
           work: Some(work),
           result: None,
       };
       // SAFETY: `_dispatch_main_q` is the live main queue; `job` is stack-allocated and outlives
       // the synchronous `dispatch_sync_f` call, which runs `trampoline` exactly once.
       unsafe {
           dispatch_sync_f(
               &raw const _dispatch_main_q,
               (&raw mut job).cast::<std::ffi::c_void>(),
               trampoline::<F, R>,
           );
       }
       job.result
           .ok_or_else(|| anyhow::anyhow!("main-queue work did not run"))
   }

   /// Shared slot a VZ completion handler writes its outcome into.
   pub type Completion = Arc<Mutex<Option<std::result::Result<(), String>>>>;

   /// Create an empty completion slot.
   #[must_use]
   pub fn new_completion() -> Completion {
       Arc::new(Mutex::new(None))
   }

   /// Fill `slot` from an `NSError` pointer: null means success.
   ///
   /// # Safety
   ///
   /// `error` must be null or a valid `NSError` pointer for the duration of the call.
   pub unsafe fn complete(slot: &Completion, error: *mut objc2_foundation::NSError) {
       let outcome = if error.is_null() {
           Ok(())
       } else {
           // SAFETY: caller guarantees `error` is a valid NSError pointer.
           let err = unsafe { &*error };
           Err(format!(
               "{} (domain={} code={})",
               err.localizedDescription(),
               err.domain(),
               err.code()
           ))
       };
       // A poisoned mutex means a waiter panicked; there is no one left to notify.
       if let Ok(mut guard) = slot.lock() {
           *guard = Some(outcome);
       }
   }

   /// Poll `slot` until it is filled or `timeout` elapses.
   ///
   /// # Errors
   ///
   /// Returns the VZ error text, or a timeout error.
   pub fn wait_for(slot: &Completion, what: &str, timeout: Duration) -> Result<()> {
       let deadline = Instant::now() + timeout;
       loop {
           if let Ok(guard) = slot.lock() {
               if let Some(outcome) = guard.as_ref() {
                   return outcome
                       .clone()
                       .map_err(|msg| anyhow::anyhow!("{what} failed: {msg}"));
               }
           }
           if Instant::now() > deadline {
               bail!("timed out waiting for {what}");
           }
           std::thread::sleep(Duration::from_millis(50));
       }
   }
   ```

3. Verify: `cargo build -p macbox --features vz` compiles on macOS.
4. Commit checkpoint: `feat(macbox): add main-queue dispatch and completion helpers`.

### Task 0.4: `VzVm` primitives with a swappable inner machine

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/vm.rs`, `crates/macbox/src/vz/vsock.rs` (call-site fixes)
**Run**: `cargo build -p macbox --features vz`

Four adapters hold `Arc<VzVm>` clones (`VzRegistry`, `VzFilesystem`, `VzLimiter`, `VzRuntime`,
`crates/macbox/src/lib.rs` around lines 563-607). Restore must replace the machine inside the
shared `VzVm`, so the inner `VZVirtualMachine` moves behind a lock.

1. In `crates/macbox/src/vz/vm.rs` `mod imp`, change the struct:

   ```rust
   pub struct VzVm {
       vm: std::sync::RwLock<Retained<VZVirtualMachine>>,
       #[allow(dead_code)]
       config: VzVmConfig,
   }
   ```

   Update the `Self { vm, config }` in `prepare_on_main_queue` to
   `Self { vm: std::sync::RwLock::new(vm), config }`.

2. Replace `vz_vm` (and every use of `vm.vm` / `self.vm` / `vm.vm.state()` in `wait_for_running`
   and `stop`) with a cloning accessor:

   ```rust
   /// Retained handle to the current underlying machine.
   ///
   /// A restore swaps the machine, so callers must not cache this across a restore.
   pub(crate) fn vz_vm(&self) -> Retained<VZVirtualMachine> {
       match self.vm.read() {
           Ok(guard) => Retained::clone(&guard),
           Err(poisoned) => Retained::clone(&poisoned.into_inner()),
       }
   }
   ```

   Run `rg -n 'vz_vm\(\)' crates/macbox` and fix each call site (`vsock.rs`); the returned
   `Retained` derefs to `&VZVirtualMachine`, so `vm.vz_vm().socketDevices()` keeps working and
   only call sites that stored the old `&VZVirtualMachine` need a local `let machine =
vm.vz_vm();`.

3. Add the primitives to `impl VzVm`:

   ```rust
   use crate::vz::main_queue::{complete, new_completion, run_on_main_queue, wait_for};

   const OP_TIMEOUT: Duration = Duration::from_mins(2);

   /// Pause the running VM. Call from a worker thread, not the main queue.
   pub fn pause(&self) -> Result<()> {
       let slot = new_completion();
       let handler_slot = Arc::clone(&slot);
       let machine = self.vz_vm();
       run_on_main_queue(move || {
           let block = block2::RcBlock::new(move |e: *mut objc2_foundation::NSError| {
               // SAFETY: VZ passes null or a valid NSError.
               unsafe { complete(&handler_slot, e) }
           });
           // SAFETY: pauseWithCompletionHandler: runs on the VM's (main) queue here.
           unsafe { machine.pauseWithCompletionHandler(&block) };
       })?;
       wait_for(&slot, "VM pause", OP_TIMEOUT)
   }

   /// Resume a paused VM.
   pub fn resume(&self) -> Result<()> {
       let slot = new_completion();
       let handler_slot = Arc::clone(&slot);
       let machine = self.vz_vm();
       run_on_main_queue(move || {
           let block = block2::RcBlock::new(move |e: *mut objc2_foundation::NSError| {
               // SAFETY: VZ passes null or a valid NSError.
               unsafe { complete(&handler_slot, e) }
           });
           // SAFETY: resumeWithCompletionHandler: runs on the VM's (main) queue here.
           unsafe { machine.resumeWithCompletionHandler(&block) };
       })?;
       wait_for(&slot, "VM resume", OP_TIMEOUT)
   }

   /// Write the paused VM's machine state to `dest`.
   pub fn save_state(&self, dest: &Path) -> Result<()> {
       let slot = new_completion();
       let handler_slot = Arc::clone(&slot);
       let machine = self.vz_vm();
       let url = unsafe { path_to_nsurl(dest) }?;
       run_on_main_queue(move || {
           let block = block2::RcBlock::new(move |e: *mut objc2_foundation::NSError| {
               // SAFETY: VZ passes null or a valid NSError.
               unsafe { complete(&handler_slot, e) }
           });
           // SAFETY: saveMachineStateToURL:completionHandler: runs on the VM's queue and the
           // machine is paused by the caller, which the framework requires.
           unsafe { machine.saveMachineStateToURL_completionHandler(&url, &block) };
       })?;
       wait_for(&slot, "VM save", OP_TIMEOUT)
   }

   /// Stop the VM and wait until it reaches the stopped state.
   pub fn stop_and_wait(&self) -> Result<()> {
       let slot = new_completion();
       let handler_slot = Arc::clone(&slot);
       let machine = self.vz_vm();
       run_on_main_queue(move || {
           let block = block2::RcBlock::new(move |e: *mut objc2_foundation::NSError| {
               // SAFETY: VZ passes null or a valid NSError.
               unsafe { complete(&handler_slot, e) }
           });
           // SAFETY: stopWithCompletionHandler: runs on the VM's (main) queue here.
           unsafe { machine.stopWithCompletionHandler(&block) };
       })?;
       wait_for(&slot, "VM stop", OP_TIMEOUT)
   }

   /// Build an unstarted machine from `config` and the pinned `identity`.
   pub fn build_unstarted(
       config: &VzVmConfig,
       identity: &crate::vz::identity::VmIdentity,
   ) -> Result<Retained<VZVirtualMachine>> {
       let config = config.clone();
       let identity = identity.clone();
       run_on_main_queue(move || -> Result<Retained<VZVirtualMachine>> {
           // SAFETY: running on the main queue; all constructors are standard inits.
           let vm_config = unsafe { build_vm_configuration(&config, Some(&identity)) }?;
           // SAFETY: initWithConfiguration: takes a validated configuration.
           Ok(unsafe {
               VZVirtualMachine::initWithConfiguration(VZVirtualMachine::alloc(), &vm_config)
           })
       })?
   }

   /// Replace the inner machine with `fresh`, restore `source` into it, and leave it paused.
   ///
   /// The caller must have stopped the previous machine.
   pub fn restore_state(&self, fresh: Retained<VZVirtualMachine>, source: &Path) -> Result<()> {
       let slot = new_completion();
       let handler_slot = Arc::clone(&slot);
       let url = unsafe { path_to_nsurl(source) }?;
       let target = Retained::clone(&fresh);
       run_on_main_queue(move || {
           let block = block2::RcBlock::new(move |e: *mut objc2_foundation::NSError| {
               // SAFETY: VZ passes null or a valid NSError.
               unsafe { complete(&handler_slot, e) }
           });
           // SAFETY: restoreMachineStateFromURL:completionHandler: runs on the VM's queue on a
           // machine built from the same configuration and never started.
           unsafe { target.restoreMachineStateFromURL_completionHandler(&url, &block) };
       })?;
       wait_for(&slot, "VM restore", OP_TIMEOUT)?;
       match self.vm.write() {
           Ok(mut guard) => *guard = fresh,
           Err(poisoned) => *poisoned.into_inner() = fresh,
       }
       Ok(())
   }
   ```

   `stop(&self)` keeps its current behavior but reads the machine via `self.vz_vm()`.

4. Add `"objc2-virtualization/VZVirtualMachine"` to the feature list only if the compiler reports
   the methods as feature-gated; `saveMachineStateToURL_completionHandler` and
   `restoreMachineStateFromURL_completionHandler` are macOS 14+ in the binding
   (`VZVirtualMachine.rs` lines 408 and 436 of objc2-virtualization 0.3.2).
5. Verify: `cargo build -p macbox --features vz`; the existing boot path still reaches Running.
6. Commit checkpoint: `feat(macbox): add VzVm pause/resume/save/restore primitives`.

### Task 0.5: Run the spike and record the result

**Crate**: `macbox`
**File(s)**: `crates/macbox/examples/vz_save_restore_spike.rs` (new),
`docs/designs/2026-11-16-snapshot-lineage-design.md` (result section)
**Run**: see steps

This task answers four questions and **gates Phase 3**.

1. Find how the VZ entitlement is applied to `miniboxd`: `rg -n -i 'entitle|codesign' xtask/src
crates/macbox/src/vz/signing.rs`. The spike binary needs the same
   `com.apple.security.virtualization` entitlement or VM creation fails with `code=2`.
2. Create `crates/macbox/examples/vz_save_restore_spike.rs`. It runs the work on a worker thread
   and pumps the main queue with `dispatch_main()`, mirroring `miniboxd`'s `main()`
   (read `crates/miniboxd/src/main.rs` lines 226-262 and copy that structure). The worker
   performs, printing a labelled line per step:
   1. Load `VmIdentity::load_or_create(&vm_dir)`; build `VzVmConfig` from `default_vm_dir()`
      with `memory_bytes = 1 << 30`, `cpu_count = 2`, and `images_dir`/`containers_dir` set to
      two fresh directories under `vm_dir`.
   2. On the main queue, build the configuration and call
      `validateSaveRestoreSupportWithError`; print `Ok` or the `NSError` text. **Question 1.**
   3. Boot with `prepare_on_main_queue` + `wait_for_running`; wait 15 seconds.
   4. `pause`, `save_state(<tmp>/spike.vzstate)`, `stop_and_wait`.
   5. `VzVm::build_unstarted` with the same identity, `restore_state`, `resume`, then wait for
      `state() == Running`. **Question 2.**
   6. Try `vz::vsock::connect_to_agent(&vm, 5)` after the restore; print whether the agent
      answers. **Question 3.** (Needs a small tokio runtime inside the example.)
   7. Walk `images_dir`, `containers_dir` and `rootfs_path()`, summing bytes and timing a
      SHA-256 of every file; print totals. **Question 4.**
3. Build, sign and run it on macOS exactly as `miniboxd` is, with a VM image already built
   (`cargo xtask build-vm-image`). Run it with output captured to
   `.ctx/_WORKING_DIR/vz-spike.log`.
4. Record the answers in `docs/designs/2026-11-16-snapshot-lineage-design.md`, replacing the
   Phase 0 section's questions with a `### Result` block: for each of the four questions, the
   observed outcome, plus the share sizes, hash time, and whether `~/.minibox/images` is shared
   with another adapter (`rg -n 'images_dir|images' crates/macbox/src/lib.rs` and the native
   adapter's data dir).
5. **Decision point.** If question 1 or 2 fails, write `.ctx/godmode/BLOCKED-vz-snapshot.md` with
   the three attempts and the framework error, stop Phase 3, and report. If question 3 shows the
   agent connection does not survive, Task 3.4 must rebuild it (it already does); if question 4
   shows hashing is impractically slow, amend Task 3.2/3.3 to exclude the images share and record
   that in the design doc before continuing.
6. Commit checkpoint: `test(macbox): add VZ save/restore spike and record its result`.

---

# Phase 1: Path Hardening (crates `minibox-domain`, `minibox`)

Ships independently and fixes the live defect: `handle_save_snapshot` calls `create_dir_all` on a
path built from the raw request `id` before the checkpoint port is called, and
`handle_restore_snapshot` joins the raw `name` into a path.

### Task 1.1: `VmId`, `SnapshotLabel`, `SnapshotError` in `minibox-domain`

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/snapshot.rs` (new), `crates/minibox-domain/src/lib.rs`,
`crates/minibox-domain/Cargo.toml`
**Run**: `cargo nextest run -p minibox-domain snapshot::`

1. Add `miniboxs` dependency line to `crates/minibox-domain/Cargo.toml`, copying the style used
   for `thiserror` there:

   ```toml
   [dependencies.miette]
   workspace = true
   ```

2. In `crates/minibox-domain/src/lib.rs` add `pub mod snapshot;` in alphabetical position (after
   `runtime`) and an explicit re-export after the existing glob re-exports:

   ```rust
   pub use snapshot::{SnapshotError, SnapshotLabel, VmId};
   ```

   Explicit, not a glob: the old `SnapshotInfo` in `checkpoint.rs` is still re-exported until
   task 2.5.

3. Write failing tests. Create `crates/minibox-domain/src/snapshot.rs` with the tests module
   first (the crate-level `cfg_attr(test, allow(...))` already allows `expect`/`unwrap`):

   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn vm_id_accepts_slugs() {
           for ok in ["minibox-vz", "vm_1", "a", "0abc", &"a".repeat(64)] {
               assert!(VmId::parse(ok).is_ok(), "{ok} should be accepted");
           }
       }

       #[test]
       fn vm_id_rejects_traversal_and_bad_shapes() {
           for bad in [
               "", "..", "../x", "a/b", "a\\b", "/abs", "-lead", "_lead", "UPPER", "sp ace",
               "nul\0byte", "dot.dot", &"a".repeat(65),
           ] {
               assert!(VmId::parse(bad).is_err(), "{bad:?} should be rejected");
           }
       }

       #[test]
       fn vm_id_error_message_names_the_identifier() {
           let err = VmId::parse("../x").expect_err("must reject");
           assert!(err.to_string().contains("invalid vm id"), "{err}");
       }

       #[test]
       fn snapshot_label_accepts_timestamps_and_names() {
           for ok in ["20260101-120000", "before-upgrade", "v1.2_rc", "a"] {
               assert!(SnapshotLabel::parse(ok).is_ok(), "{ok} should be accepted");
           }
       }

       #[test]
       fn snapshot_label_rejects_traversal_and_bad_shapes() {
           for bad in [
               "", ".", "..", ".hidden", "a/b", "a\\b", "sp ace", "nul\0byte", &"a".repeat(65),
           ] {
               assert!(SnapshotLabel::parse(bad).is_err(), "{bad:?} should be rejected");
           }
       }

       #[test]
       fn vm_id_deserialization_enforces_validation() {
           assert!(serde_json::from_str::<VmId>("\"../x\"").is_err());
           assert!(serde_json::from_str::<VmId>("\"minibox-vz\"").is_ok());
       }
   }
   ```

   Run: `cargo nextest run -p minibox-domain snapshot::`. Expected: FAIL (types missing).

4. Implement above the tests module:

   ```rust
   //! Snapshot identity, lineage and manifest types.

   /// Errors from snapshot identifier parsing, verification and backends.
   #[derive(Debug, thiserror::Error, miette::Diagnostic)]
   pub enum SnapshotError {
       /// An identifier failed validation.
       #[error("invalid {kind}: {reason}")]
       InvalidIdentifier {
           /// What was being parsed.
           kind: &'static str,
           /// Why it was rejected.
           reason: String,
       },
       /// The adapter does not manage this VM.
       #[error("unknown vm: {0}")]
       UnknownVm(String),
       /// No such snapshot.
       #[error("snapshot not found: {0}")]
       NotFound(String),
       /// A manifest or payload failed verification.
       #[error("snapshot corrupt: {0}")]
       Corrupt(String),
       /// The named parent is missing or corrupt.
       #[error("parent snapshot missing or corrupt: {0}")]
       ParentInvalid(String),
       /// A label matched more than one snapshot.
       #[error("ambiguous snapshot label: {0}")]
       AmbiguousLabel(String),
       /// The adapter does not implement checkpointing.
       #[error("checkpoint: not supported by this adapter")]
       Unsupported,
       /// The backend failed.
       #[error("checkpoint backend failure: {0}")]
       Backend(String),
   }

   fn invalid(kind: &'static str, reason: impl ToString) -> SnapshotError {
       SnapshotError::InvalidIdentifier {
           kind,
           reason: reason.to_string(),
       }
   }

   /// Identity of a VM that owns snapshots.
   ///
   /// 1 to 64 characters of `[a-z0-9_-]`, starting with a letter or digit. The invariant holds on
   /// the construction path and on the serde path.
   #[nutype::nutype(
       validate(
           not_empty,
           len_char_max = 64,
           predicate = |s: &str| {
               s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                   && s.chars().all(|c| {
                       c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
                   })
           },
       ),
       derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Serialize, Deserialize),
   )]
   pub struct VmId(String);

   impl VmId {
       /// Parse and validate a VM identifier.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::InvalidIdentifier`] if `raw` is not a valid VM ID.
       pub fn parse(raw: &str) -> Result<Self, SnapshotError> {
           Self::try_new(raw.to_string()).map_err(|e| invalid("vm id", e))
       }

       /// The identifier as a string slice.
       #[must_use]
       pub fn as_str(&self) -> &str {
           self.as_ref()
       }
   }

   impl std::fmt::Display for VmId {
       fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
           f.write_str(self.as_str())
       }
   }

   /// Human-readable snapshot label.
   ///
   /// 1 to 64 characters of `[A-Za-z0-9._-]`, not starting with `.`. Stored in the manifest only;
   /// never used to build a path.
   #[nutype::nutype(
       validate(
           not_empty,
           len_char_max = 64,
           predicate = |s: &str| {
               !s.starts_with('.')
                   && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
           },
       ),
       derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Serialize, Deserialize),
   )]
   pub struct SnapshotLabel(String);

   impl SnapshotLabel {
       /// Parse and validate a snapshot label.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::InvalidIdentifier`] if `raw` is not a valid label.
       pub fn parse(raw: &str) -> Result<Self, SnapshotError> {
           Self::try_new(raw.to_string()).map_err(|e| invalid("snapshot label", e))
       }

       /// The label as a string slice.
       #[must_use]
       pub fn as_str(&self) -> &str {
           self.as_ref()
       }
   }

   impl std::fmt::Display for SnapshotLabel {
       fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
           f.write_str(self.as_str())
       }
   }
   ```

   nutype predicates cannot call helper functions defined in the same module reliably, which is
   why the character rules are inlined in the closures. If `nutype` rejects the multi-line block
   closure, collapse each predicate to a single expression.

5. Run: `cargo nextest run -p minibox-domain snapshot::`. Expected: PASS.
6. Commit checkpoint: `feat(minibox-domain): add validated VmId, SnapshotLabel and SnapshotError`.

### Task 1.2: Regression test for the traversal defect

**Crate**: `minibox`
**File(s)**: `crates/minibox/tests/snapshot_handler_hardening_tests.rs` (new)
**Run**: `cargo nextest run -p minibox --test snapshot_handler_hardening_tests`

1. Create the test file:

   ```rust
   #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   //! The snapshot handlers must reject unvalidated identifiers before any filesystem access.
   //!
   //! Regression for the defect where `handle_save_snapshot` called `create_dir_all` on a path
   //! built from the raw request `id` before the checkpoint port ran, so even the no-op backend
   //! created directories outside the snapshot root.

   use minibox::daemon::handler::{handle_list_snapshots, handle_restore_snapshot, handle_save_snapshot};
   use minibox::testing::helpers::{make_mock_deps, make_mock_state};
   use minibox_core::protocol::DaemonResponse;
   use tempfile::TempDir;

   const TRAVERSAL_IDS: &[&str] = &["../escape", "../../escape", "a/b", "/abs", "nul\0id", ""];

   #[tokio::test]
   async fn save_rejects_traversal_ids_without_touching_the_filesystem() {
       for bad in TRAVERSAL_IDS {
           let tmp = TempDir::new().expect("tempdir");
           let state = make_mock_state(tmp.path());
           let deps = make_mock_deps(&tmp);

           let response =
               handle_save_snapshot((*bad).to_string(), Some("snap".to_string()), state, deps).await;

           let DaemonResponse::Error { message } = &response else {
               panic!("{bad:?} must be rejected, got {response:?}");
           };
           assert!(message.starts_with("save_snapshot"), "{message}");
           assert!(message.contains("invalid vm id"), "{message}");
           assert!(
               !tmp.path().join("escape").exists(),
               "{bad:?} created a directory outside the snapshot root"
           );
       }
   }

   #[tokio::test]
   async fn save_rejects_traversal_labels() {
       for bad in ["../x", "a/b", ".hidden", ""] {
           let tmp = TempDir::new().expect("tempdir");
           let state = make_mock_state(tmp.path());
           let deps = make_mock_deps(&tmp);

           let response =
               handle_save_snapshot("ctr-1".to_string(), Some(bad.to_string()), state, deps).await;

           let DaemonResponse::Error { message } = &response else {
               panic!("label {bad:?} must be rejected, got {response:?}");
           };
           assert!(message.contains("invalid snapshot label"), "{message}");
       }
   }

   #[tokio::test]
   async fn restore_rejects_traversal_ids_and_labels() {
       let tmp = TempDir::new().expect("tempdir");
       let state = make_mock_state(tmp.path());

       for (id, name) in [("../escape", "snap"), ("ctr-1", "../x"), ("ctr-1", "a/b")] {
           let deps = make_mock_deps(&tmp);
           let response = handle_restore_snapshot(
               id.to_string(),
               name.to_string(),
               std::sync::Arc::clone(&state),
               deps,
           )
           .await;
           let DaemonResponse::Error { message } = &response else {
               panic!("({id:?}, {name:?}) must be rejected, got {response:?}");
           };
           assert!(message.starts_with("restore_snapshot"), "{message}");
           assert!(message.contains("invalid"), "{message}");
       }
   }

   #[tokio::test]
   async fn list_rejects_traversal_ids() {
       let tmp = TempDir::new().expect("tempdir");
       let deps = make_mock_deps(&tmp);
       let response = handle_list_snapshots("../escape".to_string(), deps).await;
       let DaemonResponse::Error { message } = &response else {
           panic!("traversal id must be rejected, got {response:?}");
       };
       assert!(message.starts_with("list_snapshots"), "{message}");
       assert!(message.contains("invalid vm id"), "{message}");
   }
   ```

2. Run: expected FAIL, in particular `save_rejects_traversal_ids...` (the escape directory is
   created today) and the `invalid vm id` message assertions.
3. Commit checkpoint is combined with 1.3 (a failing test must not be committed alone).

### Task 1.3: Validate in the handlers

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handler/snapshot.rs`
**Run**: `cargo nextest run -p minibox --test snapshot_handler_hardening_tests` and
`cargo nextest run -p miniboxd --test checkpoint_wiring_tests`

1. Replace the body of `crates/minibox/src/daemon/handler/snapshot.rs` (keep the TODO comment
   for feature-idea-29; task 2.5 removes it):

   ```rust
   //! VM state snapshot handlers: save, restore, list.

   use chrono::Utc;
   use minibox_core::domain::{SnapshotLabel, VmId};
   use minibox_core::protocol::DaemonResponse;
   use std::path::PathBuf;
   use std::sync::Arc;

   use crate::daemon::state::DaemonState;

   use super::HandlerDependencies;

   // TODO(feature-idea-29): resolve snapshot paths relative to a trusted directory handle with
   // openat2 containment (or an openat/O_NOFOLLOW fallback) before enabling real checkpoint backends.
   // Phase 1 of docs/designs/2026-11-16-snapshot-lineage-design.md validates identifiers before
   // any path is built; phase 2 removes path construction from the handler entirely.

   fn error(op: &str, detail: impl std::fmt::Display) -> DaemonResponse {
       DaemonResponse::Error {
           message: format!("{op}: {detail}"),
       }
   }

   fn snapshot_dir(deps: &HandlerDependencies, vm: &VmId) -> PathBuf {
       let data_dir = &deps.lifecycle.containers_base;
       data_dir
           .parent()
           .unwrap_or(data_dir)
           .join("snapshots")
           .join(vm.as_str())
   }

   /// Save a VM state snapshot for a container.
   pub async fn handle_save_snapshot(
       id: String,
       name: Option<String>,
       _state: Arc<DaemonState>,
       deps: Arc<HandlerDependencies>,
   ) -> DaemonResponse {
       let vm = match VmId::parse(&id) {
           Ok(vm) => vm,
           Err(e) => return error("save_snapshot", e),
       };
       let raw_label = name.unwrap_or_else(|| Utc::now().format("%Y%m%d-%H%M%S").to_string());
       let label = match SnapshotLabel::parse(&raw_label) {
           Ok(label) => label,
           Err(e) => return error("save_snapshot", e),
       };

       let snap_dir = snapshot_dir(&deps, &vm);
       if let Err(e) = std::fs::create_dir_all(&snap_dir) {
           return error("save_snapshot", format!("failed to create snapshot dir: {e}"));
       }

       let snap_path = snap_dir.join(format!("{}.snap", label.as_str()));
       match deps.checkpoint.save_snapshot(vm.as_str(), &snap_path) {
           Ok(info) => DaemonResponse::SnapshotSaved { info },
           Err(e) => error("save_snapshot", e),
       }
   }

   /// Restore a VM state snapshot for a container.
   pub async fn handle_restore_snapshot(
       id: String,
       name: String,
       _state: Arc<DaemonState>,
       deps: Arc<HandlerDependencies>,
   ) -> DaemonResponse {
       let vm = match VmId::parse(&id) {
           Ok(vm) => vm,
           Err(e) => return error("restore_snapshot", e),
       };
       let label = match SnapshotLabel::parse(&name) {
           Ok(label) => label,
           Err(e) => return error("restore_snapshot", e),
       };

       let snap_path = snapshot_dir(&deps, &vm).join(format!("{}.snap", label.as_str()));
       match deps.checkpoint.restore_snapshot(vm.as_str(), &snap_path) {
           Ok(()) => DaemonResponse::SnapshotRestored { id, name },
           Err(e) => error("restore_snapshot", e),
       }
   }

   /// List available snapshots for a container.
   pub async fn handle_list_snapshots(id: String, deps: Arc<HandlerDependencies>) -> DaemonResponse {
       let vm = match VmId::parse(&id) {
           Ok(vm) => vm,
           Err(e) => return error("list_snapshots", e),
       };
       match deps.checkpoint.list_snapshots(vm.as_str()) {
           Ok(snapshots) => DaemonResponse::SnapshotList { id, snapshots },
           Err(e) => error("list_snapshots", e),
       }
   }
   ```

   The current port returns `anyhow::Result`, so `{e}` formatting is unchanged. The existing
   operation-name prefixes (`save_snapshot`, `restore_snapshot`, `list_snapshots`) are preserved,
   which `checkpoint_wiring_tests.rs` asserts.

2. Run the new tests: expected PASS. Run
   `cargo nextest run -p minibox --test daemon_handler_coverage_tests --test daemon_handler_error_coverage_b4b`
   and `cargo nextest run -p miniboxd --test checkpoint_wiring_tests`: expected PASS. Test ids in
   those files (`container-id`, `ctr-1`, `some-container-b4b`) are valid `VmId`s.
   A container id with uppercase letters is now rejected by these handlers; no backend ever
   produced a snapshot, so no live snapshot is affected.
3. Commit checkpoint: `fix(minibox): validate snapshot identifiers before building paths`.
   (Includes 1.1's files if not yet committed separately; do not commit the failing test alone.)

---

# Phase 2: Lineage Foundation (crates `minibox-domain`, `minibox`, `minibox-core`, `mbx`, `minibox-testsuite`)

Tasks 2.1 - 2.4 are additive and leave the tree compiling. Task 2.5 is the cutover.

### Task 2.1: `ShareTag`, `SnapshotId`, `PayloadKind`, `PayloadEntry`

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/snapshot.rs`
**Run**: `cargo nextest run -p minibox-domain snapshot::`

Append to the module. `snapshot` is **not** glob re-exported yet, so the new `SnapshotInfo` added in
2.2 does not clash with the old one in `checkpoint.rs`; import via
`minibox_core::domain::snapshot::...` until task 2.5.

1. Add failing tests to the `tests` module:

   ```rust
   const HEX64: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

   #[test]
   fn snapshot_id_round_trips_and_exposes_hex() {
       let raw = format!("sha256:{HEX64}");
       let id = SnapshotId::parse(&raw).expect("valid id");
       assert_eq!(id.as_str(), raw);
       assert_eq!(id.hex(), HEX64);
       assert_eq!(serde_json::to_string(&id).expect("ser"), format!("\"{raw}\""));
   }

   #[test]
   fn snapshot_id_rejects_malformed_values() {
       for bad in [
           "",
           "sha256:",
           "sha256:abc",
           &format!("sha1:{HEX64}"),
           &format!("sha256:{}", HEX64.to_uppercase()),
           &format!("sha256:{HEX64}0"),
           "sha256:../../../../../../../../../../../../../../../../../../../../../../../x",
           &format!("sha256:{}/", &HEX64[..63]),
       ] {
           assert!(SnapshotId::parse(bad).is_err(), "{bad:?} should be rejected");
       }
       assert!(serde_json::from_str::<SnapshotId>("\"../x\"").is_err());
   }

   #[test]
   fn snapshot_id_from_digest_matches_parse() {
       let id = SnapshotId::from_digest([0xab; 32]);
       assert_eq!(SnapshotId::parse(id.as_str()).expect("parse"), id);
   }

   #[test]
   fn share_tag_rejects_traversal() {
       for bad in ["", "..", "../x", "a/b", "UPPER", ".x"] {
           assert!(ShareTag::parse(bad).is_err(), "{bad:?}");
       }
       assert!(ShareTag::parse("minibox-rootfs").is_ok());
   }

   #[test]
   fn payload_kind_paths_stay_relative() {
       assert_eq!(
           PayloadKind::MachineState.relative_path(),
           std::path::PathBuf::from("state.vzstate")
       );
       let tag = ShareTag::parse("minibox-images").expect("tag");
       assert_eq!(
           PayloadKind::ShareClone { tag }.relative_path(),
           std::path::PathBuf::from("clones/minibox-images")
       );
   }

   #[test]
   fn payload_kind_deserialization_rejects_traversing_tags() {
       let bad = r#"{"kind":"share_clone","tag":"../escape"}"#;
       assert!(serde_json::from_str::<PayloadKind>(bad).is_err());
   }
   ```

2. Run: expected FAIL. Implement (append above `mod tests`):

   ```rust
   /// virtiofs share tag naming a share clone. Same alphabet as [`VmId`].
   #[nutype::nutype(
       validate(
           not_empty,
           len_char_max = 64,
           predicate = |s: &str| {
               s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                   && s.chars().all(|c| {
                       c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
                   })
           },
       ),
       derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Serialize, Deserialize),
   )]
   pub struct ShareTag(String);

   impl ShareTag {
       /// Parse and validate a share tag.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::InvalidIdentifier`] if `raw` is not a valid tag.
       pub fn parse(raw: &str) -> Result<Self, SnapshotError> {
           Self::try_new(raw.to_string()).map_err(|e| invalid("share tag", e))
       }

       /// The tag as a string slice.
       #[must_use]
       pub fn as_str(&self) -> &str {
           self.as_ref()
       }
   }

   /// Content address of a snapshot manifest: `sha256:<64 lowercase hex>`.
   #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
   #[serde(try_from = "String", into = "String")]
   pub struct SnapshotId(String);

   impl SnapshotId {
       const PREFIX: &'static str = "sha256:";

       /// Parse and validate a snapshot ID.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::InvalidIdentifier`] unless `raw` is `sha256:` followed by
       /// exactly 64 lowercase hex characters.
       pub fn parse(raw: &str) -> Result<Self, SnapshotError> {
           let Some(digest) = raw.strip_prefix(Self::PREFIX) else {
               return Err(invalid("snapshot id", "must start with `sha256:`"));
           };
           let is_lower_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
           if digest.len() != 64 || !digest.bytes().all(is_lower_hex) {
               return Err(invalid(
                   "snapshot id",
                   "digest must be 64 lowercase hex characters",
               ));
           }
           Ok(Self(raw.to_string()))
       }

       /// Build an ID from a raw SHA-256 digest.
       #[must_use]
       pub fn from_digest(digest: [u8; 32]) -> Self {
           Self(format!("{}{}", Self::PREFIX, hex::encode(digest)))
       }

       /// The 64-character hex digest, which is also the snapshot directory name.
       #[must_use]
       pub fn hex(&self) -> &str {
           &self.0[Self::PREFIX.len()..]
       }

       /// The full `sha256:<hex>` form.
       #[must_use]
       pub fn as_str(&self) -> &str {
           &self.0
       }
   }

   impl TryFrom<String> for SnapshotId {
       type Error = SnapshotError;

       fn try_from(value: String) -> Result<Self, Self::Error> {
           Self::parse(&value)
       }
   }

   impl From<SnapshotId> for String {
       fn from(id: SnapshotId) -> Self {
           id.0
       }
   }

   impl std::fmt::Display for SnapshotId {
       fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
           f.write_str(&self.0)
       }
   }

   /// What a payload is. Determines its location inside the snapshot directory.
   #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
   #[serde(tag = "kind", rename_all = "snake_case")]
   pub enum PayloadKind {
       /// Machine state file (`state.vzstate`).
       MachineState,
       /// Copy-on-write clone of a virtiofs share (`clones/<tag>`).
       ShareClone {
           /// Share tag.
           tag: ShareTag,
       },
   }

   impl PayloadKind {
       /// Path relative to the snapshot directory. Never contains `..` or an absolute component.
       #[must_use]
       pub fn relative_path(&self) -> std::path::PathBuf {
           match self {
               Self::MachineState => std::path::PathBuf::from("state.vzstate"),
               Self::ShareClone { tag } => std::path::Path::new("clones").join(tag.as_str()),
           }
       }
   }

   /// Length and digest of one payload.
   #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
   pub struct PayloadEntry {
       /// What the payload is.
       pub kind: PayloadKind,
       /// File length in bytes; for a clone, the total of all file lengths.
       pub len: u64,
       /// SHA-256 hex of the file; for a clone, the tree digest over sorted
       /// (relative path, mode, length, file SHA-256) records.
       pub sha256: String,
   }
   ```

3. Run: expected PASS.
4. Commit checkpoint: `feat(minibox-domain): add SnapshotId, ShareTag and payload types`.

### Task 2.2: `SnapshotManifest` and the new `SnapshotInfo`

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/snapshot.rs`
**Run**: `cargo nextest run -p minibox-domain snapshot::`

1. Failing tests:

   ```rust
   fn sample_manifest() -> SnapshotManifest {
       SnapshotManifest {
           schema_version: SnapshotManifest::SCHEMA_VERSION,
           vm_id: VmId::parse("minibox-vz").expect("vm"),
           parent: None,
           created_at: "2026-11-16T00:00:00Z".to_string(),
           adapter: "vz".to_string(),
           label: Some(SnapshotLabel::parse("before-upgrade").expect("label")),
           vm_config_digest: "0".repeat(64),
           payloads: vec![
               PayloadEntry {
                   kind: PayloadKind::ShareClone {
                       tag: ShareTag::parse("minibox-rootfs").expect("tag"),
                   },
                   len: 10,
                   sha256: "1".repeat(64),
               },
               PayloadEntry {
                   kind: PayloadKind::MachineState,
                   len: 20,
                   sha256: "2".repeat(64),
               },
           ],
       }
   }

   #[test]
   fn manifest_id_is_stable_and_order_independent() {
       let a = sample_manifest();
       let mut b = sample_manifest();
       b.payloads.reverse();
       assert_eq!(a.id().expect("id"), a.id().expect("id"));
       assert_eq!(a.id().expect("id"), b.id().expect("id"));
   }

   #[test]
   fn manifest_id_changes_when_any_field_changes() {
       let base = sample_manifest().id().expect("id");

       let mut m = sample_manifest();
       m.created_at = "2026-11-16T00:00:01Z".to_string();
       assert_ne!(m.id().expect("id"), base);

       let mut m = sample_manifest();
       m.parent = Some(SnapshotId::from_digest([7; 32]));
       assert_ne!(m.id().expect("id"), base);

       let mut m = sample_manifest();
       m.payloads[0].sha256 = "9".repeat(64);
       assert_ne!(m.id().expect("id"), base);

       let mut m = sample_manifest();
       m.label = None;
       assert_ne!(m.id().expect("id"), base);
   }

   #[test]
   fn manifest_size_sums_payload_lengths() {
       assert_eq!(sample_manifest().size_bytes(), 30);
   }

   #[test]
   fn snapshot_info_round_trips_through_json() {
       let manifest = sample_manifest();
       let info = SnapshotInfo {
           id: manifest.id().expect("id"),
           manifest,
       };
       let json = serde_json::to_string(&info).expect("ser");
       let back: SnapshotInfo = serde_json::from_str(&json).expect("de");
       assert_eq!(back, info);
   }
   ```

2. Run: expected FAIL. Implement:

   ```rust
   /// Immutable record whose canonical bytes determine the [`SnapshotId`].
   #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
   pub struct SnapshotManifest {
       /// Manifest schema version.
       pub schema_version: u32,
       /// VM the snapshot belongs to.
       pub vm_id: VmId,
       /// Parent snapshot, if this snapshot continues a lineage.
       pub parent: Option<SnapshotId>,
       /// RFC 3339 creation timestamp.
       pub created_at: String,
       /// Adapter that produced the snapshot, e.g. `vz`.
       pub adapter: String,
       /// Optional human-readable label.
       pub label: Option<SnapshotLabel>,
       /// SHA-256 hex over the pinned VM configuration.
       pub vm_config_digest: String,
       /// Payload lengths and digests.
       pub payloads: Vec<PayloadEntry>,
   }

   impl SnapshotManifest {
       /// Current manifest schema version.
       pub const SCHEMA_VERSION: u32 = 1;

       /// Deterministic serialization: struct field order, payloads sorted by relative path,
       /// no insignificant whitespace.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::Corrupt`] if serialization fails.
       pub fn canonical_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
           let mut canonical = self.clone();
           canonical.payloads.sort_by_key(|p| p.kind.relative_path());
           serde_json::to_vec(&canonical)
               .map_err(|e| SnapshotError::Corrupt(format!("manifest serialization: {e}")))
       }

       /// The content address of this manifest.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::Corrupt`] if serialization fails.
       pub fn id(&self) -> Result<SnapshotId, SnapshotError> {
           use sha2::{Digest, Sha256};
           let digest: [u8; 32] = Sha256::digest(self.canonical_bytes()?).into();
           Ok(SnapshotId::from_digest(digest))
       }

       /// Total payload size in bytes.
       #[must_use]
       pub fn size_bytes(&self) -> u64 {
           self.payloads.iter().map(|p| p.len).sum()
       }
   }

   /// A published snapshot: the manifest and its derived ID.
   #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
   pub struct SnapshotInfo {
       /// Content address of `manifest`.
       pub id: SnapshotId,
       /// The immutable manifest.
       pub manifest: SnapshotManifest,
   }
   ```

3. Run: expected PASS.
4. Commit checkpoint: `feat(minibox-domain): add SnapshotManifest and content-addressed SnapshotInfo`.

### Task 2.3: `SnapshotStore`: open, begin, publish, load

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/adapters/snapshot_store.rs` (new),
`crates/minibox/src/adapters/mod.rs`, `crates/minibox/tests/snapshot_store_tests.rs` (new)
**Run**: `cargo nextest run -p minibox --test snapshot_store_tests`

1. Register: add `pub mod snapshot_store;` to `crates/minibox/src/adapters/mod.rs` with **no**
   `#[cfg]` gate (check the file: several adapters are Linux-gated; this one must not be).
2. Failing tests first. Create `crates/minibox/tests/snapshot_store_tests.rs`:

   ```rust
   #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   //! `SnapshotStore` publishes content-addressed snapshots root-last and verifies on read.

   use minibox::adapters::snapshot_store::{SnapshotDraft, SnapshotStore};
   use minibox_core::domain::snapshot::{
       PayloadKind, ShareTag, SnapshotError, SnapshotId, SnapshotInfo, SnapshotLabel, VmId,
   };
   use std::path::Path;
   use tempfile::TempDir;

   fn vm() -> VmId {
       VmId::parse("minibox-vz").expect("vm id")
   }

   fn rootfs_kind() -> PayloadKind {
       PayloadKind::ShareClone {
           tag: ShareTag::parse("minibox-rootfs").expect("tag"),
       }
   }

   fn draft(parent: Option<SnapshotId>, label: Option<&str>) -> SnapshotDraft {
       SnapshotDraft {
           vm_id: vm(),
           parent,
           label: label.map(|l| SnapshotLabel::parse(l).expect("label")),
           adapter: "test".to_string(),
           vm_config_digest: "0".repeat(64),
           payloads: vec![PayloadKind::MachineState, rootfs_kind()],
       }
   }

   /// Write the two payloads a `draft` names into `staging`.
   fn write_payloads(staging: &Path, state: &[u8]) {
       std::fs::write(staging.join("state.vzstate"), state).expect("state");
       let clone = staging.join("clones").join("minibox-rootfs");
       std::fs::create_dir_all(clone.join("etc")).expect("clone dir");
       std::fs::write(clone.join("etc").join("hostname"), b"minibox\n").expect("file");
   }

   fn publish(store: &SnapshotStore, parent: Option<SnapshotId>, label: Option<&str>, state: &[u8]) -> SnapshotInfo {
       let staging = store.begin(&vm()).expect("begin");
       write_payloads(staging.path(), state);
       store.publish(staging, draft(parent, label)).expect("publish")
   }

   fn open(tmp: &TempDir) -> SnapshotStore {
       SnapshotStore::open(&tmp.path().join("snapshots")).expect("open")
   }

   fn snapshot_dir(tmp: &TempDir, id: &SnapshotId) -> std::path::PathBuf {
       tmp.path().join("snapshots").join("minibox-vz").join(id.hex())
   }

   #[test]
   fn publish_then_load_round_trips() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let info = publish(&store, None, Some("first"), b"state-1");

       assert_eq!(info.manifest.id().expect("id"), info.id);
       assert_eq!(info.manifest.payloads.len(), 2);
       assert!(snapshot_dir(&tmp, &info.id).join("manifest.json").is_file());

       let loaded = store.load(&vm(), &info.id).expect("load");
       assert_eq!(loaded.info(), &info);
       assert!(loaded.payload_path(&PayloadKind::MachineState).is_file());
   }

   #[test]
   fn tampered_state_byte_fails_closed() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let info = publish(&store, None, None, b"state-1");
       std::fs::write(snapshot_dir(&tmp, &info.id).join("state.vzstate"), b"state-X").expect("tamper");
       let err = store.load(&vm(), &info.id).err().expect("must fail");
       assert!(matches!(err, SnapshotError::Corrupt(_)), "{err:?}");
   }

   #[test]
   fn truncated_state_fails_closed() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let info = publish(&store, None, None, b"state-1");
       std::fs::write(snapshot_dir(&tmp, &info.id).join("state.vzstate"), b"st").expect("truncate");
       assert!(matches!(
           store.load(&vm(), &info.id).err().expect("must fail"),
           SnapshotError::Corrupt(_)
       ));
   }

   #[test]
   fn tampered_clone_file_fails_closed() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let info = publish(&store, None, None, b"state-1");
       let file = snapshot_dir(&tmp, &info.id).join("clones/minibox-rootfs/etc/hostname");
       std::fs::write(file, b"evil\n").expect("tamper");
       assert!(matches!(
           store.load(&vm(), &info.id).err().expect("must fail"),
           SnapshotError::Corrupt(_)
       ));
   }

   #[test]
   fn edited_manifest_fails_closed() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let info = publish(&store, None, Some("first"), b"state-1");
       let path = snapshot_dir(&tmp, &info.id).join("manifest.json");
       let edited = std::fs::read_to_string(&path).expect("read").replace("first", "evil1");
       std::fs::write(&path, edited).expect("write");
       assert!(matches!(
           store.load(&vm(), &info.id).err().expect("must fail"),
           SnapshotError::Corrupt(_)
       ));
   }

   #[test]
   fn renamed_directory_fails_closed() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let a = publish(&store, None, None, b"state-1");
       let b = publish(&store, None, None, b"state-2");
       let (dir_a, dir_b) = (snapshot_dir(&tmp, &a.id), snapshot_dir(&tmp, &b.id));
       std::fs::remove_dir_all(&dir_b).expect("rm b");
       std::fs::rename(&dir_a, &dir_b).expect("swap a into b's name");
       assert!(matches!(
           store.load(&vm(), &b.id).err().expect("must fail"),
           SnapshotError::Corrupt(_)
       ));
   }

   #[test]
   fn load_unknown_snapshot_is_not_found() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let missing = SnapshotId::from_digest([9; 32]);
       assert!(matches!(
           store.load(&vm(), &missing).err().expect("must fail"),
           SnapshotError::NotFound(_)
       ));
   }

   #[test]
   fn symlinked_snapshot_directory_is_refused() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let real = publish(&store, None, None, b"state-1");
       let alias = SnapshotId::from_digest([5; 32]);
       std::os::unix::fs::symlink(snapshot_dir(&tmp, &real.id), snapshot_dir(&tmp, &alias))
           .expect("symlink");
       assert!(store.load(&vm(), &alias).is_err());
   }

   #[test]
   fn child_snapshot_records_and_verifies_its_parent() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let parent = publish(&store, None, Some("base"), b"state-1");
       let child = publish(&store, Some(parent.id.clone()), Some("next"), b"state-2");
       assert_eq!(child.manifest.parent.as_ref(), Some(&parent.id));
       store.load(&vm(), &child.id).expect("child verifies with its chain");
   }

   #[test]
   fn publish_rejects_a_missing_parent() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let staging = store.begin(&vm()).expect("begin");
       write_payloads(staging.path(), b"state-1");
       let missing = SnapshotId::from_digest([3; 32]);
       let err = store.publish(staging, draft(Some(missing), None)).err().expect("must fail");
       assert!(matches!(err, SnapshotError::ParentInvalid(_)), "{err:?}");
   }

   #[test]
   fn corrupt_parent_makes_child_load_fail() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let parent = publish(&store, None, None, b"state-1");
       let child = publish(&store, Some(parent.id.clone()), None, b"state-2");
       std::fs::write(snapshot_dir(&tmp, &parent.id).join("state.vzstate"), b"xx").expect("tamper");
       assert!(store.load(&vm(), &child.id).is_err());
   }

   #[test]
   fn dropped_staging_publishes_nothing() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       {
           let staging = store.begin(&vm()).expect("begin");
           write_payloads(staging.path(), b"state-1");
           // dropped without publish: simulates a failed save
       }
       assert!(store.list(&vm()).expect("list").is_empty());
       let staging_root = tmp.path().join("snapshots/minibox-vz/.staging");
       assert_eq!(std::fs::read_dir(staging_root).expect("staging dir").count(), 0);
   }

   #[test]
   fn open_sweeps_staging_left_by_a_crash() {
       let tmp = TempDir::new().expect("tmp");
       let root = tmp.path().join("snapshots");
       let stale = root.join("minibox-vz/.staging/leftover");
       std::fs::create_dir_all(&stale).expect("stale");
       std::fs::write(stale.join("state.vzstate"), b"half").expect("write");
       let _store = SnapshotStore::open(&root).expect("open");
       assert!(!stale.exists());
   }

   #[test]
   fn publish_at_with_identical_inputs_is_idempotent() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let when = "2026-11-16T00:00:00Z";

       let first = {
           let staging = store.begin(&vm()).expect("begin");
           write_payloads(staging.path(), b"state-1");
           store.publish_at(staging, draft(None, Some("x")), when).expect("first")
       };
       let second = {
           let staging = store.begin(&vm()).expect("begin");
           write_payloads(staging.path(), b"state-1");
           store.publish_at(staging, draft(None, Some("x")), when).expect("second")
       };
       assert_eq!(first, second);
       assert_eq!(store.list(&vm()).expect("list").len(), 1);
   }

   #[test]
   fn list_returns_published_snapshots_oldest_first() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let a = publish(&store, None, Some("a"), b"state-1");
       let b = publish(&store, Some(a.id.clone()), Some("b"), b"state-2");
       let listed = store.list(&vm()).expect("list");
       let ids: Vec<_> = listed.iter().map(|s| s.id.clone()).collect();
       assert_eq!(ids, vec![a.id, b.id]);
   }

   #[test]
   fn list_of_unknown_vm_is_empty_and_corrupt_entry_is_an_error() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       assert!(store.list(&vm()).expect("empty").is_empty());

       let info = publish(&store, None, None, b"state-1");
       std::fs::write(snapshot_dir(&tmp, &info.id).join("state.vzstate"), b"zz").expect("tamper");
       assert!(store.list(&vm()).is_err(), "a corrupt snapshot must not be silently skipped");
   }
   ```

   `list` uses a monotonic order: sort by `(manifest.created_at, id)`. The test above publishes
   within the same second, so `created_at` must carry sub-second precision (below).

3. Run: expected FAIL (module missing).
4. Implement `crates/minibox/src/adapters/snapshot_store.rs`:

   ```rust
   //! Content-addressed, root-last snapshot storage.
   //!
   //! Layout under `<root>/<vm_id>/`: in-progress saves live in `.staging/<random>/` and are never
   //! listed; a published snapshot is the directory `<64-hex>/` holding `manifest.json` and its
   //! payloads. The directory name is the SHA-256 of the manifest bytes, so a snapshot cannot be
   //! altered, renamed or re-parented without failing verification.

   use miette::{IntoDiagnostic, WrapErr};
   use minibox_core::domain::snapshot::{
       PayloadEntry, PayloadKind, SnapshotError, SnapshotId, SnapshotInfo, SnapshotLabel,
       SnapshotManifest, VmId,
   };
   use sha2::{Digest, Sha256};
   use std::io::Read;
   use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
   use std::path::{Path, PathBuf};

   const MANIFEST_FILE: &str = "manifest.json";
   const STAGING_DIR: &str = ".staging";
   const MAX_LINEAGE_DEPTH: usize = 10_000;

   fn backend(context: &str, e: impl std::fmt::Display) -> SnapshotError {
       SnapshotError::Backend(format!("{context}: {e}"))
   }

   /// What a caller supplies to publish a snapshot; payloads must already be in staging.
   #[derive(Debug, Clone)]
   pub struct SnapshotDraft {
       /// VM the snapshot belongs to.
       pub vm_id: VmId,
       /// Parent snapshot, if any.
       pub parent: Option<SnapshotId>,
       /// Optional label.
       pub label: Option<SnapshotLabel>,
       /// Producing adapter.
       pub adapter: String,
       /// SHA-256 hex over the pinned VM configuration.
       pub vm_config_digest: String,
       /// Payloads already written into the staging directory.
       pub payloads: Vec<PayloadKind>,
   }

   /// An in-progress save. Dropping it without publishing removes the staging directory.
   #[derive(Debug)]
   pub struct StagingDir {
       path: PathBuf,
       vm_dir: PathBuf,
       vm_id: VmId,
       published: bool,
   }

   impl StagingDir {
       /// The staging directory the caller writes payloads into.
       #[must_use]
       pub fn path(&self) -> &Path {
           &self.path
       }
   }

   impl Drop for StagingDir {
       fn drop(&mut self) {
           if !self.published {
               if let Err(e) = std::fs::remove_dir_all(&self.path) {
                   tracing::warn!(
                       path = %self.path.display(),
                       error = %e,
                       "snapshot: failed to remove abandoned staging directory"
                   );
               }
           }
       }
   }

   /// A snapshot whose manifest, payloads and parent chain were verified.
   #[derive(Debug)]
   pub struct VerifiedSnapshot {
       info: SnapshotInfo,
       dir: PathBuf,
   }

   impl VerifiedSnapshot {
       /// The verified manifest and ID.
       #[must_use]
       pub const fn info(&self) -> &SnapshotInfo {
           &self.info
       }

       /// Absolute path of a payload inside the verified snapshot directory.
       #[must_use]
       pub fn payload_path(&self, kind: &PayloadKind) -> PathBuf {
           self.dir.join(kind.relative_path())
       }
   }

   /// Filesystem-backed snapshot store.
   #[derive(Debug, Clone)]
   pub struct SnapshotStore {
       root: PathBuf,
   }

   impl SnapshotStore {
       /// Create the root (`0700`), canonicalize it once, and remove stale staging entries.
       ///
       /// # Errors
       ///
       /// Returns an error if the root cannot be created or canonicalized.
       pub fn open(root: &Path) -> miette::Result<Self> {
           std::fs::DirBuilder::new()
               .recursive(true)
               .mode(0o700)
               .create(root)
               .into_diagnostic()
               .wrap_err_with(|| format!("create snapshot root {}", root.display()))?;
           let root = std::fs::canonicalize(root)
               .into_diagnostic()
               .wrap_err_with(|| format!("canonicalize snapshot root {}", root.display()))?;
           let store = Self { root };
           store.sweep_staging();
           Ok(store)
       }

       fn sweep_staging(&self) {
           let Ok(vms) = std::fs::read_dir(&self.root) else {
               return;
           };
           for vm in vms.filter_map(Result::ok) {
               let staging = vm.path().join(STAGING_DIR);
               if staging.is_dir() {
                   if let Err(e) = std::fs::remove_dir_all(&staging) {
                       tracing::warn!(
                           path = %staging.display(),
                           error = %e,
                           "snapshot: failed to sweep stale staging directory"
                       );
                   }
               }
           }
       }

       fn vm_dir(&self, vm: &VmId) -> PathBuf {
           self.root.join(vm.as_str())
       }

       /// Start a save: create `.staging/<random>/` under the VM directory.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::Backend`] if the directory cannot be created.
       pub fn begin(&self, vm: &VmId) -> Result<StagingDir, SnapshotError> {
           let vm_dir = self.vm_dir(vm);
           let staging_root = vm_dir.join(STAGING_DIR);
           std::fs::DirBuilder::new()
               .recursive(true)
               .mode(0o700)
               .create(&staging_root)
               .map_err(|e| backend("create staging root", e))?;
           let path = staging_root.join(uuid::Uuid::new_v4().simple().to_string());
           std::fs::DirBuilder::new()
               .mode(0o700)
               .create(&path)
               .map_err(|e| backend("create staging dir", e))?;
           Ok(StagingDir {
               path,
               vm_dir,
               vm_id: vm.clone(),
               published: false,
           })
       }

       /// Publish `staging` as a new snapshot stamped with the current time.
       ///
       /// # Errors
       ///
       /// See [`Self::publish_at`].
       pub fn publish(
           &self,
           staging: StagingDir,
           draft: SnapshotDraft,
       ) -> Result<SnapshotInfo, SnapshotError> {
           let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
           self.publish_at(staging, draft, &now)
       }

       /// Publish `staging` with an explicit creation timestamp (deterministic for tests).
       ///
       /// Hashes every payload, verifies the parent, writes `manifest.json` last, fsyncs, and
       /// atomically renames the staging directory to the snapshot ID. If a snapshot with the
       /// same ID already exists the call succeeds and the staging directory is discarded.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::ParentInvalid`] if the parent does not verify, and
       /// [`SnapshotError::Backend`] / [`SnapshotError::Corrupt`] for I/O or hashing failures.
       pub fn publish_at(
           &self,
           mut staging: StagingDir,
           draft: SnapshotDraft,
           created_at: &str,
       ) -> Result<SnapshotInfo, SnapshotError> {
           if draft.vm_id != staging.vm_id {
               return Err(SnapshotError::Backend(
                   "draft vm_id does not match the staging directory".to_string(),
               ));
           }
           if let Some(parent) = &draft.parent {
               self.verify_chain(&draft.vm_id, parent)
                   .map_err(|e| SnapshotError::ParentInvalid(format!("{parent}: {e}")))?;
           }

           let mut payloads = Vec::with_capacity(draft.payloads.len());
           for kind in &draft.payloads {
               let entry = hash_payload(kind, &staging.path.join(kind.relative_path()))?;
               payloads.push(entry);
           }
           let manifest = SnapshotManifest {
               schema_version: SnapshotManifest::SCHEMA_VERSION,
               vm_id: draft.vm_id.clone(),
               parent: draft.parent,
               created_at: created_at.to_string(),
               adapter: draft.adapter,
               label: draft.label,
               vm_config_digest: draft.vm_config_digest,
               payloads,
           };
           let bytes = manifest.canonical_bytes()?;
           let id = manifest.id()?;

           let manifest_path = staging.path.join(MANIFEST_FILE);
           std::fs::write(&manifest_path, &bytes).map_err(|e| backend("write manifest", e))?;
           sync_tree(&staging.path)?;
           sync_path(&staging.path)?;

           let target = staging.vm_dir.join(id.hex());
           match std::fs::rename(&staging.path, &target) {
               Ok(()) => {
                   staging.published = true;
                   sync_path(&staging.vm_dir)?;
               }
               Err(_) if target.is_dir() => {
                   // Identical snapshot already published; the staging copy is discarded on drop.
                   tracing::debug!(snapshot_id = %id, "snapshot: identical snapshot already published");
               }
               Err(e) => return Err(backend("publish snapshot", e)),
           }
           Ok(SnapshotInfo { id, manifest })
       }

       /// Load and fully verify a snapshot.
       ///
       /// # Errors
       ///
       /// Returns [`SnapshotError::NotFound`] if the snapshot is absent and
       /// [`SnapshotError::Corrupt`] if the manifest, any payload, or the parent chain fails
       /// verification.
       pub fn load(&self, vm: &VmId, id: &SnapshotId) -> Result<VerifiedSnapshot, SnapshotError> {
           let dir = self.vm_dir(vm).join(id.hex());
           let (info, dir) = self.read_manifest(vm, id, &dir)?;
           for entry in &info.manifest.payloads {
               let actual = hash_payload(&entry.kind, &dir.join(entry.kind.relative_path()))?;
               if &actual != entry {
                   return Err(SnapshotError::Corrupt(format!(
                       "{id}: payload {:?} does not match its manifest",
                       entry.kind
                   )));
               }
           }
           if let Some(parent) = &info.manifest.parent {
               self.verify_chain(vm, parent)
                   .map_err(|e| SnapshotError::Corrupt(format!("{id}: parent {parent}: {e}")))?;
           }
           Ok(VerifiedSnapshot { info, dir })
       }

       /// Every verified snapshot of `vm`, oldest first. A corrupt entry is an error.
       ///
       /// # Errors
       ///
       /// Returns the first verification failure, or [`SnapshotError::Backend`] on I/O errors.
       pub fn list(&self, vm: &VmId) -> Result<Vec<SnapshotInfo>, SnapshotError> {
           let vm_dir = self.vm_dir(vm);
           let entries = match std::fs::read_dir(&vm_dir) {
               Ok(entries) => entries,
               Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
               Err(e) => return Err(backend("read vm snapshot dir", e)),
           };
           let mut out = Vec::new();
           for entry in entries {
               let entry = entry.map_err(|e| backend("read vm snapshot dir entry", e))?;
               let name = entry.file_name().to_string_lossy().into_owned();
               if name == STAGING_DIR {
                   continue;
               }
               let id = SnapshotId::parse(&format!("sha256:{name}")).map_err(|_| {
                   SnapshotError::Corrupt(format!("unexpected entry in snapshot dir: {name}"))
               })?;
               out.push(self.load(vm, &id)?.info);
           }
           out.sort_by(|a, b| {
               (&a.manifest.created_at, a.id.as_str()).cmp(&(&b.manifest.created_at, b.id.as_str()))
           });
           Ok(out)
       }

       /// Read and verify one manifest (hash equals directory name, canonical, same VM).
       fn read_manifest(
           &self,
           vm: &VmId,
           id: &SnapshotId,
           dir: &Path,
       ) -> Result<(SnapshotInfo, PathBuf), SnapshotError> {
           open_dir_nofollow(dir).map_err(|e| match e.kind() {
               std::io::ErrorKind::NotFound => SnapshotError::NotFound(id.to_string()),
               _ => SnapshotError::Corrupt(format!("{id}: cannot open snapshot directory: {e}")),
           })?;
           let bytes = std::fs::read(dir.join(MANIFEST_FILE))
               .map_err(|e| SnapshotError::Corrupt(format!("{id}: read manifest: {e}")))?;
           let digest: [u8; 32] = Sha256::digest(&bytes).into();
           if SnapshotId::from_digest(digest) != *id {
               return Err(SnapshotError::Corrupt(format!(
                   "{id}: manifest hash does not match the snapshot ID"
               )));
           }
           let manifest: SnapshotManifest = serde_json::from_slice(&bytes)
               .map_err(|e| SnapshotError::Corrupt(format!("{id}: manifest parse: {e}")))?;
           if manifest.canonical_bytes()? != bytes {
               return Err(SnapshotError::Corrupt(format!("{id}: manifest is not canonical")));
           }
           if manifest.schema_version != SnapshotManifest::SCHEMA_VERSION {
               return Err(SnapshotError::Corrupt(format!(
                   "{id}: unsupported manifest schema version {}",
                   manifest.schema_version
               )));
           }
           if &manifest.vm_id != vm {
               return Err(SnapshotError::Corrupt(format!(
                   "{id}: manifest belongs to vm {}, not {vm}",
                   manifest.vm_id
               )));
           }
           Ok((
               SnapshotInfo {
                   id: id.clone(),
                   manifest,
               },
               dir.to_path_buf(),
           ))
       }

       /// Verify `start` and every ancestor: manifest hash, canonical form, same VM.
       fn verify_chain(&self, vm: &VmId, start: &SnapshotId) -> Result<(), SnapshotError> {
           let mut next = Some(start.clone());
           for _ in 0..MAX_LINEAGE_DEPTH {
               let Some(id) = next else {
                   return Ok(());
               };
               let dir = self.vm_dir(vm).join(id.hex());
               let (info, _) = self.read_manifest(vm, &id, &dir)?;
               next = info.manifest.parent;
           }
           Err(SnapshotError::Corrupt("lineage deeper than the supported maximum".to_string()))
       }
   }

   /// Open `dir` refusing symlinks, so a swapped link cannot redirect a read.
   fn open_dir_nofollow(dir: &Path) -> std::io::Result<std::fs::File> {
       std::fs::OpenOptions::new()
           .read(true)
           .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_DIRECTORY)
           .open(dir)
   }

   fn sync_path(path: &Path) -> Result<(), SnapshotError> {
       std::fs::File::open(path)
           .and_then(|f| f.sync_all())
           .map_err(|e| backend("fsync", e))
   }

   fn sync_tree(root: &Path) -> Result<(), SnapshotError> {
       for entry in walkdir::WalkDir::new(root).follow_links(false) {
           let entry = entry.map_err(|e| backend("walk staging", e))?;
           if entry.file_type().is_file() {
               sync_path(entry.path())?;
           }
       }
       Ok(())
   }

   fn hash_payload(kind: &PayloadKind, path: &Path) -> Result<PayloadEntry, SnapshotError> {
       let (len, sha256) = match kind {
           PayloadKind::MachineState => hash_file(path)?,
           PayloadKind::ShareClone { .. } => hash_tree(path)?,
       };
       Ok(PayloadEntry {
           kind: kind.clone(),
           len,
           sha256,
       })
   }

   fn hash_file(path: &Path) -> Result<(u64, String), SnapshotError> {
       let meta = std::fs::symlink_metadata(path)
           .map_err(|e| SnapshotError::Corrupt(format!("{}: {e}", path.display())))?;
       if !meta.is_file() {
           return Err(SnapshotError::Corrupt(format!("{} is not a regular file", path.display())));
       }
       let mut file = std::fs::File::open(path)
           .map_err(|e| SnapshotError::Corrupt(format!("{}: {e}", path.display())))?;
       let mut hasher = Sha256::new();
       let mut buf = vec![0_u8; 1 << 20];
       let mut total = 0_u64;
       loop {
           let n = file
               .read(&mut buf)
               .map_err(|e| SnapshotError::Corrupt(format!("{}: {e}", path.display())))?;
           if n == 0 {
               break;
           }
           hasher.update(&buf[..n]);
           total += n as u64;
       }
       Ok((total, hex::encode(hasher.finalize())))
   }

   /// Digest of a directory tree: sorted records of (kind, relative path, mode, length, hash).
   fn hash_tree(root: &Path) -> Result<(u64, String), SnapshotError> {
       let meta = std::fs::symlink_metadata(root)
           .map_err(|e| SnapshotError::Corrupt(format!("{}: {e}", root.display())))?;
       if !meta.is_dir() {
           return Err(SnapshotError::Corrupt(format!("{} is not a directory", root.display())));
       }
       let mut hasher = Sha256::new();
       let mut total = 0_u64;
       let walker = walkdir::WalkDir::new(root)
           .follow_links(false)
           .min_depth(1)
           .sort_by_file_name();
       for entry in walker {
           let entry = entry.map_err(|e| SnapshotError::Corrupt(format!("walk {}: {e}", root.display())))?;
           let rel = entry
               .path()
               .strip_prefix(root)
               .map_err(|e| SnapshotError::Corrupt(format!("strip prefix: {e}")))?;
           let meta = entry
               .metadata()
               .map_err(|e| SnapshotError::Corrupt(format!("{}: {e}", entry.path().display())))?;
           let mode = meta.permissions().mode() & 0o7777;
           let ft = entry.file_type();
           let record = if ft.is_dir() {
               format!("d\0{}\0{mode:o}\n", rel.display())
           } else if ft.is_file() {
               let (len, sha) = hash_file(entry.path())?;
               total += len;
               format!("f\0{}\0{mode:o}\0{len}\0{sha}\n", rel.display())
           } else if ft.is_symlink() {
               let target = std::fs::read_link(entry.path())
                   .map_err(|e| SnapshotError::Corrupt(format!("{}: {e}", entry.path().display())))?;
               format!("l\0{}\0{}\n", rel.display(), target.display())
           } else {
               return Err(SnapshotError::Corrupt(format!(
                   "unsupported file type in clone: {}",
                   entry.path().display()
               )));
           };
           hasher.update(record.as_bytes());
           let _ = meta.ino();
       }
       Ok((total, hex::encode(hasher.finalize())))
   }
   ```

   Remove the stray `let _ = meta.ino();` line and the `MetadataExt` import (they are left over
   from an inode-based variant and must not ship); everything else is final.

5. Run: `cargo nextest run -p minibox --test snapshot_store_tests`. Expected: PASS.
6. Commit checkpoint: `feat(minibox): add content-addressed SnapshotStore`.

### Task 2.4: Domain test for the lineage invariants used by handlers

**Crate**: `minibox`
**File(s)**: `crates/minibox/tests/snapshot_store_tests.rs`
**Run**: `cargo nextest run -p minibox --test snapshot_store_tests`

1. Add a test that the manifest read from disk with a traversing `tag` is refused at the type
   level (defense for the `ShareTag` refinement):

   ```rust
   #[test]
   fn manifest_with_a_traversing_clone_tag_cannot_be_loaded() {
       let tmp = TempDir::new().expect("tmp");
       let store = open(&tmp);
       let info = publish(&store, None, None, b"state-1");
       let path = snapshot_dir(&tmp, &info.id).join("manifest.json");
       let edited = std::fs::read_to_string(&path)
           .expect("read")
           .replace("minibox-rootfs", "../../escape");
       std::fs::write(&path, edited).expect("write");
       // Hash mismatch or tag rejection: either way, load must fail closed.
       assert!(store.load(&vm(), &info.id).is_err());
   }
   ```

2. Run: expected PASS (the hash check fails first; the tag validation is the second line of
   defense and is covered by `payload_kind_deserialization_rejects_traversing_tags` in the domain
   crate).
3. Commit checkpoint: `test(minibox): cover traversing clone tag in a tampered manifest`.

### Task 2.5: Cutover: new port, protocol, handlers, CLI, mocks, conformance (one compile-clean step)

**Crates**: `minibox-domain`, `minibox-core`, `minibox`, `mbx`, `minibox-testsuite`, `miniboxd`
(tests only)
**File(s)**: listed per step
**Run**: `cargo check --workspace --all-targets` is not a gate, but it is the fastest way to find
the next site in this step; the commit is gated by `cargo xtask pre-commit`.

This cannot be split: replacing the `VmCheckpoint` signatures breaks every consumer until all of
them change. Do the steps in order and do not commit until the workspace compiles and its tests
pass.

**A. Domain: port, no-op, re-export** (`crates/minibox-domain/src/checkpoint.rs`,
`crates/minibox-domain/src/lib.rs`)

1. In `crates/minibox-domain/src/lib.rs` replace
   `pub use snapshot::{SnapshotError, SnapshotLabel, VmId};` with `pub use snapshot::*;`.
2. Rewrite `crates/minibox-domain/src/checkpoint.rs` (this removes the old `SnapshotInfo` and
   `TODO(feature-idea-31)`):

   ```rust
   //! VM checkpoint port definitions.

   use crate::snapshot::{SnapshotError, SnapshotId, SnapshotInfo, SnapshotLabel, VmId};
   use std::sync::Arc;

   /// Port for saving and restoring VM state checkpoints.
   ///
   /// Snapshots are content-addressed and immutable: the adapter owns storage, and callers refer
   /// to snapshots only by [`SnapshotId`]. No caller-supplied path reaches an adapter. Only the
   /// macOS `vz` adapter implements the port; every other wiring site installs
   /// [`NoopVmCheckpoint`], so snapshot requests fail with an explicit "not supported" error
   /// rather than reporting state that was never persisted. An adapter that gains real
   /// checkpointing implements this trait and declares the `Checkpoint` capability in the same
   /// change.
   pub trait VmCheckpoint: Send + Sync {
       /// Capture VM state as a new immutable snapshot, optionally as a child of `parent`.
       ///
       /// # Errors
       ///
       /// Returns an error if checkpointing is unsupported, `vm` is unknown, `parent` does not
       /// verify, or the backend fails.
       fn save_snapshot(
           &self,
           vm: &VmId,
           parent: Option<&SnapshotId>,
           label: Option<&SnapshotLabel>,
       ) -> Result<SnapshotInfo, SnapshotError>;

       /// Roll the VM back to `snapshot`. The snapshot is verified before the VM is touched.
       ///
       /// # Errors
       ///
       /// Returns an error if the snapshot is missing or corrupt, or the restore fails.
       fn restore_snapshot(&self, vm: &VmId, snapshot: &SnapshotId) -> Result<(), SnapshotError>;

       /// All verified snapshots of `vm`, oldest first.
       ///
       /// # Errors
       ///
       /// Returns an error if listing fails or any snapshot is corrupt.
       fn list_snapshots(&self, vm: &VmId) -> Result<Vec<SnapshotInfo>, SnapshotError>;
   }

   /// Type alias for a shared, dynamic [`VmCheckpoint`] implementation.
   pub type DynVmCheckpoint = Arc<dyn VmCheckpoint>;

   /// A no-op [`VmCheckpoint`] that always returns [`SnapshotError::Unsupported`].
   ///
   /// This is the production default for every adapter that does not implement checkpointing.
   /// All three methods fail with the same reason and none touch the filesystem, so a failed
   /// request leaves no snapshot behind. `list_snapshots` also errors rather than returning an
   /// empty vector: an empty list would assert that no snapshots exist, which this adapter cannot
   /// know.
   pub struct NoopVmCheckpoint;

   impl VmCheckpoint for NoopVmCheckpoint {
       fn save_snapshot(
           &self,
           _vm: &VmId,
           _parent: Option<&SnapshotId>,
           _label: Option<&SnapshotLabel>,
       ) -> Result<SnapshotInfo, SnapshotError> {
           Err(SnapshotError::Unsupported)
       }

       fn restore_snapshot(&self, _vm: &VmId, _snapshot: &SnapshotId) -> Result<(), SnapshotError> {
           Err(SnapshotError::Unsupported)
       }

       fn list_snapshots(&self, _vm: &VmId) -> Result<Vec<SnapshotInfo>, SnapshotError> {
           Err(SnapshotError::Unsupported)
       }
   }
   ```

   The error text `checkpoint: not supported by this adapter` is unchanged, so the existing
   "not supported" assertions keep matching.

**B. Protocol** (`crates/minibox-core/src/protocol.rs`)

3. Change `SaveSnapshot` (around line 604):

   ```rust
   /// Save a VM state snapshot.
   SaveSnapshot {
       /// VM ID to snapshot.
       id: String,
       /// Optional human-readable snapshot label.
       #[serde(default)]
       name: Option<String>,
       /// Optional parent snapshot ID (`sha256:<hex>`) to record lineage.
       #[serde(default)]
       parent: Option<String>,
   },

   /// Restore a VM state snapshot.
   RestoreSnapshot {
       /// VM ID to restore.
       id: String,
       /// Snapshot reference: a snapshot ID (`sha256:<hex>`), or a label that matches exactly
       /// one snapshot.
       name: String,
   },

   /// List available snapshots for a VM.
   ListSnapshots {
       /// VM ID.
       id: String,
   },
   ```

   Update the doc comments on `SnapshotRestored` (`name` is the restored snapshot ID) and
   `SnapshotList`/`SnapshotSaved`. Response variants are otherwise unchanged, so
   `is_terminal()` is unaffected.

4. Replace `sample_snapshot_info()` (around line 2714) with:

   ```rust
   fn sample_snapshot_info() -> crate::domain::SnapshotInfo {
       use crate::domain::{PayloadEntry, PayloadKind, SnapshotInfo, SnapshotManifest, VmId};
       let manifest = SnapshotManifest {
           schema_version: SnapshotManifest::SCHEMA_VERSION,
           vm_id: VmId::parse("abc").expect("vm id"),
           parent: None,
           created_at: "2026-01-01T00:00:00Z".to_string(),
           adapter: "native".to_string(),
           label: None,
           vm_config_digest: "0".repeat(64),
           payloads: vec![PayloadEntry {
               kind: PayloadKind::MachineState,
               len: 0,
               sha256: "0".repeat(64),
           }],
       };
       let id = manifest.id().expect("manifest id");
       SnapshotInfo { id, manifest }
   }
   ```

   Find every other construction/match of the changed request with
   `rg -n 'SaveSnapshot \{' crates xtask --glob '*.rs'` and add `parent: None` to every
   constructor (patterns using `{ id, name }` become `{ id, name, parent }`; patterns using
   `{ .. }` need no change; `crates/mcp/src/client.rs:242` uses `{ .. }`).

**C. Handlers and dispatch** (`crates/minibox/src/daemon/handler/snapshot.rs`,
`crates/minibox/src/daemon/server.rs`)

5. Replace `crates/minibox/src/daemon/handler/snapshot.rs` entirely (this removes
   `TODO(feature-idea-29)` and all path construction):

   ```rust
   //! VM state snapshot handlers: save, restore, list.
   //!
   //! Identifiers are parsed into validated types before anything else happens, and the
   //! checkpoint adapter owns storage, so no request string is ever joined into a path here.

   use minibox_core::domain::{
       DynVmCheckpoint, SnapshotError, SnapshotId, SnapshotLabel, VmCheckpoint, VmId,
   };
   use minibox_core::protocol::DaemonResponse;
   use std::sync::Arc;

   use crate::daemon::state::DaemonState;

   use super::HandlerDependencies;

   fn error(op: &str, detail: impl std::fmt::Display) -> DaemonResponse {
       DaemonResponse::Error {
           message: format!("{op}: {detail}"),
       }
   }

   async fn blocking<T: Send + 'static>(
       op: &'static str,
       work: impl FnOnce() -> Result<T, SnapshotError> + Send + 'static,
   ) -> Result<T, DaemonResponse> {
       match tokio::task::spawn_blocking(work).await {
           Ok(Ok(value)) => Ok(value),
           Ok(Err(e)) => Err(error(op, e)),
           Err(e) => Err(error(op, format!("task failed: {e}"))),
       }
   }

   /// Resolve a snapshot reference: a snapshot ID, or a label matching exactly one snapshot.
   fn resolve_snapshot_ref(
       checkpoint: &dyn VmCheckpoint,
       vm: &VmId,
       reference: &str,
   ) -> Result<SnapshotId, SnapshotError> {
       if let Ok(id) = SnapshotId::parse(reference) {
           return Ok(id);
       }
       let label = SnapshotLabel::parse(reference)?;
       let mut matches = checkpoint
           .list_snapshots(vm)?
           .into_iter()
           .filter(|s| s.manifest.label.as_ref() == Some(&label));
       match (matches.next(), matches.next()) {
           (Some(found), None) => Ok(found.id),
           (None, _) => Err(SnapshotError::NotFound(reference.to_string())),
           (Some(_), Some(_)) => Err(SnapshotError::AmbiguousLabel(reference.to_string())),
       }
   }

   /// Save a VM state snapshot.
   pub async fn handle_save_snapshot(
       id: String,
       name: Option<String>,
       parent: Option<String>,
       _state: Arc<DaemonState>,
       deps: Arc<HandlerDependencies>,
   ) -> DaemonResponse {
       let parsed = (|| -> Result<_, SnapshotError> {
           let vm = VmId::parse(&id)?;
           let label = name.as_deref().map(SnapshotLabel::parse).transpose()?;
           let parent = parent.as_deref().map(SnapshotId::parse).transpose()?;
           Ok((vm, label, parent))
       })();
       let (vm, label, parent) = match parsed {
           Ok(parsed) => parsed,
           Err(e) => return error("save_snapshot", e),
       };

       let checkpoint: DynVmCheckpoint = Arc::clone(&deps.checkpoint);
       match blocking("save_snapshot", move || {
           checkpoint.save_snapshot(&vm, parent.as_ref(), label.as_ref())
       })
       .await
       {
           Ok(info) => DaemonResponse::SnapshotSaved { info },
           Err(response) => response,
       }
   }

   /// Restore a VM state snapshot.
   pub async fn handle_restore_snapshot(
       id: String,
       name: String,
       _state: Arc<DaemonState>,
       deps: Arc<HandlerDependencies>,
   ) -> DaemonResponse {
       let vm = match VmId::parse(&id) {
           Ok(vm) => vm,
           Err(e) => return error("restore_snapshot", e),
       };

       let checkpoint: DynVmCheckpoint = Arc::clone(&deps.checkpoint);
       let restored = blocking("restore_snapshot", move || {
           let snapshot = resolve_snapshot_ref(checkpoint.as_ref(), &vm, &name)?;
           checkpoint.restore_snapshot(&vm, &snapshot)?;
           Ok(snapshot)
       })
       .await;
       match restored {
           Ok(snapshot) => DaemonResponse::SnapshotRestored {
               id,
               name: snapshot.to_string(),
           },
           Err(response) => response,
       }
   }

   /// List available snapshots for a VM.
   pub async fn handle_list_snapshots(id: String, deps: Arc<HandlerDependencies>) -> DaemonResponse {
       let vm = match VmId::parse(&id) {
           Ok(vm) => vm,
           Err(e) => return error("list_snapshots", e),
       };
       let checkpoint: DynVmCheckpoint = Arc::clone(&deps.checkpoint);
       match blocking("list_snapshots", move || checkpoint.list_snapshots(&vm)).await {
           Ok(snapshots) => DaemonResponse::SnapshotList { id, snapshots },
           Err(response) => response,
       }
   }
   ```

6. In `crates/minibox/src/daemon/server.rs` (around line 723) change the save dispatch:

   ```rust
   DaemonRequest::SaveSnapshot { id, name, parent } => {
       let response = handler::handle_save_snapshot(id, name, parent, state, deps).await;
       send_terminal_response(&tx, "SaveSnapshot", response).await;
   }
   ```

**D. Mock** (`crates/minibox/src/testing/mocks/vm_checkpoint.rs`)

7. Replace the file:

   ```rust
   //! Mock [`VmCheckpoint`] for conformance testing.
   #![allow(clippy::expect_used, clippy::unwrap_used)]

   use minibox_core::domain::{
       SnapshotError, SnapshotId, SnapshotInfo, SnapshotLabel, SnapshotManifest, VmCheckpoint, VmId,
   };
   use std::sync::Mutex;
   use std::sync::atomic::{AtomicU64, Ordering};

   /// Mock VM checkpoint that stores snapshots in memory.
   ///
   /// Snapshots are keyed by VM ID and accumulated across calls. Each save gets a distinct
   /// timestamp so repeated saves of the same VM produce distinct content addresses.
   #[derive(Debug)]
   pub struct MockVmCheckpoint {
       snapshots: Mutex<Vec<SnapshotInfo>>,
       counter: AtomicU64,
   }

   impl MockVmCheckpoint {
       /// Create a fresh mock with no snapshots.
       pub const fn new() -> Self {
           Self {
               snapshots: Mutex::new(Vec::new()),
               counter: AtomicU64::new(0),
           }
       }

       /// Total number of snapshots stored across all VM IDs.
       pub fn snapshot_count(&self) -> usize {
           self.snapshots.lock().expect("lock").len()
       }
   }

   impl Default for MockVmCheckpoint {
       fn default() -> Self {
           Self::new()
       }
   }

   impl VmCheckpoint for MockVmCheckpoint {
       fn save_snapshot(
           &self,
           vm: &VmId,
           parent: Option<&SnapshotId>,
           label: Option<&SnapshotLabel>,
       ) -> Result<SnapshotInfo, SnapshotError> {
           let mut snaps = self.snapshots.lock().expect("lock");
           if let Some(parent) = parent {
               if !snaps.iter().any(|s| &s.id == parent && &s.manifest.vm_id == vm) {
                   return Err(SnapshotError::ParentInvalid(parent.to_string()));
               }
           }
           let n = self.counter.fetch_add(1, Ordering::SeqCst);
           let manifest = SnapshotManifest {
               schema_version: SnapshotManifest::SCHEMA_VERSION,
               vm_id: vm.clone(),
               parent: parent.cloned(),
               created_at: format!("2026-01-01T00:00:00.{n:09}Z"),
               adapter: "mock".to_string(),
               label: label.cloned(),
               vm_config_digest: "0".repeat(64),
               payloads: Vec::new(),
           };
           let info = SnapshotInfo {
               id: manifest.id()?,
               manifest,
           };
           snaps.push(info.clone());
           Ok(info)
       }

       fn restore_snapshot(&self, vm: &VmId, snapshot: &SnapshotId) -> Result<(), SnapshotError> {
           let snaps = self.snapshots.lock().expect("lock");
           if snaps.iter().any(|s| &s.id == snapshot && &s.manifest.vm_id == vm) {
               Ok(())
           } else {
               Err(SnapshotError::NotFound(snapshot.to_string()))
           }
       }

       fn list_snapshots(&self, vm: &VmId) -> Result<Vec<SnapshotInfo>, SnapshotError> {
           let snaps = self.snapshots.lock().expect("lock");
           Ok(snaps
               .iter()
               .filter(|s| &s.manifest.vm_id == vm)
               .cloned()
               .collect())
       }
   }

   #[cfg(test)]
   mod tests {
       use super::*;

       fn vm(s: &str) -> VmId {
           VmId::parse(s).expect("vm id")
       }

       #[test]
       fn save_and_list_snapshot() {
           let mock = MockVmCheckpoint::new();
           mock.save_snapshot(&vm("ctr-1"), None, None).expect("save");
           let list = mock.list_snapshots(&vm("ctr-1")).expect("list");
           assert_eq!(list.len(), 1);
           assert_eq!(list[0].manifest.vm_id, vm("ctr-1"));
       }

       #[test]
       fn restore_fails_when_no_snapshot() {
           let mock = MockVmCheckpoint::new();
           let missing = SnapshotId::from_digest([1; 32]);
           assert!(mock.restore_snapshot(&vm("ctr-999"), &missing).is_err());
       }

       #[test]
       fn list_returns_empty_for_unknown_vm() {
           let mock = MockVmCheckpoint::new();
           assert!(mock.list_snapshots(&vm("unknown")).expect("list").is_empty());
       }

       #[test]
       fn repeated_saves_get_distinct_ids() {
           let mock = MockVmCheckpoint::new();
           let a = mock.save_snapshot(&vm("ctr-1"), None, None).expect("a");
           let b = mock.save_snapshot(&vm("ctr-1"), None, None).expect("b");
           assert_ne!(a.id, b.id);
       }

       #[test]
       fn child_requires_an_existing_parent() {
           let mock = MockVmCheckpoint::new();
           let bogus = SnapshotId::from_digest([2; 32]);
           assert!(mock.save_snapshot(&vm("ctr-1"), Some(&bogus), None).is_err());
           let parent = mock.save_snapshot(&vm("ctr-1"), None, None).expect("parent");
           let child = mock
               .save_snapshot(&vm("ctr-1"), Some(&parent.id), None)
               .expect("child");
           assert_eq!(child.manifest.parent, Some(parent.id));
       }
   }
   ```

**E. CLI** (`crates/mbx/src/commands/snapshot.rs`, `crates/mbx/src/main.rs`)

8. In `crates/mbx/src/main.rs` (around line 534) replace `SnapshotCommands`:

   ```rust
   /// Snapshot sub-subcommands.
   #[derive(Subcommand)]
   enum SnapshotCommands {
       /// Save a VM state snapshot.
       Save {
           /// VM ID.
           id: String,
           /// Snapshot label.
           #[arg(long)]
           name: Option<String>,
           /// Parent snapshot ID (`sha256:<hex>`) to continue a lineage.
           #[arg(long)]
           parent: Option<String>,
       },
       /// Restore a VM state snapshot.
       Restore {
           /// VM ID.
           id: String,
           /// Snapshot ID (`sha256:<hex>`) or a label that matches exactly one snapshot.
           name: String,
       },
       /// List available snapshots.
       List {
           /// VM ID.
           id: String,
       },
   }
   ```

   and the dispatch (around line 809):

   ```rust
   SnapshotCommands::Save { id, name, parent } => {
       into_cli(commands::snapshot::execute_save(id, name, parent, socket_path).await)
   }
   ```

9. In `crates/mbx/src/commands/snapshot.rs`: change `execute_save` to take `parent:
Option<String>` after `name`, send `DaemonRequest::SaveSnapshot { id: id.clone(), name, parent
}`, and print the new shape. Replace the three response arms' printing:

   ```rust
   DaemonResponse::SnapshotSaved { info } => {
       match &info.manifest.label {
           Some(label) => println!(
               "Snapshot saved: {} (label {label}, vm {})",
               info.id, info.manifest.vm_id
           ),
           None => println!("Snapshot saved: {} (vm {})", info.id, info.manifest.vm_id),
       }
       Ok(())
   }
   ```

   ```rust
   DaemonResponse::SnapshotRestored { id, name } => {
       println!("Snapshot restored: {name} for vm {id}");
       Ok(())
   }
   ```

   ```rust
   DaemonResponse::SnapshotList { id, snapshots } => {
       if snapshots.is_empty() {
           println!("No snapshots for vm {id}");
       } else {
           println!("Snapshots for vm {id}:");
           for s in &snapshots {
               let label = s.manifest.label.as_ref().map_or("-", |l| l.as_str());
               let parent = s.manifest.parent.as_ref().map_or_else(
                   || "-".to_string(),
                   |p| p.hex()[..12].to_string(),
               );
               println!(
                   "  {} label={label} parent={parent} created={} adapter={} size={} bytes",
                   &s.id.hex()[..12],
                   s.manifest.created_at,
                   s.manifest.adapter,
                   s.manifest.size_bytes()
               );
           }
       }
       Ok(())
   }
   ```

**F. Tests and conformance migration**

10. `crates/minibox-testsuite/src/adapters/vm_checkpoint.rs`: replace the file with the same six
    conformance cases ported to the new signatures plus lineage cases:

    ```rust
    //! Conformance tests for the [`VmCheckpoint`] trait contract.
    //!
    //! All tests use `MockVmCheckpoint` — no real VM state is touched.

    use minibox::testing::mocks::vm_checkpoint::MockVmCheckpoint;
    use minibox_core::domain::{SnapshotId, VmCheckpoint, VmId};

    fn vm(s: &str) -> VmId {
        VmId::parse(s).expect("vm id")
    }

    crate::conformance_test! {
        name: "save_snapshot_returns_info",
        adapter: "vm_checkpoint",
        capability: Checkpoint,
        category: Unit,
        |ctx| {
            let mock = MockVmCheckpoint::new();
            let result = mock.save_snapshot(&vm("vm-snap-001"), None, None);
            if let Some(info) = ctx.assert_ok(result, "save_snapshot should succeed") {
                ctx.assert_eq(vm("vm-snap-001"), info.manifest.vm_id.clone(), "snapshot vm_id must match");
                ctx.assert_eq(
                    Some(info.id.clone()),
                    info.manifest.id().ok(),
                    "snapshot id must be the content address of its manifest",
                );
            }
            ctx.result()
        }
    }

    crate::conformance_test! {
        name: "save_snapshot_increments_count",
        adapter: "vm_checkpoint",
        capability: Checkpoint,
        category: Unit,
        |ctx| {
            let mock = MockVmCheckpoint::new();
            mock.save_snapshot(&vm("vm-snap-002"), None, None).expect("save");
            ctx.assert_eq(1, mock.snapshot_count(), "snapshot_count after one save");
            mock.save_snapshot(&vm("vm-snap-002"), None, None).expect("save");
            ctx.assert_eq(2, mock.snapshot_count(), "snapshot_count after two saves");
            ctx.result()
        }
    }

    crate::conformance_test! {
        name: "list_snapshots_filters_by_vm_id",
        adapter: "vm_checkpoint",
        capability: Checkpoint,
        category: Unit,
        |ctx| {
            let mock = MockVmCheckpoint::new();
            mock.save_snapshot(&vm("vm-a"), None, None).expect("save a");
            mock.save_snapshot(&vm("vm-b"), None, None).expect("save b");
            let list = mock.list_snapshots(&vm("vm-a")).expect("list");
            ctx.assert_eq(1, list.len(), "list_snapshots should return only vm-a snapshots");
            ctx.assert_eq(vm("vm-a"), list[0].manifest.vm_id.clone(), "vm_id in list");
            ctx.result()
        }
    }

    crate::conformance_test! {
        name: "list_snapshots_empty_for_unknown",
        adapter: "vm_checkpoint",
        capability: Checkpoint,
        category: EdgeCase,
        |ctx| {
            let mock = MockVmCheckpoint::new();
            let list = mock.list_snapshots(&vm("no-such-vm")).expect("list");
            ctx.assert_eq(0, list.len(), "list_snapshots must be empty for unknown vm");
            ctx.result()
        }
    }

    crate::conformance_test! {
        name: "restore_snapshot_succeeds_after_save",
        adapter: "vm_checkpoint",
        capability: Checkpoint,
        category: Unit,
        |ctx| {
            let mock = MockVmCheckpoint::new();
            let info = mock.save_snapshot(&vm("vm-snap-003"), None, None).expect("save");
            let result = mock.restore_snapshot(&vm("vm-snap-003"), &info.id);
            ctx.assert_ok(result, "restore_snapshot should succeed after save");
            ctx.result()
        }
    }

    crate::conformance_test! {
        name: "restore_snapshot_fails_without_save",
        adapter: "vm_checkpoint",
        capability: Checkpoint,
        category: EdgeCase,
        |ctx| {
            let mock = MockVmCheckpoint::new();
            let missing = SnapshotId::from_digest([4; 32]);
            let result = mock.restore_snapshot(&vm("nonexistent"), &missing);
            ctx.assert_err(result, "restore_snapshot without prior save must return Err");
            ctx.result()
        }
    }

    crate::conformance_test! {
        name: "save_snapshot_records_parent_lineage",
        adapter: "vm_checkpoint",
        capability: Checkpoint,
        category: Unit,
        |ctx| {
            let mock = MockVmCheckpoint::new();
            let parent = mock.save_snapshot(&vm("vm-lineage"), None, None).expect("parent");
            let child = mock
                .save_snapshot(&vm("vm-lineage"), Some(&parent.id), None)
                .expect("child");
            ctx.assert_eq(
                Some(parent.id.clone()),
                child.manifest.parent.clone(),
                "child manifest must record its parent",
            );
            ctx.result()
        }
    }

    crate::conformance_test! {
        name: "save_snapshot_rejects_unknown_parent",
        adapter: "vm_checkpoint",
        capability: Checkpoint,
        category: EdgeCase,
        |ctx| {
            let mock = MockVmCheckpoint::new();
            let bogus = SnapshotId::from_digest([8; 32]);
            let result = mock.save_snapshot(&vm("vm-lineage"), Some(&bogus), None);
            ctx.assert_err(result, "a parent that does not exist must be rejected");
            ctx.result()
        }
    }
    ```

    `VmId` and `SnapshotId` need `PartialEq` and `Debug` for `assert_eq` (both derive them).
    If `ctx.assert_eq` requires `Clone`, both derive it.

11. `crates/minibox/tests/conformance_snapshot.rs`: replace the imports and the three
    `noop_*` tests and the round-trip test (lines 45-110):

    ```rust
    use minibox_core::domain::{
        BackendCapability, BackendCapabilitySet, NoopVmCheckpoint, PayloadEntry, PayloadKind,
        SnapshotId, SnapshotInfo, SnapshotManifest, VmCheckpoint, VmId,
    };
    ```

    ```rust
    #[test]
    fn noop_save_returns_error() {
        let noop = NoopVmCheckpoint;
        let vm = VmId::parse("ctr-1").expect("vm id");
        let result = noop.save_snapshot(&vm, None, None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not supported"), "expected 'not supported' in error: {msg}");
    }

    #[test]
    fn noop_restore_returns_error() {
        let noop = NoopVmCheckpoint;
        let vm = VmId::parse("ctr-1").expect("vm id");
        let id = SnapshotId::from_digest([1; 32]);
        assert!(noop.restore_snapshot(&vm, &id).is_err());
    }

    #[test]
    fn noop_list_returns_error() {
        let noop = NoopVmCheckpoint;
        let vm = VmId::parse("ctr-1").expect("vm id");
        assert!(noop.list_snapshots(&vm).is_err());
    }

    #[test]
    fn snapshot_info_serializes_roundtrip() {
        let manifest = SnapshotManifest {
            schema_version: SnapshotManifest::SCHEMA_VERSION,
            vm_id: VmId::parse("ctr-abc").expect("vm id"),
            parent: None,
            created_at: "2026-04-27T00:00:00Z".to_string(),
            adapter: "vz".to_string(),
            label: None,
            vm_config_digest: "0".repeat(64),
            payloads: vec![PayloadEntry {
                kind: PayloadKind::MachineState,
                len: 104_857_600,
                sha256: "0".repeat(64),
            }],
        };
        let info = SnapshotInfo {
            id: manifest.id().expect("id"),
            manifest,
        };
        let json = serde_json::to_string(&info).expect("serialize");
        let deser: SnapshotInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deser, info);
        assert_eq!(deser.manifest.size_bytes(), 104_857_600);
    }
    ```

    Remove the now-unused `use std::path::Path;` import.

12. `crates/minibox-core/tests/property_roundtrip.rs` (line 230): replace `arb_snapshot_info`:

    ```rust
    fn arb_snapshot_info() -> impl Strategy<Value = minibox_core::domain::SnapshotInfo> {
        use minibox_core::domain::{
            PayloadEntry, PayloadKind, SnapshotInfo, SnapshotLabel, SnapshotManifest, VmId,
        };
        ("[a-z0-9]{1,16}", "[A-Za-z0-9]{1,16}", 0..1_000_000u64).prop_map(|(vm, label, len)| {
            let manifest = SnapshotManifest {
                schema_version: SnapshotManifest::SCHEMA_VERSION,
                vm_id: VmId::parse(&vm).expect("generated vm id is valid"),
                parent: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                adapter: "mock".to_string(),
                label: Some(SnapshotLabel::parse(&label).expect("generated label is valid")),
                vm_config_digest: "0".repeat(64),
                payloads: vec![PayloadEntry {
                    kind: PayloadKind::MachineState,
                    len,
                    sha256: "0".repeat(64),
                }],
            };
            let id = manifest.id().expect("manifest id");
            SnapshotInfo { id, manifest }
        })
    }
    ```

13. `crates/minibox-core/tests/protocol_evolution.rs`: at both construction sites (lines ~182 and
    ~514) replace the inline `SnapshotInfo { container_id, name, created_at, adapter, image,
size_bytes }` with a call to a new helper defined once near the top of the file, and update
    the import (`use minibox_core::domain::{PhaseOutcome, SnapshotInfo, StepStatus};` stays):

    ```rust
    fn sample_snapshot_info() -> SnapshotInfo {
        use minibox_core::domain::{PayloadEntry, PayloadKind, SnapshotManifest, VmId};
        let manifest = SnapshotManifest {
            schema_version: SnapshotManifest::SCHEMA_VERSION,
            vm_id: VmId::parse("abc123").expect("vm id"),
            parent: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            adapter: "vz".to_string(),
            label: None,
            vm_config_digest: "0".repeat(64),
            payloads: vec![PayloadEntry {
                kind: PayloadKind::MachineState,
                len: 1024,
                sha256: "0".repeat(64),
            }],
        };
        let id = manifest.id().expect("id");
        SnapshotInfo { id, manifest }
    }
    ```

    and `DaemonResponse::SnapshotSaved { info: sample_snapshot_info() }` at both sites.
    If a golden/insta snapshot of these responses exists, regenerate it with
    `cargo insta review` and read the diff before accepting.

14. Handler tests. Add `None,` as the new third argument to every `handle_save_snapshot` call:
    `rg -n 'handle_save_snapshot\(' crates --glob '*.rs'` (sites:
    `crates/minibox/tests/daemon_handler_coverage_tests.rs:385`,
    `crates/minibox/tests/daemon_handler_error_coverage_b4b.rs:299`,
    `crates/minibox/tests/snapshot_handler_hardening_tests.rs` ×2,
    `crates/miniboxd/tests/checkpoint_wiring_tests.rs:174,268,311,338`). The restore and list
    signatures are unchanged.
15. `crates/miniboxd/tests/checkpoint_wiring_tests.rs`: besides adding `None,`:
    - `save_snapshot_succeeds_when_adapter_provides_an_implementation`: replace
      `assert_eq!(info.container_id, "ctr-1");` with
      `assert_eq!(info.manifest.vm_id.as_str(), "ctr-1");`.
    - `save_restore_round_trips_through_an_in_memory_checkpoint_adapter`: the restore uses the
      label, which resolves to the saved ID. Replace the final assertions with:

      ```rust
      let DaemonResponse::SnapshotRestored { id, name } = &restored else {
          panic!("restore must succeed for a listed snapshot, got {restored:?}");
      };
      assert_eq!(id, "ctr-1");
      assert_eq!(name, snapshots[0].id.as_str());
      ```

      (`snapshots` is already bound from the `SnapshotList` match above it.)

    - `save_snapshot_reports_unsupported_and_writes_no_snapshot`: keep as is; with the
      handler no longer touching the filesystem the "no files left behind" assertions still
      hold (`read_dir` of a missing directory is the `unwrap_or_default` empty case).
    - `noop_checkpoint_rejects_every_operation` (line 378): replace the body with:

      ```rust
      let noop: minibox_core::domain::DynVmCheckpoint = Arc::new(NoopVmCheckpoint);
      let vm = minibox_core::domain::VmId::parse("ctr-1").expect("vm id");
      let id = minibox_core::domain::SnapshotId::from_digest([1; 32]);

      for message in [
          noop.save_snapshot(&vm, None, None).err(),
          noop.restore_snapshot(&vm, &id).err(),
          noop.list_snapshots(&vm).err(),
      ] {
          let message = message.expect("no-op must reject, never fabricate success");
          assert!(
              message.to_string().contains("not supported"),
              "rejection must say 'not supported', got: {message}"
          );
      }
      ```

16. Find anything else still referencing removed fields or old signatures:
    `rg -n 'container_id|size_bytes|\.snap\b' crates --glob '*.rs'` filtered to snapshot code, and
    `rg -n 'SnapshotInfo' crates xtask --glob '*.rs'`. Fix each hit to the new shape. Run
    `cargo check --workspace --all-targets` until clean.
17. Verify (hooks run the gates on commit): `cargo xtask test unit` passes, including
    `snapshot_store_tests`, `snapshot_handler_hardening_tests`, `checkpoint_wiring_tests`,
    `conformance_snapshot`, `protocol_evolution` and `property_roundtrip`.
18. Commit checkpoint: `feat(snapshot): content-addressed lineage and VmId-keyed checkpoint port`.
    Read `git diff --cached --stat` first; it should list only the files named in this task.

### Task 2.6: Documentation and tracker cleanup

**Crate**: docs only
**File(s)**: `crates/miniboxd/src/main.rs` (comments only), `docs/core/FEATURE_MATRIX.mbx.md`,
`docs/core/SECURITY_INVARIANTS.mbx.md`, `docs/designs/2026-11-16-snapshot-lineage-design.md`
**Run**: `cargo xtask verify` (docs-audit) is run by the commit hook

1. The wiring comments in `crates/miniboxd/src/main.rs` (lines ~1069, 1138, 1266, 1335) say "no
   adapter implements VmCheckpoint". Leave them; they stay true for these suites. Do not edit
   `checkpoint_wiring_tests.rs` expectations about them.
2. In `docs/core/FEATURE_MATRIX.mbx.md`, find the Checkpoint/snapshot row with
   `rg -n -i 'checkpoint|snapshot' docs/core/FEATURE_MATRIX.mbx.md` and update only that row's
   text to say the port is keyed by VM ID with content-addressed snapshots and that only `vz`
   will implement it (Phase 3). Do not change other rows.
3. In `docs/core/SECURITY_INVARIANTS.mbx.md` add one bullet under the path-validation section:
   snapshot identifiers (`VmId`, `SnapshotLabel`, `SnapshotId`, `ShareTag`) are validated types;
   snapshot directories are named by content address and opened `O_NOFOLLOW`.
4. In the design doc, change `**Status**` to note Phases 1 and 2 landed (fill in after they do).
5. Mark the doob todos for ideas 29 and 31 (`doob todo list`, then `doob todo complete <id>` for
   the matching records), and put `closes <uuid>` in the commit message.
6. Commit checkpoint: `docs: record snapshot lineage foundation and identifier validation`.

---

# Phase 3: VZ Backend (crates `macbox`, `minibox-testsuite`, `miniboxd` tests)

**Do not start until Phase 0's spike (Task 0.5) passed and Phase 2 landed.** If the spike changed
any assumption, amend the affected task below first and record why in the design doc.

### Task 3.1: `clone_tree` via `clonefile`

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/clone.rs` (new), `crates/macbox/src/vz/mod.rs`
**Run**: `cargo nextest run -p macbox clone` (macOS, APFS temp dir)

1. Register `pub mod clone;` in `crates/macbox/src/vz/mod.rs` with the module's existing cfg.
2. Failing test and implementation:

   ```rust
   //! Copy-on-write directory cloning for snapshot payloads.

   use anyhow::{Context, Result, bail};
   use std::ffi::CString;
   use std::os::unix::ffi::OsStrExt;
   use std::path::Path;

   /// Clone `src` to `dst` with `clonefile(2)`. `dst` must not exist.
   ///
   /// On APFS this is a constant-time copy-on-write clone of the whole tree. On a filesystem
   /// without clone support the call fails with `ENOTSUP`, and so does the snapshot; there is no
   /// silent fallback to a full copy.
   ///
   /// # Errors
   ///
   /// Returns an error if `dst` exists, a path contains a NUL byte, or the clone fails.
   pub fn clone_tree(src: &Path, dst: &Path) -> Result<()> {
       if dst.exists() {
           bail!("clone destination already exists: {}", dst.display());
       }
       let src_c = CString::new(src.as_os_str().as_bytes())
           .with_context(|| format!("path contains NUL: {}", src.display()))?;
       let dst_c = CString::new(dst.as_os_str().as_bytes())
           .with_context(|| format!("path contains NUL: {}", dst.display()))?;
       // SAFETY: both pointers are valid NUL-terminated C strings that outlive the call;
       // clonefile only reads them. Flags 0 means default behaviour (follow no extra options).
       let rc = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
       if rc != 0 {
           let err = std::io::Error::last_os_error();
           bail!("clonefile {} -> {}: {err}", src.display(), dst.display());
       }
       Ok(())
   }

   #[cfg(test)]
   #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   mod tests {
       use super::*;

       #[test]
       fn clone_tree_copies_files_and_isolates_later_writes() {
           let tmp = tempfile::tempdir().expect("tempdir");
           let src = tmp.path().join("src");
           std::fs::create_dir_all(src.join("etc")).expect("mkdir");
           std::fs::write(src.join("etc/hostname"), b"before\n").expect("write");

           let dst = tmp.path().join("dst");
           clone_tree(&src, &dst).expect("clone");
           assert_eq!(std::fs::read(dst.join("etc/hostname")).expect("read"), b"before\n");

           std::fs::write(src.join("etc/hostname"), b"after\n").expect("mutate source");
           assert_eq!(
               std::fs::read(dst.join("etc/hostname")).expect("read"),
               b"before\n",
               "a clone must not observe later writes to its source"
           );
       }

       #[test]
       fn clone_tree_refuses_an_existing_destination() {
           let tmp = tempfile::tempdir().expect("tempdir");
           let src = tmp.path().join("src");
           std::fs::create_dir_all(&src).expect("mkdir");
           let dst = tmp.path().join("dst");
           std::fs::create_dir_all(&dst).expect("mkdir");
           assert!(clone_tree(&src, &dst).is_err());
       }
   }
   ```

   This module is macOS-only (`clonefile` exists only in Apple libc); gate it with
   `#[cfg(target_os = "macos")]` in `mod.rs`. `tempfile` must be a `macbox` dev-dependency (Task
   0.1 adds it). The temp dir must be on APFS; the default `$TMPDIR` on macOS is.

3. Verify the tests on macOS. Expected FAIL before the implementation, PASS after.
4. Commit checkpoint: `feat(macbox): add clonefile-based clone_tree`.

### Task 3.2: VM configuration digest

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/checkpoint.rs` (new, first part), `crates/macbox/src/vz/mod.rs`
**Run**: `cargo nextest run -p macbox config_digest`

1. Register `pub mod checkpoint;` in `mod.rs` (macOS + `vz` cfg, like the other VZ modules).
2. Failing test, then implementation of the digest, in `checkpoint.rs`:

   ```rust
   //! `VzCheckpoint`: the macOS `VmCheckpoint` implementation.

   use crate::vz::identity::VmIdentity;
   use crate::vz::vm::VzVmConfig;
   use anyhow::{Context, Result};
   use minibox_core::vm_boot;
   use sha2::{Digest, Sha256};
   use std::io::Read;
   use std::path::Path;

   fn hash_file_into(hasher: &mut Sha256, path: &Path) -> Result<()> {
       let mut file = std::fs::File::open(path)
           .with_context(|| format!("open boot asset {}", path.display()))?;
       let mut buf = vec![0_u8; 1 << 20];
       loop {
           let n = file.read(&mut buf).with_context(|| format!("read {}", path.display()))?;
           if n == 0 {
               return Ok(());
           }
           hasher.update(&buf[..n]);
       }
   }

   /// SHA-256 hex over everything that must match for a saved machine state to restore:
   /// memory, CPU count, share tags, the pinned identity and the boot assets.
   ///
   /// # Errors
   ///
   /// Returns an error if a boot asset cannot be read.
   pub fn config_digest(config: &VzVmConfig, identity: &VmIdentity) -> Result<String> {
       let mut hasher = Sha256::new();
       hasher.update(config.memory_bytes.to_le_bytes());
       hasher.update((config.cpu_count as u64).to_le_bytes());
       for tag in [vm_boot::ROOTFS_TAG, vm_boot::IMAGES_TAG, vm_boot::CONTAINERS_TAG] {
           hasher.update(tag.as_bytes());
           hasher.update([0]);
       }
       hasher.update(identity.digest_input());
       hash_file_into(&mut hasher, &config.kernel_path())?;
       hash_file_into(&mut hasher, &config.initramfs_path())?;
       hash_file_into(&mut hasher, &config.vm_dir.join(vm_boot::ESP_IMAGE))?;
       Ok(hex::encode(hasher.finalize()))
   }

   #[cfg(test)]
   #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   mod tests {
       use super::*;

       fn fixture(dir: &Path) -> VzVmConfig {
           std::fs::create_dir_all(dir.join("boot")).expect("boot dir");
           std::fs::write(dir.join("boot/vmlinuz-virt"), b"kernel").expect("kernel");
           std::fs::write(dir.join("boot/initramfs-virt"), b"initramfs").expect("initramfs");
           std::fs::write(dir.join(vm_boot::ESP_IMAGE), b"esp").expect("esp");
           VzVmConfig {
               vm_dir: dir.to_path_buf(),
               images_dir: dir.join("images"),
               containers_dir: dir.join("containers"),
               memory_bytes: 1 << 30,
               cpu_count: 2,
           }
       }

       fn identity() -> VmIdentity {
           VmIdentity::from_parts(vec![1, 2, 3], "02:00:00:00:00:01".to_string())
       }

       #[test]
       fn config_digest_is_stable() {
           let tmp = tempfile::tempdir().expect("tmp");
           let cfg = fixture(tmp.path());
           assert_eq!(
               config_digest(&cfg, &identity()).expect("a"),
               config_digest(&cfg, &identity()).expect("b")
           );
       }

       #[test]
       fn config_digest_changes_with_memory_cpu_identity_and_assets() {
           let tmp = tempfile::tempdir().expect("tmp");
           let cfg = fixture(tmp.path());
           let base = config_digest(&cfg, &identity()).expect("base");

           let mut more_mem = cfg.clone();
           more_mem.memory_bytes += 1;
           assert_ne!(config_digest(&more_mem, &identity()).expect("mem"), base);

           let mut more_cpu = cfg.clone();
           more_cpu.cpu_count += 1;
           assert_ne!(config_digest(&more_cpu, &identity()).expect("cpu"), base);

           let other = VmIdentity::from_parts(vec![9], "02:00:00:00:00:01".to_string());
           assert_ne!(config_digest(&cfg, &other).expect("id"), base);

           std::fs::write(tmp.path().join("boot/vmlinuz-virt"), b"kernel2").expect("swap kernel");
           assert_ne!(config_digest(&cfg, &identity()).expect("asset"), base);
       }
   }
   ```

   `VzVmConfig` derives `Clone` and `Debug` already. `hex` is not a `macbox` dependency: add
   `hex.workspace = true` (it is a workspace dependency) or use `format!("{:x}", digest)` via
   `sha2`'s `LowerHex`; prefer the workspace `hex`.

3. Run: expected PASS.
4. Commit checkpoint: `feat(macbox): add VM configuration digest for snapshot manifests`.

### Task 3.3: `VzCheckpoint` save, list

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/checkpoint.rs`
**Run**: `cargo build -p macbox --features vz`; behavior is covered by the ignored round-trip in
Task 3.8

Append to `checkpoint.rs`:

```rust
use crate::vz::clone::clone_tree;
use crate::vz::vm::VzVm;
use minibox::adapters::snapshot_store::{SnapshotDraft, SnapshotStore};
use minibox_core::domain::{
    PayloadKind, ShareTag, SnapshotError, SnapshotId, SnapshotInfo, SnapshotLabel, VmCheckpoint,
    VmId,
};
use std::path::PathBuf;
use std::sync::Arc;

/// The single VM the VZ adapter manages.
pub const VZ_VM_ID: &str = "minibox-vz";

fn backend(context: &str, e: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::Backend(format!("{context}: {e}"))
}

/// One writable virtiofs share captured by every snapshot.
struct Share {
    tag: &'static str,
    host_path: PathBuf,
}

/// Resumes a VM paused by `save_snapshot` on every exit path.
struct ResumeGuard<'a> {
    vm: &'a VzVm,
    armed: bool,
}

impl ResumeGuard<'_> {
    fn resume_now(&mut self) -> Result<(), SnapshotError> {
        self.armed = false;
        self.vm.resume().map_err(|e| backend("resume VM", e))
    }
}

impl Drop for ResumeGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(e) = self.vm.resume() {
                tracing::error!(error = %e, "vz: failed to resume VM after an aborted snapshot");
            }
        }
    }
}

/// `VmCheckpoint` for the macOS VZ adapter: whole-VM state plus clones of the writable shares.
pub struct VzCheckpoint {
    vm: Arc<VzVm>,
    store: Arc<SnapshotStore>,
    config: VzVmConfig,
    identity: VmIdentity,
    runtime: tokio::runtime::Handle,
}

impl VzCheckpoint {
    /// Build the checkpoint adapter. Must be called inside the tokio runtime that serves requests
    /// (it captures `Handle::current()` to re-establish the agent connection after a restore).
    #[must_use]
    pub fn new(
        vm: Arc<VzVm>,
        store: Arc<SnapshotStore>,
        config: VzVmConfig,
        identity: VmIdentity,
    ) -> Self {
        Self {
            vm,
            store,
            config,
            identity,
            runtime: tokio::runtime::Handle::current(),
        }
    }

    fn require_vm(&self, vm: &VmId) -> Result<(), SnapshotError> {
        if vm.as_str() == VZ_VM_ID {
            Ok(())
        } else {
            Err(SnapshotError::UnknownVm(vm.to_string()))
        }
    }

    fn shares(&self) -> Vec<Share> {
        vec![
            Share {
                tag: vm_boot::ROOTFS_TAG,
                host_path: self.config.rootfs_path(),
            },
            Share {
                tag: vm_boot::IMAGES_TAG,
                host_path: self.config.images_dir.clone(),
            },
            Share {
                tag: vm_boot::CONTAINERS_TAG,
                host_path: self.config.containers_dir.clone(),
            },
        ]
    }

    fn share_kind(tag: &str) -> Result<PayloadKind, SnapshotError> {
        Ok(PayloadKind::ShareClone {
            tag: ShareTag::parse(tag)?,
        })
    }
}

impl VmCheckpoint for VzCheckpoint {
    fn save_snapshot(
        &self,
        vm: &VmId,
        parent: Option<&SnapshotId>,
        label: Option<&SnapshotLabel>,
    ) -> Result<SnapshotInfo, SnapshotError> {
        self.require_vm(vm)?;
        let staging = self.store.begin(vm)?;

        let mut payloads = vec![PayloadKind::MachineState];
        let mut guard = ResumeGuard {
            vm: &self.vm,
            armed: false,
        };
        self.vm.pause().map_err(|e| backend("pause VM", e))?;
        guard.armed = true;

        self.vm
            .save_state(&staging.path().join(PayloadKind::MachineState.relative_path()))
            .map_err(|e| backend("save machine state", e))?;

        let clones_dir = staging.path().join("clones");
        std::fs::create_dir_all(&clones_dir).map_err(|e| backend("create clones dir", e))?;
        for share in self.shares() {
            let kind = Self::share_kind(share.tag)?;
            clone_tree(&share.host_path, &staging.path().join(kind.relative_path()))
                .map_err(|e| backend(&format!("clone share {}", share.tag), e))?;
            payloads.push(kind);
        }
        guard.resume_now()?;

        let digest =
            config_digest(&self.config, &self.identity).map_err(|e| backend("config digest", e))?;
        self.store.publish(
            staging,
            SnapshotDraft {
                vm_id: vm.clone(),
                parent: parent.cloned(),
                label: label.cloned(),
                adapter: "vz".to_string(),
                vm_config_digest: digest,
                payloads,
            },
        )
    }

    fn restore_snapshot(&self, vm: &VmId, snapshot: &SnapshotId) -> Result<(), SnapshotError> {
        self.require_vm(vm)?;
        self.restore_verified(vm, snapshot)
    }

    fn list_snapshots(&self, vm: &VmId) -> Result<Vec<SnapshotInfo>, SnapshotError> {
        self.require_vm(vm)?;
        self.store.list(vm)
    }
}
```

`restore_verified` is Task 3.4. Add a `todo`-free stub only in the same commit as 3.4; do not
commit 3.3 alone, because the crate must compile (`restore_snapshot` references
`restore_verified`).

### Task 3.4: `VzCheckpoint` restore with share swap and rollback

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/vz/checkpoint.rs`
**Run**: `cargo build -p macbox --features vz`

Append:

```rust
impl VzCheckpoint {
    /// Verify, stop, swap shares, restore state, resume, reconnect. Rolls shares back on failure.
    fn restore_verified(&self, vm: &VmId, snapshot: &SnapshotId) -> Result<(), SnapshotError> {
        let verified = self.store.load(vm, snapshot)?;

        let current_digest =
            config_digest(&self.config, &self.identity).map_err(|e| backend("config digest", e))?;
        if verified.info().manifest.vm_config_digest != current_digest {
            return Err(SnapshotError::Corrupt(format!(
                "{snapshot}: saved VM configuration differs from the current one; \
                 the machine state cannot be restored"
            )));
        }

        // Build the replacement machine before disturbing anything.
        let fresh = VzVm::build_unstarted(&self.config, &self.identity)
            .map_err(|e| backend("build replacement VM", e))?;

        self.vm.stop_and_wait().map_err(|e| backend("stop VM", e))?;

        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let mut swapped: Vec<(PathBuf, PathBuf)> = Vec::new(); // (live path, parked path)
        let swap_result = (|| -> Result<(), SnapshotError> {
            for share in self.shares() {
                let kind = Self::share_kind(share.tag)?;
                let parked = share.host_path.with_extension(format!("pre-restore-{suffix}"));
                std::fs::rename(&share.host_path, &parked)
                    .map_err(|e| backend(&format!("park share {}", share.tag), e))?;
                swapped.push((share.host_path.clone(), parked));
                clone_tree(&verified.payload_path(&kind), &share.host_path)
                    .map_err(|e| backend(&format!("restore share {}", share.tag), e))?;
            }
            Ok(())
        })();

        let restore_result = swap_result.and_then(|()| {
            self.vm
                .restore_state(fresh, &verified.payload_path(&PayloadKind::MachineState))
                .map_err(|e| backend("restore machine state", e))?;
            self.vm.resume().map_err(|e| backend("resume restored VM", e))
        });

        match restore_result {
            Ok(()) => {
                for (_, parked) in &swapped {
                    if let Err(e) = std::fs::remove_dir_all(parked) {
                        tracing::warn!(
                            path = %parked.display(),
                            error = %e,
                            "vz: failed to remove parked share after restore"
                        );
                    }
                }
                self.runtime
                    .block_on(crate::vz::vsock::connect_to_agent(&self.vm, 60))
                    .map_err(|e| backend("reconnect agent after restore", e))?;
                tracing::info!(snapshot_id = %snapshot, "vz: snapshot restored");
                Ok(())
            }
            Err(e) => {
                for (live, parked) in swapped.iter().rev() {
                    if live.exists() {
                        if let Err(rm) = std::fs::remove_dir_all(live) {
                            tracing::error!(
                                path = %live.display(),
                                error = %rm,
                                "vz: failed to remove partially restored share during rollback"
                            );
                        }
                    }
                    if let Err(rb) = std::fs::rename(parked, live) {
                        tracing::error!(
                            path = %parked.display(),
                            error = %rb,
                            "vz: failed to roll a share back; manual recovery needed"
                        );
                    }
                }
                Err(e)
            }
        }
    }
}
```

Notes for the implementer:

- `connect_to_agent` is async and takes `&Arc<VzVm>` in `lib.rs:566`; adapt the argument to
  match its real signature (`self.vm` is an `Arc<VzVm>`, so pass `&self.vm`).
- `block_on` is legal here because the trait methods run inside `spawn_blocking` (handlers call
  them through `blocking(...)`), never on a runtime worker thread.
- If Task 0.5 showed the host agent connection survives a restore, the reconnect is a harmless
  no-op check; keep it.
- Build, then commit 3.3 and 3.4 together:
  `feat(macbox): implement VmCheckpoint for the VZ adapter`.

### Task 3.5: Wire `VzCheckpoint` into the VZ start path

**Crate**: `macbox`
**File(s)**: `crates/macbox/src/lib.rs`
**Run**: `cargo build -p macbox --features vz`

1. In the VZ start function (around line 459-531), after the config is built and before
   `prepare_on_main_queue`, load the pinned identity and pass it through:

   ```rust
   let identity = vz::identity::VmIdentity::load_or_create(&vm_dir)
       .context("vz: load pinned VM identity")?;
   ```

   Change `prepare_on_main_queue(config: VzVmConfig)` to also take `identity:
&VmIdentity` (and `PrepareCtx` to carry it), calling
   `build_vm_configuration(&config, Some(identity))` instead of `None`. From now on the booted VM
   always uses the pinned identity.

2. After `vm_arc` exists (line ~563) create the store and the adapter:

   ```rust
   let snapshot_root = containers_dir
       .parent()
       .context("vz: containers_dir has no parent directory")?
       .join("snapshots");
   let snapshot_store = Arc::new(
       minibox::adapters::snapshot_store::SnapshotStore::open(&snapshot_root)
           .map_err(|e| anyhow::anyhow!("vz: open snapshot store: {e:?}"))?,
   );
   let checkpoint: minibox_core::domain::DynVmCheckpoint =
       Arc::new(vz::checkpoint::VzCheckpoint::new(
           Arc::clone(&vm_arc),
           snapshot_store,
           vz_config.clone(),
           identity,
       ));
   ```

   (`vz_config` is the `VzVmConfig` cloned before it moves into `PrepareCtx`; clone it there.)

3. In the `HandlerDependencies` literal (line ~626) replace
   `checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),` with
   `checkpoint,`. Leave the two other `NoopVmCheckpoint` sites in `macbox/src/lib.rs`
   (lines 133 and 416, the Colima and krun paths) untouched, and add a comment above each:
   `// VmCheckpoint is a no-op here: only the vz adapter implements the port.`
4. Verify: `cargo build -p macbox --features vz` on macOS; the daemon still boots to the agent.
5. Commit checkpoint: `feat(macbox): wire VzCheckpoint into the VZ daemon`.

### Task 3.6: Declare the `Checkpoint` capability and update the wiring pin

**Crates**: `minibox-testsuite`, `miniboxd` (tests only)
**File(s)**: `crates/minibox-testsuite/src/showcase/context.rs`,
`crates/miniboxd/tests/checkpoint_wiring_tests.rs`
**Run**: `cargo nextest run -p miniboxd --test checkpoint_wiring_tests`

1. Read `crates/minibox-testsuite/src/showcase/context.rs` lines 350-400
   (`descriptor_for_active_adapter`). Add an arm matching the existing style, for the adapter
   name the registry uses for the VZ adapter (confirm with
   `rg -n '"vz"' crates/minibox-core/src/adapter_registry.rs`):

   ```rust
   "vz" => BackendDescriptor::new("vz")
       .with_capability(BackendCapability::Filesystem)
       .with_capability(BackendCapability::Checkpoint),
   ```

   Keep whatever other capabilities the neighboring arms grant the macOS VM adapters; add only
   `Checkpoint` for this change.

2. The pin in `checkpoint_wiring_tests.rs` reads only `main.rs`, which keeps its no-op sites, so
   it needs no change. Add a second pin for the macbox sites:

   ```rust
   const MACBOX_LIB_RS: &str = include_str!("../../macbox/src/lib.rs");

   /// Only the VZ start path may install a real checkpoint; every other `checkpoint:` line in
   /// `macbox/src/lib.rs` must be the honest no-op.
   #[test]
   fn macbox_non_vz_checkpoint_wiring_is_the_honest_noop() {
       let noop = "checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),";
       let real: Vec<&str> = MACBOX_LIB_RS
           .lines()
           .map(str::trim)
           .filter(|l| l.starts_with("checkpoint"))
           .filter(|l| *l != noop)
           .collect();
       assert_eq!(
           real,
           vec!["checkpoint,"],
           "exactly one non-noop wiring (the VZ path's `checkpoint,`) is allowed"
       );
   }
   ```

   The path is relative to `crates/miniboxd/tests/`; confirm it resolves to
   `crates/macbox/src/lib.rs`.

3. Run: expected PASS.
4. Commit checkpoint: `test: declare vz Checkpoint capability and pin macbox checkpoint wiring`.

### Task 3.7: Fix the stale `IMAGES_TAG` doc and update notes

**Crate**: `minibox-core`, docs
**File(s)**: `crates/minibox-core/src/vm_boot.rs`,
`docs/designs/2026-11-16-snapshot-lineage-design.md`,
`.ctx/memory-bank/progress.mbx.md`
**Run**: hook runs docs checks on commit

1. In `crates/minibox-core/src/vm_boot.rs` line 14-15 change the doc comment to:
   `/// virtiofs tag of the image cache share. Writable: the guest daemon pulls images itself and writes layers into it.`
2. Update the design doc `**Status**` to implemented and record the as-built differences
   (`ShareTag`, `publish_at`, Phase 0 primitives).
3. Add a short entry to `.ctx/memory-bank/progress.mbx.md` (tracked file) under what works:
   snapshot lineage and the VZ checkpoint backend. Do not touch the untracked
   `active-context.mbx.md`.
4. Commit checkpoint: `docs: record snapshot lineage as built, fix stale IMAGES_TAG doc`.

### Task 3.8: Ignored macOS round-trip test

**Crate**: `macbox`
**File(s)**: `crates/macbox/tests/vz_checkpoint_roundtrip.rs` (new)
**Run**: `cargo nextest run -p macbox --features vz --run-ignored only vz_checkpoint_roundtrip`
(macOS, entitled and signed binary, built VM image; run on request only)

1. Mirror the process structure of the Task 0.5 spike (worker thread plus `dispatch_main()`),
   because VZ needs the main queue. The test boots the VM with the pinned identity, starts a
   `VzCheckpoint`, and:
   1. writes a marker file into the containers share (`containers_dir/marker`) with contents
      `before`;
   2. `save_snapshot(&vm, None, Some(&label("before")))`;
   3. overwrites the marker with `after`;
   4. `restore_snapshot(&vm, &saved.id)`;
   5. asserts the marker reads `before` again, the agent answers, and `list_snapshots`
      returns exactly the one snapshot with a verifying manifest;
   6. saves a child with `Some(&saved.id)` and asserts `manifest.parent == Some(saved.id)`.
2. Mark it `#[ignore = "needs a macOS host, the virtualization entitlement and a built VM image"]`.
3. Run it once manually; record the result in the design doc's as-built notes.
4. Commit checkpoint: `test(macbox): add ignored VZ snapshot round-trip test`.

---

## Requirement Coverage

| Design requirement                                             | Task(s)         |
| -------------------------------------------------------------- | --------------- |
| Spike gates the VZ backend; answers 4 questions                | 0.5             |
| Pinned machine identifier and MAC                              | 0.1, 0.2, 3.5   |
| `VzVm` swappable inner machine and primitives                  | 0.3, 0.4        |
| `VmId` / `SnapshotLabel` validation; live traversal fix        | 1.1, 1.2, 1.3   |
| `SnapshotId`, manifest, `SnapshotInfo`, `ShareTag`             | 2.1, 2.2        |
| Root-last publication, verification, O_NOFOLLOW, staging sweep | 2.3, 2.4        |
| Port re-key, `SnapshotError`, no `&Path` in the port           | 2.5 (A)         |
| Protocol changes, `parent`, snapshot-reference restore         | 2.5 (B, C)      |
| Handlers validate and use `spawn_blocking`                     | 1.3, 2.5 (C)    |
| Mock, conformance, CLI migration                               | 2.5 (D, E, F)   |
| Docs and tracker cleanup (TODOs 29 and 31)                     | 2.5 (A, C), 2.6 |
| Share cloning via `clonefile`                                  | 3.1             |
| Config digest                                                  | 3.2             |
| `VzCheckpoint` save, list, restore with rollback               | 3.3, 3.4        |
| VZ wiring; other suites keep the no-op                         | 3.5, 3.6        |
| `Checkpoint` capability declared with the implementation       | 3.6             |
| `IMAGES_TAG` doc fix                                           | 3.7             |
| VZ round-trip test (ignored)                                   | 3.8             |

## Risks (carried from the design)

- [ ] Spike may show VZ rejects save/restore for this configuration. Phases 1 and 2 still ship.
- [ ] Hashing the three shares on every save may be slow; hashing runs after the VM resumes.
- [ ] Restoring the images share rolls back images pulled after the snapshot; the spike checks
      whether `~/.minibox/images` is shared with another adapter.
- [ ] A restore is a VM restart for clients; `DaemonState` is not reconciled (out of scope).
- [ ] VZ container execution is blocked on the `pivot_root` `EINVAL`
      (`.ctx/godmode/BLOCKED-vz-pivot-root-einval.md`); snapshotting the VM itself is unaffected.
- [ ] Task 2.5 cannot be split; budget it as a single review.
- [ ] Breaking changes: `VmCheckpoint` signatures, `SnapshotInfo` shape, `SaveSnapshot` gains a
      field. No backend has ever produced a snapshot, so nothing persisted is invalidated.
