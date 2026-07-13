# Plan: MoA Review HIGH Fixes (F1-F8 + D2)

## Goal

Resolve the 11 HIGH findings from the 2026-07-06 MoA review of `develop` vs `main`
(`.ctx/review/00-synthesis.md`), grouped into 8 fix units, plus register the orphan
`crates/ail` crate in the workspace (D2).

## Architecture

- Crates affected: `xtask`, `miniboxd`, `minibox`, `minibox-core`, `minibox-crux-plugin`,
  `minibox-testsuite`, `ail`, workspace root `Cargo.toml`.
- New API: `DaemonResponse::is_terminal()` (inherent const method, `minibox-core/src/protocol.rs`).
- No wire-format changes anywhere in this plan. Protocol-drift lock may need
  `cargo xtask check protocol-drift --update` after Task 10 (method addition changes file hash).
- Branch: all work on `develop`. Every task runs `git branch --show-current` before commit and
  stops if the output is not `develop`.
- Context map source: lens reports `.ctx/review/01..05-*.md` (files-to-modify, callers, risks
  verified there against the working tree).

## Tech Stack

Rust 2024, no new dependencies. Test crates already available: `serial_test`, `wiremock`,
`tempfile`, `proptest`.

## Risks

- Task 1 changes xtask arg dispatch consumed by 6 GHA workflows — verify against the exact
  invocations in `.github/workflows/{stability-gates,merge,pr,conformance,protocol-sites}.yml`
  and `.claude/settings.json` hooks.
- Task 4 (Linux test) and Task 16 cannot be fully validated on macOS — gate with
  `cargo check -p miniboxd --all-targets --target aarch64-unknown-linux-gnu`.
- Task 10 changes plugin terminal semantics — `ContainerCreated` becomes non-terminal for the
  plugin; the plugin's run handler relies on stream close (tx dropped) which `dispatch()`
  already handles via `stream.next() == None`.
- Task 18 (version bump) conflicts with nothing but should land last to avoid rebase churn.

---

## Fix Unit F1 — xtask alias dispatch (H1+H2)

### Task 1: Thread explicit arg slices through check/info dispatch

**Crate**: `xtask`
**File(s)**: `xtask/src/main.rs`
**Run**: `cargo nextest run -p xtask`

Root cause: `dispatch_check`/`dispatch_info` re-read `env::args()` at positions calibrated for
the group form (`xtask check protocol-sites <file>`, file at nth(3)), but deprecated aliases
(`xtask check-protocol-sites <file>`, file at nth(2)) shift everything by one.

1. Write failing tests (new `#[cfg(test)] mod dispatch_args_tests` in `main.rs`) against two
   new pure parsers:

   ```rust
   #[cfg(test)]
   mod dispatch_args_tests {
       use super::*;

       #[test]
       fn protocol_sites_args_from_alias_form() {
           // argv after slicing: alias `xtask check-protocol-sites <file> --expected 4`
           // must yield rest = ["crates/miniboxd/src/main.rs", "--expected", "4"]
           let rest = vec![
               "crates/miniboxd/src/main.rs".to_string(),
               "--expected".to_string(),
               "4".to_string(),
           ];
           let parsed = parse_protocol_sites_args(&rest);
           assert_eq!(
               parsed.file.as_deref(),
               Some(std::path::Path::new("crates/miniboxd/src/main.rs"))
           );
           assert_eq!(parsed.expected, 4);
           assert!(!parsed.warn_only);
       }

       #[test]
       fn protocol_sites_args_default_file_when_flag_first() {
           let rest = vec!["--expected".to_string(), "4".to_string()];
           let parsed = parse_protocol_sites_args(&rest);
           assert!(parsed.file.is_none(), "flag must not be mistaken for the file path");
           assert_eq!(parsed.expected, 4);
       }

       #[test]
       fn protocol_drift_args_sarif_first_flag() {
           // alias `xtask check-protocol-drift --sarif protocol-drift.sarif`
           let rest = vec!["--sarif".to_string(), "protocol-drift.sarif".to_string()];
           let parsed = parse_protocol_drift_args(&rest);
           assert_eq!(
               parsed.sarif.as_deref(),
               Some(std::path::Path::new("protocol-drift.sarif"))
           );
           assert!(!parsed.update && !parsed.warn_only && !parsed.hook);
       }

       #[test]
       fn protocol_drift_args_hook_warn_only() {
           let rest = vec!["--hook".to_string(), "--warn-only".to_string()];
           let parsed = parse_protocol_drift_args(&rest);
           assert!(parsed.hook);
           assert!(parsed.warn_only);
           assert!(parsed.sarif.is_none());
       }

       #[test]
       fn info_changes_base_ref_from_alias_form() {
           // alias `xtask detect-changes origin/main`
           let rest = vec!["origin/main".to_string()];
           assert_eq!(changes_base_ref(&rest), "origin/main");
           assert_eq!(changes_base_ref(&[]), "HEAD^");
       }
   }
   ```

   Run: `cargo nextest run -p xtask -- dispatch_args_tests`
   Expected: FAIL (functions do not exist)

2. Implement in `xtask/src/main.rs`:

   ```rust
   struct ProtocolSitesArgs {
       file: Option<std::path::PathBuf>,
       expected: usize,
       warn_only: bool,
   }

   fn parse_protocol_sites_args(rest: &[String]) -> ProtocolSitesArgs {
       let file = rest
           .first()
           .filter(|a| !a.starts_with("--"))
           .map(std::path::PathBuf::from);
       let expected = rest
           .windows(2)
           .find(|w| w[0] == "--expected")
           .and_then(|w| w[1].parse().ok())
           .unwrap_or(4);
       let warn_only = rest.iter().any(|a| a == "--warn-only");
       ProtocolSitesArgs { file, expected, warn_only }
   }

   struct ProtocolDriftArgs {
       update: bool,
       warn_only: bool,
       hook: bool,
       sarif: Option<std::path::PathBuf>,
   }

   fn parse_protocol_drift_args(rest: &[String]) -> ProtocolDriftArgs {
       ProtocolDriftArgs {
           update: rest.iter().any(|a| a == "--update"),
           warn_only: rest.iter().any(|a| a == "--warn-only"),
           hook: rest.iter().any(|a| a == "--hook"),
           sarif: rest
               .windows(2)
               .find(|w| w[0] == "--sarif")
               .map(|w| std::path::PathBuf::from(&w[1])),
       }
   }

   fn changes_base_ref(rest: &[String]) -> String {
       rest.first()
           .filter(|a| !a.starts_with("--"))
           .cloned()
           .unwrap_or_else(|| "HEAD^".to_string())
   }
   ```

   Then rewire dispatch to pass slices instead of re-reading `env::args()`:

   - In `main()`: `let argv: Vec<String> = env::args().collect();`
   - `Some("check") => cmd_check(&sh, root, &argv[2..])` — `cmd_check` takes
     `rest: &[String]`, uses `rest.first()` as the sub and passes `&rest[1..]` on.
   - `Some("info") => cmd_info(&sh, root, &argv[2..])` — same shape.
   - Alias arms pass `&argv[2..]`:
     `Some(cmd) if is_check_alias(cmd) => { ...; dispatch_check(&sh, root, &sub, &argv[2..]) }`
     `Some(cmd) if is_info_alias(cmd) => { ...; dispatch_info(&sh, root, &sub, &argv[2..]) }`
   - `dispatch_check(sh, root, sub, rest: &[String])`:
     - `"protocol-drift"` arm uses `parse_protocol_drift_args(rest)`.
     - `"protocol-sites"` arm uses `parse_protocol_sites_args(rest)`;
       `file` defaults to `root.join("crates/miniboxd/src/main.rs")` when `None`.
   - `dispatch_info(sh, root, sub, rest)`: `"changes"` arm uses `changes_base_ref(rest)`.
   - `dispatch_docs` and `dispatch_test` keep their signatures (docs lint already scans
     flags via `skip(1)`; test suites take no positionals) — out of scope.

3. Verify:

   ```
   cargo nextest run -p xtask                             → all green
   cargo clippy -p xtask -- -D warnings                   → zero warnings
   cargo run -p xtask -- check-protocol-sites crates/miniboxd/src/main.rs --expected 4
                                                          → runs site-count check, exit 0
   cargo run -p xtask -- detect-changes origin/main       → classifies against origin/main
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(xtask): pass arg slices through alias dispatch [moa-review]"`

### Task 2: Resolve `check-protocol-sites` command collision

**Crate**: `xtask`
**File(s)**: `xtask/src/main.rs`, `xtask/src/check_protocol_sites.rs`
**Run**: `cargo nextest run -p xtask`

The explicit arm `Some("check-protocol-sites") => check_protocol_sites::run(root)`
(main.rs:240, the new dead-variant scanner) shadows the deprecated alias for the
HandlerDependencies site-count guard — CI invokes the wrong tool with all args ignored.

1. Write failing test:

   ```rust
   #[test]
   fn check_alias_maps_protocol_sites_to_site_count_guard() {
       // The alias must map to the `check protocol-sites` sub, and no top-level
       // command may shadow it.
       assert!(is_check_alias("check-protocol-sites"));
       assert_eq!(check_alias_to_sub("check-protocol-sites"), "protocol-sites");
   }
   ```

   Plus a shell-level check in step 3 (the real regression is dispatch order).
   Run: `cargo nextest run -p xtask -- check_alias_maps`
   Expected: PASS already — the failing signal is step 3's first verify command,
   which currently runs the dead-variant scanner. Capture that before the fix:
   `cargo run -p xtask -- check-protocol-sites nonexistent-file --expected 99` currently
   exits 0 (scanner ignores args). After the fix it must fail on the missing file.

2. Implement:

   - Delete the `Some("check-protocol-sites") => check_protocol_sites::run(root)` arm
     (main.rs:240).
   - Register the dead-variant scanner under the check group instead: add
     `"protocol-variants" => check_protocol_sites::run(root),` to `dispatch_check`, and a
     help line under `cmd_check`:
     `eprintln!("  protocol-variants  scan for DaemonRequest/DaemonResponse variants with no handler sites");`
   - Grep for callers of the scanner before renaming:
     `grep -rn "check-protocol-sites" .github/ Justfile .claude/settings.json scripts/`
     — `stability-gates.yml:90` and `protocol-sites.yml:30` expect the site-count guard
     (correct after this fix). If any caller expects the scanner, update it to
     `cargo xtask check protocol-variants`.

3. Verify:

   ```
   cargo run -p xtask -- check-protocol-sites crates/miniboxd/src/main.rs --expected 4  → exit 0 (site-count guard)
   cargo run -p xtask -- check-protocol-sites nonexistent-file --expected 99            → exit != 0 (file not found)
   cargo run -p xtask -- check protocol-variants                                        → runs scanner
   cargo clippy -p xtask -- -D warnings                                                 → zero warnings
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(xtask): unshadow check-protocol-sites alias; scanner moves to check protocol-variants [moa-review]"`

---

## Fix Unit F2 — compile breaks invisible on macOS (H3+H4)

### Task 3: Fix bench `trait_overhead` import of privatized `handler::run`

**Crate**: `minibox`
**File(s)**: `crates/minibox/benches/trait_overhead.rs`
**Run**: `cargo check -p minibox --benches`

1. Failing state is the compile error itself:
   `cargo check -p minibox --benches` → `error[E0603]: module 'run' is private` at line 11.

2. Implement:

   - Line 11: `use minibox::daemon::handler::run::RunParams;` →
     `use minibox::daemon::handler::RunParams;`
   - Sweep every remaining `handler::run::` path in the file to the `handler::` re-export
     (`grep -n "handler::run::" crates/minibox/benches/trait_overhead.rs` — ~15 sites).
   - Line 9: replace deprecated `criterion::black_box` with `std::hint::black_box`:
     drop `black_box` from the criterion import list and add `use std::hint::black_box;`.

3. Verify:

   ```
   cargo check -p minibox --benches           → clean, zero warnings
   cargo clippy -p minibox --benches -- -D warnings  → zero warnings
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(minibox): repair trait_overhead bench imports after handler privatization [moa-review]"`

### Task 4: Fix Linux-gated nesting integration test against current handler API

**Crate**: `miniboxd`
**File(s)**: `crates/miniboxd/tests/integration_tests.rs` (lines 604-669)
**Run**: `cargo check -p miniboxd --all-targets --target aarch64-unknown-linux-gnu`

The stale block references the pre-refactor surface: `create_test_deps` (now
`create_real_deps`, line 88), 3-arg `handle_pull`/`handle_run`, `RunParams.network_mode`/
`capture_output` (removed), `DaemonResponse::ContainerStarted` (now `ContainerCreated`).

1. Failing state is the compile error on the Linux target (8 errors):
   `cargo check -p miniboxd --all-targets --target aarch64-unknown-linux-gnu`

2. Implement — rewrite the test body to the current API, using the ephemeral streaming
   contract (zero or more `ContainerOutput`, then terminal `ContainerStopped`) instead of
   the removed `capture_output`:

   ```rust
   #[tokio::test]
   #[serial]
   async fn test_container_receives_nesting_env_vars() {
       if !is_root() {
           eprintln!("SKIP: test_container_receives_nesting_env_vars (not root)");
           return;
       }

       let (deps, state, _tmp) = create_real_deps();

       let (pull_tx, mut pull_rx) = tokio::sync::mpsc::channel(16);
       handler::handle_pull(
           "alpine".to_string(),
           Some("latest".to_string()),
           None,
           state.clone(),
           deps.clone(),
           pull_tx,
       )
       .await;
       while pull_rx.recv().await.is_some() {}

       let (tx, mut rx) = tokio::sync::mpsc::channel(64);
       let params = handler::RunParams {
           image: "alpine".to_string(),
           tag: Some("latest".to_string()),
           command: vec![
               "/bin/sh".to_string(),
               "-c".to_string(),
               "echo DEPTH=$MINIBOX_NEST_DEPTH MAX=$MINIBOX_MAX_NEST_DEPTH".to_string(),
           ],
           ephemeral: true,
           ..Default::default()
       };
       handler::handle_run(params, state.clone(), deps.clone(), tx).await;

       let mut output = String::new();
       while let Some(resp) = rx.recv().await {
           match resp {
               DaemonResponse::ContainerOutput { data, .. } => {
                   output.push_str(&String::from_utf8_lossy(&data));
               }
               DaemonResponse::ContainerStopped { .. } => break,
               DaemonResponse::Error { message } => panic!("run failed: {message}"),
               _ => {}
           }
       }

       let depth_line = output
           .lines()
           .find(|l| l.contains("DEPTH="))
           .expect("should have DEPTH= line in output");
       assert!(depth_line.contains("DEPTH=1"), "expected DEPTH=1, got: {depth_line}");
       assert!(depth_line.contains("MAX=4"), "expected MAX=4, got: {depth_line}");
   }
   ```

   Adjust to the actual signatures at implementation time — the gate is the Linux-target
   check below. Confirm `handle_pull`'s exact parameter order from
   `crates/minibox/src/daemon/handler/image.rs:78` and whether the file already imports
   `serial_test::serial` (add `use serial_test::serial;` if not — neighboring tests at
   line 605 already use the attribute).

3. Verify:

   ```
   cargo check -p miniboxd --all-targets --target aarch64-unknown-linux-gnu  → clean
   cargo check --workspace --all-targets                                     → clean on macOS
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(miniboxd): port nesting integration test to refactored handler API [moa-review]"`

---

## Fix Unit F3 — pipeline policy_override wiring (H5)

### Task 5: Consume `policy_override` in `handle_run`

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handler/run.rs`
**Run**: `cargo nextest run -p minibox -- policy`

1. Write failing test (in `crates/minibox/src/daemon/handler/mod.rs`
   `pub_crate_handler_tests`, which already builds mock deps):

   ```rust
   #[tokio::test]
   async fn handle_run_policy_override_permits_bind_mount_under_deny_policy() {
       // Default policy denies bind mounts; the override (as used by handle_pipeline)
       // must widen it for this run only.
       let (state, deps) = make_mock_state_and_deps_with_policy(ContainerPolicy::default());
       let (tx, mut rx) = tokio::sync::mpsc::channel(8);
       let params = run::RunParams {
           image: "alpine".to_string(),
           mounts: vec![minibox_core::domain::BindMount {
               host_path: "/tmp/pipeline".into(),
               container_path: "/pipeline.crux".into(),
               read_only: true,
           }],
           policy_override: Some(PolicyOverride {
               allow_bind_mounts: Some(true),
               ..Default::default()
           }),
           ..Default::default()
       };
       run::handle_run(params, state, deps, tx).await;
       let first = rx.recv().await.expect("response");
       if let minibox_core::protocol::DaemonResponse::Error { message } = &first {
           assert!(
               !message.contains("policy violation"),
               "run must pass the policy gate with an override, got: {message}"
           );
       }
   }
   ```

   Use the existing mock-deps builder in that test module (`make_mock_deps`-style — reuse
   whatever `pub_crate_handler_tests` already constructs; only the policy field matters).
   Run: `cargo nextest run -p minibox -- handle_run_policy_override`
   Expected: FAIL (error message contains "policy violation: bind mount requested")

2. Implement in `handle_run` (run.rs:150-156) — compute the effective policy before the gate:

   ```rust
   // Policy gate: deny bind mounts and privileged mode unless explicitly allowed.
   // Internal callers (pipeline runs) may widen the policy via `policy_override`.
   let effective_policy = params
       .policy_override
       .as_ref()
       .map_or_else(|| deps.policy.clone(), |ov| deps.policy.with_overrides(ov));
   if let Err(msg) = super::validate_policy(
       &params.mounts,
       params.privileged,
       params.priority,
       &effective_policy,
   ) {
   ```

   `prepare_run`'s `policy_override: _` destructure (run.rs:563) stays — the field is
   consumed at the gate.

3. Verify:

   ```
   cargo nextest run -p minibox                    → all green
   cargo clippy -p minibox -- -D warnings          → zero warnings
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(minibox): apply policy_override in handle_run policy gate [moa-review]"`

### Task 6: Pin pipeline runs under the default deny policy

**Crate**: `minibox`
**File(s)**: `crates/minibox/tests/daemon_handler_pipeline_tests.rs` (or the existing pipeline
test file found via `grep -rln handle_pipeline crates/minibox/tests/`)
**Run**: `cargo nextest run -p minibox -- pipeline_default_deny`

1. Write the test (failing before Task 5, green after — order Tasks 5 then 6; the test
   pins the regression permanently):

   ```rust
   #[tokio::test]
   async fn pipeline_run_passes_policy_gate_under_default_deny_policy() {
       // Regression: pipeline.rs mounts /pipeline.crux via policy_override; with the
       // default deny policy this must NOT be rejected by validate_policy.
       // Build deps exactly like the existing pipeline tests in this file, but with
       // ContainerPolicy::default() instead of allow_bind_mounts: true.
       ...
       // Assert: no DaemonResponse::Error containing "policy violation" is received.
   }
   ```

   Copy the harness of the nearest existing pipeline test in the same file
   (`tests/daemon_handler_common/mod.rs:210` builds the permissive deps — clone that
   builder with `ContainerPolicy::default()`).

2. Verify: `cargo nextest run -p minibox -- pipeline` → all green.

3. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "test(minibox): pin pipeline run under default deny policy [moa-review]"`

---

## Fix Unit F4 — reconcile_on_startup wiring (H6)

### Task 7: Call `reconcile_on_startup` in daemon init

**Crate**: `miniboxd`
**File(s)**: `crates/miniboxd/src/main.rs` (line ~400, after `load_from_disk`)
**Run**: `cargo check -p miniboxd --all-targets --target aarch64-unknown-linux-gnu`

1. Write failing test first — in `crates/minibox/src/daemon/state.rs` there are 6 reconcile
   unit tests calling the method directly; the missing piece is the production call. Add an
   init-path test in `crates/miniboxd/tests/` (or extend an existing daemon-recovery test
   file found via `grep -rln reconcile crates/miniboxd/tests/ crates/minibox/tests/`):

   ```rust
   #[tokio::test]
   async fn startup_reconciles_running_record_with_dead_pid() {
       // Write a state file containing a "Running" record with PID 999999 (dead),
       // then drive the same load+reconcile path main() uses and assert the record
       // is "Orphaned".
   }
   ```

   To make that path testable, extract the init sequence from `run_daemon` into a helper
   in miniboxd (e.g. `async fn load_state(paths: &DaemonPaths) -> Result<Arc<DaemonState>>`)
   that does `DaemonState::new` + `load_from_disk` + `reconcile_on_startup`, and test the
   helper. Use the production `ProcessChecker` impl (locate with
   `grep -n "impl ProcessChecker" crates/minibox/src/daemon/state.rs`) and
   `FsCgroupFreezeChecker` (state.rs:63).
   Expected: FAIL until step 2 wires reconcile in.

2. Implement in `run_daemon` (main.rs:400):

   ```rust
   state.load_from_disk().await;
   state
       .reconcile_on_startup(&<production ProcessChecker>, &FsCgroupFreezeChecker)
       .await;
   info!("state loaded from disk and reconciled");
   ```

   (Exact checker constructor names from the grep in step 1; both live in
   `minibox::daemon::state`.)

3. Verify:

   ```
   cargo nextest run -p miniboxd                                               → green
   cargo check -p miniboxd --all-targets --target aarch64-unknown-linux-gnu    → clean
   cargo clippy -p miniboxd -- -D warnings                                     → zero warnings
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(miniboxd): wire reconcile_on_startup into daemon init [moa-review]"`

---

## Fix Unit F5 — single terminal-response predicate (H7)

### Task 8: Add `DaemonResponse::is_terminal()` to minibox-core

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/protocol.rs`
**Run**: `cargo nextest run -p minibox-core -- is_terminal`

1. Write failing test (protocol.rs tests module) — exhaustive table over all 28 variants,
   copied from the canonical list in `crates/minibox/src/daemon/server.rs:409-435`:

   ```rust
   #[test]
   fn is_terminal_matches_canonical_table() {
       // Terminal: ContainerStopped, Error, Success, ContainerList, ImageLoaded,
       // BuildComplete, ContainerPaused, ContainerResumed, Pruned, PipelineComplete,
       // SnapshotSaved, SnapshotRestored, SnapshotList, ImageList, Manifest,
       // VerifyResult, WorkflowStepComplete, WorkflowComplete, PipelineList,
       // PipelineDetail.
       // Non-terminal: ContainerOutput, LogLine, ContainerCreated, ExecStarted,
       // PushProgress, BuildOutput, Event, UpdateProgress.
       assert!(!sample_container_created().is_terminal());
       assert!(sample_build_complete().is_terminal());
       // ... one assertion per variant, constructing minimal sample values.
   }
   ```

   Run: `cargo nextest run -p minibox-core -- is_terminal` — Expected: FAIL (no method).

2. Implement — move the match from server.rs verbatim into an inherent method:

   ```rust
   impl DaemonResponse {
       /// True for response variants that terminate a request/response exchange.
       ///
       /// `ContainerCreated` is intentionally non-terminal: ephemeral runs send it
       /// first, followed by `ContainerOutput` chunks and a terminal
       /// `ContainerStopped`. Non-ephemeral runs send it and then drop the sender.
       #[must_use]
       pub const fn is_terminal(&self) -> bool {
           matches!(
               self,
               Self::ContainerStopped { .. }
                   | Self::Error { .. }
                   | Self::Success { .. }
                   | Self::ContainerList { .. }
                   | Self::ImageLoaded { .. }
                   | Self::BuildComplete { .. }
                   | Self::ContainerPaused { .. }
                   | Self::ContainerResumed { .. }
                   | Self::Pruned { .. }
                   | Self::PipelineComplete { .. }
                   | Self::SnapshotSaved { .. }
                   | Self::SnapshotRestored { .. }
                   | Self::SnapshotList { .. }
                   | Self::ImageList { .. }
                   | Self::Manifest { .. }
                   | Self::VerifyResult { .. }
                   | Self::WorkflowStepComplete { .. }
                   | Self::WorkflowComplete { .. }
                   | Self::PipelineList { .. }
                   | Self::PipelineDetail { .. }
           )
       }
   }
   ```

3. Verify (wire format unchanged — snapshots must not move):

   ```
   cargo nextest run -p minibox-core            → green, snapshot tests untouched
   cargo clippy -p minibox-core -- -D warnings  → zero warnings
   cargo xtask check protocol-drift             → if hash-fail: re-run with --update and
                                                  commit the lock alongside (method addition
                                                  only; no wire change)
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "feat(minibox-core): add DaemonResponse::is_terminal predicate [moa-review]"`

### Task 9: Delegate server.rs to the shared predicate

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/server.rs:409-435`
**Run**: `cargo nextest run -p minibox -- server`

1. No new test — behavior pinned by Task 8's table plus existing server tests.

2. Implement: replace the body of `is_terminal_response` with delegation (keep the fn so
   call sites and tests are untouched):

   ```rust
   const fn is_terminal_response(r: &DaemonResponse) -> bool {
       r.is_terminal()
   }
   ```

3. Verify: `cargo nextest run -p minibox` green; `cargo clippy -p minibox -- -D warnings`.

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "refactor(minibox): server terminal check delegates to DaemonResponse::is_terminal [moa-review]"`

### Task 10: Fix crux-plugin dispatch + stale integration test

**Crate**: `minibox-crux-plugin`
**File(s)**: `crates/minibox-crux-plugin/src/lib.rs:105-116`,
`crates/minibox-crux-plugin/tests/integration.rs:575-608`
**Run**: `cargo nextest run -p minibox-crux-plugin`

1. Failing state: `invoke_build_returns_streaming_output` asserts 3 responses with a stale
   "BuildComplete is NOT terminal" comment, while `dispatch()` already treats BuildComplete
   as terminal — run `cargo nextest run -p minibox-crux-plugin -- invoke_build` and record
   the current result before changing anything.

2. Implement:

   - lib.rs:105-115: replace the hand-rolled `matches!` with `resp.is_terminal()`.
     Behavior change: `ContainerCreated` stops being terminal for the plugin — correct per
     the protocol contract; the plugin's run handler then relies on stream close (the
     daemon drops `tx` after `ContainerCreated` for non-ephemeral runs, and `dispatch`'s
     `while let Some(resp) = stream.next()` exits on `None`). Add a run-handler integration
     test pinning that: mock daemon sends `ContainerCreated` then closes; assert InvokeOk
     with the single response.
   - integration.rs:583-591: drop the trailing `DaemonResponse::Success` from the mock
     (BuildComplete is the terminal response), delete the stale comment, and change the
     assertion to `assert_eq!(arr.len(), 2, "BuildOutput + BuildComplete");`.
   - Add two tests pinning `PipelineComplete` and `WorkflowComplete` as terminal through
     `dispatch()` (mock sends the variant followed by a `Success` that must NOT be read;
     assert single/2-element response accordingly).

3. Verify:

   ```
   cargo nextest run -p minibox-crux-plugin        → all green
   cargo clippy -p minibox-crux-plugin -- -D warnings  → zero warnings
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(minibox-crux-plugin): use shared is_terminal predicate; fix stale BuildComplete test [moa-review]"`

---

## Fix Unit F7 — executable security-boundary tests (H9)

### Task 11: ghcr allowlist prefix-squatting tests

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/adapters/ghcr.rs` (tests module, near line 590)
**Run**: `cargo nextest run -p minibox -- allowlist`

1. Write the tests (pure function, no env mutation — mirror kani proof 36 as plain tests):

   ```rust
   #[test]
   fn allowlist_rejects_prefix_squatting() {
       // "org" must not permit "orgevil/image" — slash-bounded prefix only.
       assert!(!allowlist_permits("orgevil/image", "org"));
       assert!(!allowlist_permits("myorgx/image", "myorg"));
       assert!(allowlist_permits("org/image", "org"));
       assert!(allowlist_permits("org", "org"));
   }

   #[test]
   fn allowlist_entry_with_repo_component_is_exact_or_slash_bounded() {
       assert!(allowlist_permits("myorg/private-image", "myorg/private-image"));
       assert!(!allowlist_permits("myorg/private-image-extra", "myorg/private-image"));
   }
   ```

   Run: expected PASS immediately (the code is correct; the gap is executable coverage).
   If either fails, that is a real vulnerability — stop and report before any fix.

2. Verify: `cargo nextest run -p minibox -- allowlist`; clippy clean.

3. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "test(minibox): pin ghcr allowlist slash-boundary as executable tests [moa-review]"`

### Task 12: BindMount parse traversal-rejection tests

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain.rs` (tests module near the existing BindMount tests)
**Run**: `cargo nextest run -p minibox-core -- parse_mount`

1. Write the tests:

   ```rust
   #[test]
   fn parse_volume_rejects_parent_dir_traversal() {
       assert!(BindMount::parse_volume("/tmp/../etc:/mnt").is_err());
   }

   #[test]
   fn parse_mount_rejects_parent_dir_traversal() {
       assert!(BindMount::parse_mount("type=bind,src=/tmp/../etc,dst=/mnt").is_err());
   }

   #[test]
   fn parse_mount_rejects_relative_src() {
       assert!(BindMount::parse_mount("type=bind,src=tmp/data,dst=/mnt").is_err());
   }
   ```

   Expected: PASS immediately (coverage gap, not a code bug). Same rule as Task 11 —
   a failure here is a vulnerability; stop and report.

2. Verify: `cargo nextest run -p minibox-core`; clippy clean.

3. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "test(minibox-core): executable tests for bind-mount path traversal rejection [moa-review]"`

---

## Fix Unit F8 — trust-boundary tests + conformance guard (H10+H11)

### Task 13: crux-plugin privileged passthrough + no-response tests

**Crate**: `minibox-crux-plugin`
**File(s)**: `crates/minibox-crux-plugin/src/lib.rs` (inline tests),
`crates/minibox-crux-plugin/tests/integration.rs`
**Run**: `cargo nextest run -p minibox-crux-plugin`

1. Write the tests:

   ```rust
   #[test]
   fn build_request_run_maps_privileged_true() {
       let req = build_request(
           "minibox::container::run",
           &serde_json::json!({"image": "alpine", "privileged": true}),
       )
       .expect("build_request");
       let DaemonRequest::Run { privileged, .. } = req else {
           panic!("expected Run");
       };
       assert!(privileged);
   }

   #[test]
   fn build_request_run_defaults_privileged_false() {
       let req = build_request(
           "minibox::container::run",
           &serde_json::json!({"image": "alpine"}),
       )
       .expect("build_request");
       let DaemonRequest::Run { privileged, .. } = req else {
           panic!("expected Run");
       };
       assert!(!privileged);
   }
   ```

   Integration test for the no-response path (tests/integration.rs — reuse the existing
   mock-daemon harness): a mock that accepts the connection, reads the request, and closes
   without writing; assert the invoke result is an error containing "no response".

2. Verify: `cargo nextest run -p minibox-crux-plugin`; clippy clean.

3. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "test(minibox-crux-plugin): pin privileged passthrough and empty-response error [moa-review]"`

### Task 14: Fix fail-silent policy env overrides in miniboxd config

**Crate**: `miniboxd`
**File(s)**: `crates/miniboxd/src/config.rs:110-144` (+ tests module)
**Run**: `cargo nextest run -p miniboxd -- env_override`

Current behavior: `MINIBOX_ALLOW_PRIVILEGED=yes` fails `parse::<bool>()` and is silently
dropped — inconsistent with `ContainerPolicy::from_env` (handler/mod.rs:352) which accepts
`1|true|yes`. Align semantics and warn on garbage instead of silently ignoring it.

1. Write failing tests (ENV-mutex-guarded, using the crate's existing env-test pattern —
   check for an existing `ENV_MUTEX`/`unsafe_set_var!` usage in miniboxd tests and follow it):

   ```rust
   #[test]
   fn env_override_accepts_yes_and_one() {
       let _guard = ENV_LOCK.lock().expect("env lock");
       minibox_macros::unsafe_set_var!("MINIBOX_ALLOW_PRIVILEGED", "yes");
       let cfg = DaemonConfig::default().with_env_overrides();
       assert_eq!(cfg.policy.allow_privileged, Some(true));
       minibox_macros::unsafe_remove_var!("MINIBOX_ALLOW_PRIVILEGED");
   }

   #[test]
   fn env_override_warns_and_ignores_garbage_bool() {
       let _guard = ENV_LOCK.lock().expect("env lock");
       minibox_macros::unsafe_set_var!("MINIBOX_ALLOW_PRIVILEGED", "banana");
       let cfg = DaemonConfig::default().with_env_overrides();
       assert_eq!(cfg.policy.allow_privileged, None);
       minibox_macros::unsafe_remove_var!("MINIBOX_ALLOW_PRIVILEGED");
   }

   #[test]
   fn env_override_invalid_u64_leaves_max_image_size_unset() {
       let _guard = ENV_LOCK.lock().expect("env lock");
       minibox_macros::unsafe_set_var!("MINIBOX_MAX_IMAGE_SIZE_MB", "lots");
       let cfg = DaemonConfig::default().with_env_overrides();
       assert_eq!(cfg.policy.max_image_size_mb, None);
       minibox_macros::unsafe_remove_var!("MINIBOX_MAX_IMAGE_SIZE_MB");
   }
   ```

   Run: `cargo nextest run -p miniboxd -- env_override` — `accepts_yes` FAILS today.

2. Implement — shared parser in config.rs:

   ```rust
   /// Parse a boolean-ish policy env value. Accepts 1|true|yes / 0|false|no
   /// (case-insensitive). Unrecognised values are rejected with a warning —
   /// never silently ignored on a security-policy variable.
   fn parse_policy_bool(name: &str, v: &str) -> Option<bool> {
       match v.trim().to_lowercase().as_str() {
           "1" | "true" | "yes" => Some(true),
           "0" | "false" | "no" => Some(false),
           other => {
               tracing::warn!(var = name, value = other, "config: unrecognised boolean value ignored");
               None
           }
       }
   }
   ```

   Use it for both `MINIBOX_ALLOW_PRIVILEGED` and `MINIBOX_ALLOW_BIND_MOUNTS` arms; add a
   `warn!` on the unparseable-u64 arm for `MINIBOX_MAX_IMAGE_SIZE_MB`.

3. Verify: `cargo nextest run -p miniboxd`; clippy clean.

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(miniboxd): warn on unparseable policy env overrides; accept 1/true/yes [moa-review]"`

### Task 15: Injectable aggregate image-size limit + over-limit test

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/image/registry.rs` (limit at line ~807 and struct field)
**Run**: `cargo nextest run -p minibox-core -- total_size`

1. Write failing test (wiremock, mirroring the existing pull tests near registry.rs:2719):

   ```rust
   #[tokio::test]
   async fn pull_rejects_when_actual_bytes_exceed_total_limit() {
       // Two small layers whose combined actual bytes exceed a tiny injected limit.
       // Serve real gzipped tar layers (reuse this file's existing layer fixture
       // helper); set client.max_total_image_size = <bytes of layer 1 + 1>.
       // Assert the pull error mentions "total size limit".
   }
   ```

   Run: FAIL (no injectable limit exists).

2. Implement: add `max_total_image_size: u64` to `RegistryClient` (default
   `MAX_TOTAL_IMAGE_SIZE` in `new()`), replace the const at the line-807 comparison with
   `self.max_total_image_size` (thread through to the closure as a captured local), and add
   a `#[cfg(any(test, feature = "test-utils"))] pub fn with_max_total_image_size(mut self, n: u64) -> Self`.

3. Verify: `cargo nextest run -p minibox-core`; clippy clean; confirm the default is still
   the const (add `assert_eq!(RegistryClient::new(...)?.max_total_image_size, MAX_TOTAL_IMAGE_SIZE)`
   to an existing constructor test).

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(minibox-core): make aggregate image size limit injectable; test over-limit bail [moa-review]"`

### Task 16: Conformance zero-test guard

**Crate**: `minibox-testsuite` (+ `xtask`)
**File(s)**: `crates/minibox-testsuite/src/harness/runner.rs`,
`crates/minibox-testsuite/src/bin/run_conformance.rs`,
`crates/minibox-testsuite/src/bin/generate_report.rs`
**Run**: `cargo nextest run -p minibox-testsuite`

1. Write failing tests:

   ```rust
   #[test]
   fn inventory_collects_expected_test_count() {
       // Pin the floor so a dropped adapter module or stripped inventory ctor
       // cannot silently zero the suite. Update EXPECTED_MIN when adding suites.
       const EXPECTED_MIN: usize = 28; // current count — verify with the run below
       let runner = TestRunner::collect_inventory();
       assert!(
           runner.len() >= EXPECTED_MIN,
           "conformance inventory collapsed: {} tests (expected >= {EXPECTED_MIN})",
           runner.len()
       );
   }
   ```

   Add `pub fn len(&self) -> usize` + `pub fn is_empty(&self) -> bool` to `TestRunner`
   if absent. Determine the real current count first:
   `cargo run -p minibox-testsuite --bin run_conformance 2>&1 | tail -5` and set
   `EXPECTED_MIN` to that count.

2. Implement the runtime guard in both bins: after collection/filtering, bail before
   reporting success:

   ```rust
   anyhow::ensure!(
       !runner.is_empty(),
       "conformance runner collected 0 tests — inventory registration is broken \
        (dropped adapter module or stripped linker section)"
   );
   ```

3. Verify: `cargo nextest run -p minibox-testsuite`; run the conformance bin once and
   confirm the count matches; clippy clean.

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "fix(minibox-testsuite): guard against silently-empty conformance inventory [moa-review]"`

---

## Fix Unit F6 + D2 — workspace hygiene (H8)

### Task 17: Register `crates/ail` in the workspace

**Crate**: `ail`, workspace root
**File(s)**: `Cargo.toml` (members), `crates/ail/Cargo.toml`
**Run**: `cargo check --workspace`

1. Failing state: `cargo check -p ail` errors ("not included in the workspace").

2. Implement:

   - Root `Cargo.toml` members: add `"crates/ail",` after `"crates/smolbox",`.
   - `crates/ail/Cargo.toml`:

     ```toml
     [package]
     name = "ail"
     version.workspace = true
     edition.workspace = true
     license.workspace = true
     rust-version.workspace = true
     repository.workspace = true
     publish = false
     ```

   - If xtask gate package lists enumerate crates (`grep -n "\-p " xtask/src/gates.rs`),
     add `ail` where the other bin crates appear.
   - Fix any clippy findings this exposes in `crates/ail/src/main.rs` (dead `TraceRecord`:
     remove it — placeholder code should be minimal, not dead-pub).

3. Verify: `cargo check --workspace` + `cargo clippy --workspace -- -D warnings` clean.

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "chore(ail): register crate in workspace; adopt workspace package fields [moa-review]"`

### Task 18: Bump workspace version 0.30.0 → 0.31.0

**Crate**: workspace root
**File(s)**: `Cargo.toml:21` (`[workspace.package] version`), `Cargo.toml:77`
(minibox-core pinned version), `Cargo.toml:79` (minibox-macros pinned version)
**Run**: `cargo check --workspace`

Covers all six semver breaks in publishable crates (port-trait signature changes to
`DynProgressSink`, removed `Default for RegistryClient`, new `DomainError::ContainerNotStopped`
variant, reqwest 0.13 in public API) plus minibox-macros' new macros.

1. No test — verification is the workspace check plus a version assertion:
   `grep -n '0\.30\.0' Cargo.toml` must return nothing after the edit.

2. Implement: change all three `0.30.0` occurrences to `0.31.0`. Do NOT add
   `#[non_exhaustive]` to `DomainError` in this task (deferred — separate decision; it is
   itself a breaking change and ripples through in-workspace exhaustive matches).

3. Verify:

   ```
   cargo check --workspace                    → clean (Cargo.lock updates)
   cargo nextest run -p minibox-core          → green
   git diff --stat                            → Cargo.toml + Cargo.lock only
   ```

4. Run: `git branch --show-current` (must be `develop`).
   Commit: `git commit -m "chore(workspace): bump version to 0.31.0 for minibox-core API changes [moa-review]"`

---

## Execution order

Wave 1 (independent, parallel-safe — disjoint files): Task 1+2 (one agent, xtask),
Task 3+4 (one agent, compile fixes), Task 5+6 (one agent, policy), Task 8+9+10 (one agent,
terminal predicate), Task 11+12 (one agent, security tests).

Wave 2 (after wave 1 merges): Task 7 (reconcile), Task 13+14+15+16 (trust-boundary tests +
conformance guard — 13/14/15/16 touch disjoint crates and may split across two agents),
Task 17, Task 18 (last).

Final gate after all tasks: `cargo xtask verify` + `cargo nextest run --workspace` +
`cargo check --workspace --all-targets --target aarch64-unknown-linux-gnu`.

Deferred (tracked in `.ctx/review/00-synthesis.md`, not in this plan): all MED/LOW findings,
`#[non_exhaustive]` on `DomainError`, D1 (`ci.yml` stays untracked per user decision), D3
(kani/fuzz CI wiring — Task 11/12 mirror the critical proofs as plain tests).
