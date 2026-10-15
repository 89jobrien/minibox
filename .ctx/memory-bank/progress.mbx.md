# Progress

## What works

### Core container lifecycle (Linux native adapter)

- **Pull**:
    - OCI image pull from Docker Hub v2 + ghcr.io,
    - anonymous auth,
    - parallel layer downloads,
    - content-addressed caching
- **Run**:
    - Full namespace isolation (PID, NET, UTS, IPC, MNT),
    - cgroups v2 resource limits (memory, CPU weight, PID limit),
    - overlay filesystem layering
- **Exec**:
    - setns-based exec with PTY support (`-it`),
    - interactive terminal
- **Stop/rm**:
    - Graceful shutdown with signal forwarding,
    - cleanup of cgroups + overlay mounts
- **ps**:
    - List running/stopped containers with status, image, command, created
- **logs**:
    - Stored stdout/stderr capture per container
- **events**:
    - `minibox events` streams lifecycle events via event bus
- **pause/resume**:
    - cgroup.freeze-based pause/resume
- **Bind mounts**:
    - `-v`/`--mount` with host path validation
- **Privileged mode**:
    - `--privileged` flag
- **Bridge networking**:
    - veth pairs,
    - NAT via iptables DNAT
    - `MINIBOX_NETWORK_MODE=bridge`
- **Image management**:
    - prune / rmi with lease-based ImageGarbageCollector
- **Image build pipeline**:
    - push,
    - commit,
    - build (experimental)
- **Port forwarding**:
    - DNAT via iptables in bridge network mode
    - `BridgeNetwork::apply_port_mappings`
    - teardown on cleanup
- **DNS config**:
    - defaults to 8.8.8.8 + 8.8.4.4,
    - configurable per container
- **Execution manifest**:
    - persisted before spawn,
    - workload digest,
    - env value
    - SHA-256 (never plaintext),
    - policy evaluation via `mbx verify`
- **OTEL trace export**:
    - full OTLP/gRPC exporter via opentelemetry 0.32,
    - batch export, graceful fallback,
    - OtelGuard shutdown
    - Wired in miniboxd main.
- **Auth-policy gate**:
    - `ContainerPolicy` deny-by-default for bind mounts and privileged mode
    - Env-var opt-in (`MINIBOX_ALLOW_BIND_MOUNTS`, `MINIBOX_ALLOW_PRIVILEGED`)
    - `validate_policy()` enforced on every run request

### GKE adapter

- Unprivileged pod execution via proot + copy-FS
- Image pull/push support via OciPushAdapter

### macOS adapters

- **smolvm** (default): run/stop/ps via lightweight Linux VM
- **krun**: run/stop/ps via libkrun VM (fallback when smolvm absent)
- **colima**: run/stop/ps via Lima VM (Intel Macs); `ColimaContainerCommitter` added for
  commit/save (nerdctl commit/save -> docker-archive import) (1ae7528e)
- Limitations: exec/logs not supported on any macOS adapter

### Testing infrastructure

- 96 integration test files under `crates/*/tests/`
- ~1,060 integration-test annotations + ~1,228 inline test annotations
- 19 security regression tests pinning all 12 invariants
- ~46 proptest property tests (protocol roundtrip, cgroup bounds, daemon state)
- 123 conformance tests (backend-agnostic adapter and port contracts)
- 19 borrow-reasoning fixtures (must-pass/must-fail)
- 15 e2e daemon+CLI tests (Linux+root)
- 16 cgroup integration tests (Linux+root)
- ~17 sandbox tests, 30 CLI subprocess tests
- Conformance suite generates Markdown/JSON reports
- e2e showcase suite (`crates/minibox-testsuite/src/showcase/`) — narrated demo
  scenarios (e.g. lifecycle: run/stop/rm), backs a CLI demo mode; xtask CLI schema
  at `xtask/schema/cli.schema.json` (02194fd9, 3b9b85bd)
- **Platform-gated coverage gap:** `crates/minibox/src/lib.rs:57` declares
  `#[cfg(target_os = "linux")] pub mod container;`, so the cgroups, namespace,
  mount/seccomp, and overlay unit tests are not compiled into a darwin build at all. Any
  macOS gate run has zero coverage of them. The QEMU aarch64 harness closes the gap:
  `rust-script scripts/qemu-linux-tests/run.rs --filter <module>` runs the `minibox` **lib**
  test binary on a real Linux kernel (`cargo xtask test-in-vm` does *not* — its script
  collects only integration test binaries). Its 3 `container::filesystem::bind_mount_tests`
  failures are expected: `mount_setattr(MOUNT_ATTR_IDMAP)` needs a user namespace the
  `virt` kernel denies. Documented in `scripts/qemu-linux-tests/README.md`.

### Gate behavior

- `cargo xtask prepush` is the gate that builds, lints, and tests: musl cross-build,
  release build, release lib tests, vm_checkpoint, and the 123-test conformance suite.
- `cargo xtask pre-commit` is conditional. `xtask/src/gates.rs:245` wraps fmt, clippy, and
  the architecture check in `if rust_staged`, so with no Rust staged it runs docs-lint only
  and still prints "pre-commit checks passed". It runs no release build and no tests.
  A gate that skips its work should report the skip rather than a generic pass.

### CI pipeline

- 15 GHA workflows covering CI/PR/merge, macOS, conformance, context snapshots,
  stability and protocol gates, clippy review, nightly audits, promotion, releases,
  package publishing, and issue summaries
- Self-hosted runner on VPS for Linux-specific tests
- Pre-commit/pre-push local gates via cargo xtask

### Developer tooling

- `mbx doctor` — preflight diagnostics (compiled adapters, capabilities)
- `mcp` / `minibox-mcp` — MCP stdio server exposing agent-safe daemon tools
  for doctor, ps, images, logs, manifest, pull, run, stop, and rm
- `mbx tui` — new read-only `minibox-tui` crate (ratatui + crossterm): live container table
  (polls `DaemonRequest::List` every 1s) and live-tailing lifecycle event log
  (`DaemonRequest::SubscribeEvents`); no run/stop/exec by design (adf70510)
- `mbx completions` — hidden subcommand generating a Nushell completion script via
  `clap_complete`/`clap_complete_nushell`, sourced through `nu_libs.nu` (b9a84847)
- `cargo xtask musl-check` — new prepush gate catching `cfg(target_os = "linux")` build
  failures against the musl target before CI (1ae7528e)
- `cargo xtask ci-watch` — watch GHA run status with job-level detail
- `cargo xtask promote` — branch cascade (develop->staging->release->main)

## Recently completed
- **`mbx build` COPY/VM-bridging gap fixed on the smolvm adapter**
  (2026-10-15, uncommitted) — `COPY`/`ADD` wrote to minibox's host-side
  layer-snapshot mechanism, but RUN steps on smolvm execute inside a
  persistent build VM with its own separate disk that never saw that
  host-side content, so a `RUN` depending on a file just `COPY`'d (e.g. `RUN
  chmod +x entrypoint.sh` after `COPY entrypoint.sh`) failed with `chmod:
  cannot access '...': No such file or directory`. Fixed by adding
  `ContainerRuntime::sync_persistent_machine_path` (default no-op,
  `crates/minibox-domain/src/runtime.rs`), implemented on `SmolVmRuntime`:
  ensures the persistent machine exists (via a new shared
  `ensure_persistent_machine` helper factored out of `spawn_process`),
  `mkdir -p`s the destination's parent inside the VM (COPY/ADD destinations
  get no automatic mkdir the way WORKDIR+RUN does), then copies the file in
  via `smolvm machine cp`. `builder.rs`'s `Instruction::Copy`/
  `Instruction::Add` arms call this through a new
  `sync_copy_to_build_machine` helper, resolving the destination with the
  same `copy::resolve_container_path` (made `pub(super)`) that `copy_sources`
  uses internally. One deployment mistake caught during verification (not a
  code bug): this workspace's `.cargo/config.toml` sets `target-dir` to
  `~/.minibox/cache/target`, so `cargo build --release` does not write to
  `target/release/` at the repo root — copying from that path redeployed a
  stale pre-fix binary that looked like a successful rebuild (exit 0, correct
  compile output) until binary mtimes were checked against source mtimes.
  After redeploying from the correct path, a full rebuild+redeploy+rerun
  against the real `ops/ci-runner/Dockerfile` completed all 11 steps
  (`Successfully built`/`Successfully tagged`), and `smolvm machine ls`
  showed no lingering `minibox-build-*` entry afterward.
- **`mbx build` RUN-step chaining fixed on the smolvm adapter** (2026-10-14,
  uncommitted) — every RUN step after the first used to boot a fresh,
  unrelated VM from the hardcoded `DEFAULT_IMAGE = "ubuntu:24.04"` instead of
  continuing from the previous step's modified rootfs, because
  `execute_run_step` passed `image_ref: None` and `SmolVmRuntime` had no way
  to reuse a VM across steps at all. Fixed by giving `SmolVmRuntime` a
  persistent-named-machine path (`smolvm machine create/start/exec`,
  mirroring the existing krun `KrunExecRuntime` pattern) keyed by the
  hostname `builder.rs` already sets identically per build stage, booted from
  the stage's actual FROM image via a new `StageState.source_image_ref`
  field. A new `ContainerRuntime::cleanup_persistent_machine` trait method
  (default no-op, overridden by `SmolVmRuntime`) tears the VM down
  unconditionally after `build_image`'s instruction loop. Two bugs found and
  fixed during live verification: `smolvm machine create` needs `--net` at
  create time (not start time), and `smolvm machine delete` silently no-ops
  with exit 0 when run non-interactively without `--force` — the second one
  required a full rebuild+redeploy+rerun cycle to actually confirm fixed
  (grepping the binary's strings for `--force`, not just reading the source
  diff, is what caught that the first "fixed" redeploy hadn't actually picked
  up the edit). Verified end-to-end against the real `ops/ci-runner/Dockerfile`:
  steps 2-9 all succeed (apt-get install, rustup via curl|sh against the
  *correct* base with step 2's packages present, `cargo install
  cargo-nextest` compiling, ARG/WORKDIR, GH-runner tarball download), and the
  persistent build machine is confirmed absent from `smolvm machine ls`
  immediately after the build request completes (success or failure). See
  the COPY/VM-bridging gap below for the next, separate blocker.
- **GitHub Actions workflow audit series closed out** (commit `87ce6ef8`, 2026-10-13) —
  full audit of all 14 `.github/workflows/*.yml` files, across several passes this
  session. Final fixes: dropped the duplicate `handler-coverage` job from
  `stability-gates.yml` (nightly.yml already runs the same `coverage-check` on
  schedule), moved `macos.yml`'s `fmt-check` job off `macos-latest` to
  `ubuntu-latest` (rustfmt is platform-independent), aligned `protocol-drift.yml`'s
  branch wildcards with the rest of the repo, and consolidated every
  `taiki-e/install-action` pin repo-wide onto one verified SHA (v2.85.10) — 4 stray
  SHAs plus an unpinned `@v2` in `merge.yml`. Earlier in the same series: deleted
  `rust-clippy.yml` and `promote.yml` (redundant clippy run + competing promotion
  mechanism), deleted `context-snapshot.yml` (structural 100%-failure validator,
  see Known Issues), disabled 4 dead self-hosted-runner jobs in `merge.yml`,
  rescoped `.coderabbit.yaml` to staging-only, added a native `build-macos` release
  job. All CI green on `develop` post-push (macOS, Stability Gates, Conformance,
  Merge, CI).
- **Release: develop -> staging -> release -> main** (PR #592, merge commit `30e60ec2`,
  2026-10-12T16:26:57Z) — promoted the 3 overlay rstest-fixture-ordering fixes plus 5
  accumulated CI/protocol-drift-lock fixes to `main`. `staging -> release` needed
  `--skip-ci-check` because the self-hosted runner had 0 instances registered against the
  repo, stalling the `Merge` workflow's Linux-root jobs indefinitely on both `staging` and
  `release` (all other checks were green). `release -> main` went through PR #592 rather
  than a direct push per branch protection, and merged while non-required checks were
  still running (`mergeStateStatus: UNSTABLE`, not `BLOCKED`) — required checks (`CI
  passed`, `Stability Gates`, `actionlint`) were all green. Runner location is unresolved;
  it is confirmed not on jobrien-vm despite prior documentation assuming it was.
- **Mock system duplication resolved (t23)** — `crates/minibox/src/adapters/mocks.rs` is now an
  11-line `pub use minibox_core::adapters::mocks::*;` re-export; the ~1000-line duplicate
  reimplementation this entry used to describe is gone. Verified by direct read 2026-10-08.
- **cgroup root validation fix** — `cgroup_root()` no longer passes an unvalidated
  `MINIBOX_CGROUP_ROOT` into `PathBuf::from`. An empty value previously produced a relative
  container path, so privileged-container `mkdir` calls landed in the daemon's cwd and left a
  real leaked cgroup at the repo root (`813f2623a2ee4712/` containing `cgroup.subtree_control`
  and an `init/` leaf), which had been masked by a `.gitignore` entry. Both the env var and the
  client-supplied `cgroup_parent` now route through the existing `validate_cgroup_parent` via a
  pure `resolve_cgroup_root()`; 8 env-free regression tests; `delegate_subtree` failure promoted
  from `debug!` to `warn!`. Verified 17/17 on `container::cgroups::` under the QEMU Linux
  harness (1db50b86).
- **Protocol drift expectation fix** — xtask's expected surface registry updated to track the
  already-split `domain-*` entries instead of the stale single `domain-ports` entry; file-level
  clippy allow for unwrap/expect/panic in `cli_subprocess.rs` (fe9bae3e).
- **Colima commit adapter + image lease conformance** — `ColimaContainerCommitter`
  (nerdctl commit/save -> docker-archive import), `ImageLeaseService` port conformance suite +
  `InMemoryLeaseService` test double, `ContainerRecord.upper_dir`/`merged_dir` populated from
  rootfs metadata, `xtask musl-check` prepush gate (1ae7528e).
- **minibox-tui crate** — new read-only TUI dashboard crate, `mbx tui` subcommand, 6 unit tests
  covering App state transitions, live-smoke-tested against `miniboxd` (adf70510).
- **Nushell completion generation** — `clap_complete_nushell`-backed hidden `completions`
  invocation intercepted before clap parsing (b9a84847).
- **MCP control surface first slice** — new `crates/mcp` workspace package
  `minibox-mcp`, with Rust library and binary names set to `mcp`. Exposes
  typed stdio MCP tools backed by `minibox-core::client::DaemonClient`,
  policy-gated mutating/high-risk options, miette diagnostics, and unit +
  stdio integration tests.

- **smolvm async/sync boundary fix** — `SmolVmRegistry`/`SmolVmRuntime::vm_exec` and the
  spawn_process command path ran `std::process::Command::output()` inline in async fns with
  no `spawn_blocking`, starving the tokio worker on long-running VM ops (boot+pull+workload
  can exceed a minute). Fixed and confirmed via live repro: `mbx ps` stays responsive during
  a backgrounded long-running container (94f227b9).
- **mbx pause/resume + ps polling fixes** — corrected terminal response handling for
  pause/resume and the ps polling parser (e5c40152).
- **e2e showcase suite** — narrated demo scenarios in
  `crates/minibox-testsuite/src/showcase/`, xtask CLI schema
  (`xtask/schema/cli.schema.json`); lifecycle scenario fixed to expect `rm` after `stop`
  on an ephemeral run (02194fd9, 3b9b85bd).
- **Docs audit fixes** — crate count, doc links, stale version refs, domain.rs
  attribution, path prefixes, test file counts corrected (10a03d62, d9121ac3).
- **minibox-bench crate** — dedicated Criterion benchmark crate with 8 hot-path
  targets (layer_extract, image_pull, linux_rootfs, cgroup, spawn), Justfile,
  `cargo xtask bench --check` regression gate, nightly CI job with self-hosted
  runner preference (b9df139f, 2dd9d4ca, 1eee8706, 5fa390cb).
- **MoA review HIGH fixes** — resolved F1-F8/D2 findings across two waves;
  workspace bumped to v0.31.0; ail + minibox-bench crates registered (54510f59,
  8b842b53).
- **conformance_test! macro** — replaces ConformanceTest boilerplate in
  minibox-testsuite; design doc at docs/designs/2026-07-01-conformance-macro-design.md
  (4ce6ce9f).
- **miette diagnostics** — rich CLI error rendering via miette; plan doc at
  docs/plans/2026-07-07-structured-errors-miette.md (cf37b05a).
- **PR-based auto-promote CI** — cascade develop->staging->release->main via PR
  workflow (c1a16d8e).
- **Open PR merge pass + final verification** — #462, #460, #459, #464, and
  #324 merged; open PR list empty. `cargo xtask verify` passed for task `t12`
  on 2026-07-26, but the verify checkpoint was not recorded because the
  checkout remains dirty with pre-existing work.
- **Workspace-wide clippy sweep** — 64 Linux-only warnings resolved (826a54ed).
- **Rustqual SRP sweep** — workspace-wide SRP_PARAMS/FRAGMENT/BOILERPLATE/TQ
  elimination (8212a9a4).
- **Mistakes ledger** — .ctx/memory-bank/mistakes.md (30 recurring patterns).
- **Python code removal** — removed all 15 Python files + uv.lock from scripts/.
  ai-review.nu now calls ai-review.rs (rust-script). Gitea CI diagnose job
  removed. No Python remains in the project.
- **Dead script cleanup** — removed gen-class-diagrams.py, install-claude-skills.sh,
  demo-smolvm.sh, agent_hello_world.py (no callers found).
- **14-doc audit and fix** — verified all 20 docs/core/ files against code.
  Fixed 19 critical errors and 10 stale references across 14 docs:
  smolbox crate visibility, minibox-conformance->minibox-testsuite rename,
  version 0.24.0->0.30.0, vz adapter removal, DoS limit corrections,
  select_adapter->adapter_from_env, stable->staging, minibox-cli->mbx,
  SO_PEERCRED/handler file attributions.
- **Rustqual SRP sweep** — RunParams extraction, named constants in miniboxd,
  dead code removal, duplicate dedup (lima_exec, lifecycle handlers),
  long function extraction across core+minibox+miniboxd (791e81a..dd3e9a6)
- **test-in-vm xtask** — dual backend minibox+smolvm, pull tests pass (41279b1)
- **smolvm output_reader fix** — fixed adapter output streaming (747b636)
- **CI fixes** — InternalPath/PathBuf mismatches, hyper pin, RunParams refactor
  fallout (c7a82bf, 64beade, 0c8e693, b930406, a179c64)
- **TODO markers** — GitHub issue refs from code review (b54d05d)
- **Exhaustive small-domain tests** — path validation edge cases (870aa9f)
- **Protocol/domain roundtrip property tests** — proptest coverage (6277bde)
- **Test helper cleanup** — fixes #403-#409 (f8f677a)

## In progress

- macOS exec/logs via VM adapters — container run + stdout streaming works
  (smolvm/krun). `exec_runtime: None` on both means exec-into-running is
  unsupported. No historical log retrieval.
- Promote develop -> staging (pending CI green on develop)

## Not started / backlog

### Partially done

- **Container-in-container** (DinD):
    - UX — works today via privileged mode + bind mounts (full e2e test in system_tests.rs). Missing: turnkey `--dind`
      flag, auto-provisioned inner daemon, socket forwarding, shared image cache
- **Capability dropping**:
    - Grant path implemented (`apply_full_capabilities` with curated whitelist minus SYS*MODULE/SYS_BOOT/MAC*\*)
    - Drop path for unprivileged containers missing (no default allowlist like Docker's ~14 caps)
- **User namespace remapping**:
    - `RuntimeCapabilities.supports_user_namespaces` field exists on every adapter
    - no `CLONE_NEWUSER`
    - no uid_map/gid_map writing
    - no newuidmap helper
    - purely informational today
- **Windows (WSL2)** adapter:
    - ~40% scaffolded. `Wsl2Runtime`/`Wsl2Filesystem`/`Wsl2Limiter` implement traits via `wsl.exe` delegation
    - `winbox` crate has module stubs + conformance test
    - Not wired into miniboxd
    - no Named Pipe server
    - no `minibox-wsl-helper` binary exists

### Not started

- **Seccomp filters**:
    - zero code
    - no BPF profiles
    - no syscall filtering
- **Rootless support**:
    - blocked on user namespace impl
    - no slirp4netns
    - no fuse-overlayfs
    - no unprivileged cgroup delegation
- **CRI compliance**:
    - zero protobuf/gRPC
    - no RuntimeService/ImageService
- **Image pull size limits**:
    - per-layer 10 GiB enforced
    - aggregate 50 GiB budget enforced across each pull
- **ValidatedPath newtype**:
    - all validation is function-call based (`validate_layer_path()`)
    - no type-level guarantee
    - documented in SECURITY_INVARIANTS.md as a consideration

## Known issues
- **Self-hosted GHA runner is gone, not just offline** — the `Merge` workflow's
  self-hosted Linux-root jobs (`integration tests`, `e2e tests`, `cargo
  audit/deny/machete`, `cleanup (Linux)`, `protocol e2e (Linux)`) sit `pending`
  indefinitely because `gh api repos/89jobrien/minibox/actions/runners` returns
  `{"total_count":0,"runners":[]}`. Confirmed 2026-10-12: this runner lived on a
  separate VPS (not jobrien-vm) that has since been shut down — there is no host
  to restart it on. `.claude.local.md`'s GHA-runner section (`systemctl --user
  restart 'actions.runner.*'` on jobrien-vm) describes a *different* runner setup
  and is stale for this purpose; do not chase it as a fix. A replacement runner
  needs to be built from scratch, not restarted. Workaround until one exists:
  `cargo xtask promote --skip-ci-check` for any staging/release hop.
  Direction under discussion: host it as an ephemeral minibox container
  (`--ephemeral` GHA runner registration, one job per container, supervisor loop
  mints a fresh registration token per start) rather than a persistent VM
  process. Split by privilege: light jobs (clippy/fmt/unit/docs) can self-host on
  minibox; the Linux-root integration/e2e suite needs `--privileged` + cgroup
  delegation, which is circular (using minibox to test minibox's own privileged
  path) — likely keep that one on real hardware/VM instead.

  Design sketch lives at `ops/ci-runner/` (Dockerfile, entrypoint.sh,
  supervisor.sh, systemd unit) — not yet exercised end-to-end against a live
  GitHub repo. Corrected 2026-10-14: an earlier session wrongly assumed this
  needed Docker/Colima to build the image locally on macOS (neither is
  installed, and repo convention forbids Colima for Linux container work).
  `mbx` itself has `build`/`push`/`commit` subcommands — confirmed present and
  functional this session, no Docker required. `mbx build` builds via
  whatever adapter the daemon selected (krun/smolvm), so it needs the same
  adapter capability as `mbx run`, not a separate container engine.
  `mbx push` supports `--token`/`--username`+`--password` credentialed
  pushes; `mbx pull` (confirmed via `--help`) has no such flags and is
  anonymous-only, so the ghcr.io image repo still must be public once
  consumed via `mbx pull`/`mbx run`. See `ops/ci-runner/README.md` for the
  corrected build/push/run instructions, including a local-macOS-via-krun
  path to use while no Linux host exists.
- **GitHub Actions is fully non-functional** — every recent run on `develop` and `main`
  fails in 3-10s with "The job was not started because your account is locked due to a
  billing issue." No workflow can validate a push until that clears (observed 2026-10-03).
- **`validate_lima_paths` allowlist bypass on empty or `/` HOME** —
  `Path::new("").starts_with(anything)` is `true`, so when `HOME` is empty or `/` the
  Colima bind-mount host-path allowlist accepts *every* path. Pinned by the failing
  `validate_lima_paths_rejects_opt` test, which passes on a dev machine only because
  `$HOME` is a real directory. Same bug class as the cgroup root issue fixed in 1db50b86.
  Not yet filed as an issue.
- **`cargo xtask pre-commit` can report success without verifying anything** —
  `gates.rs:245` gates fmt/clippy/architecture on `if rust_staged`. CLAUDE.md also
  overstates it as including a release build. Prefer `cargo xtask prepush`.
- **Duplicate memory banks** — canonical `.ctx/memory-bank/` (`*.mbx.md`) coexists with a
  stale `.ctx/godmode/memory-bank/` (~2 weeks old). The `mistake-tracker` skill still writes
  to the legacy path, so ledger entries can land in the wrong file.
- Stabilization freeze (#127) closed 2026-08-22 — 5/6 acceptance sub-issues done
  (#122 protocol, #114 state, #120 docs, #123 security, #117 support tiers, #133
  CI enforcement); #116 (handler.rs coverage) remains open and tracked separately
- krun metrics inconsistency — constructs its own `NoOpMetricsRecorder`
  internally rather than accepting the shared broker
- Colima path validation bypass via `..` (P1)
- Bind-mount teardown absent on container cleanup (P2)
- `xtask` binary not currently available (preflight reports fail)
- CI coverage gaps — sandbox tests and the feature-gated CLI subprocess suite are
  not in any CI workflow
- macOS VZ.framework — restored behind the opt-in `vz` feature, but VM boot remains
  blocked by `VZLinuxBootLoader` failures on current macOS
