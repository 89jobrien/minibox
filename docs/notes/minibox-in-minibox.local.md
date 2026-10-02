# Minibox-in-Minibox (DinD) Analysis

Last updated: 2026-10-02

---

## Current State: Works ONLY on the `native` Linux adapter

The headline finding from a live run on this machine: **the DinD test cannot run
on macOS at all**, regardless of policy configuration. It requires the `native`
Linux adapter (Linux namespaces + overlayfs + cgroups v2), which needs a real
Linux host with root.

The daemon's own `GetCapabilities` response reports, for the `smolvm` backend:

| Capability            | smolvm status         |
| --------------------- | --------------------- |
| `bind_mounts`         | `unsupported`         |
| `privileged_mode`     | `unsupported`         |
| `overlay_fs`          | `unsupported`         |
| `cgroups_v2`          | `provided_by` (`vm`)  |
| `pid_namespace` & co. | `provided_by` (`vm`)  |

Verified behavior on this machine (smolvm adapter, policy enabled):

| Request                                    | Observed result                        |
| ------------------------------------------ | -------------------------------------- |
| `mbx run alpine -- /bin/echo hi`           | works, output streams                  |
| `mbx run --privileged alpine -- /bin/echo` | **silently ignored**, reports success |
| `mbx run -v /tmp/src:/mnt/x alpine -- cat` | **exit 1, no diagnostic, no output**   |
| DinD test (privileged + 5 bind mounts)     | container stuck in `Created`, no output |

The privileged path is not degraded-but-working on smolvm — it is a silent
no-op for `--privileged` and a silent failure for `-v`. Nothing in the run path
warns that the requested capability is unavailable.

### The harness advertises a path that does not exist

`xtask/src/test_in_vm.rs` picks a backend in `detect_backend` and decides
privilege with `VmBackend::is_privileged()`:

- `is_privileged()` returns `true` for the `Minibox` backend based **only** on
  whether `MINIBOX_ALLOW_BIND_MOUNTS` / `MINIBOX_ALLOW_PRIVILEGED` are set
  (`test_in_vm.rs:66-76`). It never consults the adapter's actual capability
  matrix.
- Because of that, `build_test_script(..., privileged = true, ...)` includes
  `system_tests`, `cgroup_tests`, and `sandbox_tests` (`test_in_vm.rs:550-556`).
- On macOS the underlying adapter is smolvm, which cannot honor those requests.

Result: the suite list says privileged, the adapter cannot deliver, and the run
hangs or silently no-ops with no error pointing at the real cause.

### What works (native Linux adapter, root, cgroups v2)

| Component                  | Status  | Notes                                   |
| -------------------------- | ------- | --------------------------------------- |
| Privileged mode            | Done    | `capset(2)` grants curated cap set      |
| Cap exclusion list         | Done    | SYS_MODULE, SYS_BOOT, MAC_OVERRIDE/ADMIN |
| Bind mounts into container | Done    | `-v host:container[:ro]` syntax         |
| Cgroup delegation (auto)   | New     | Auto-delegated when `--privileged`      |
| `--cgroup-parent` flag     | New     | Explicit parent slice selection         |
| `/dev` population          | New     | tmpfs + host device bind mounts         |
| Proc mount                 | New     | Explicit, `nosuid,nodev,noexec`         |
| Nesting depth limit        | New     | `MINIBOX_NEST_DEPTH` / `MAX_NEST_DEPTH` |
| Nested overlay probe       | New     | Empirical overlay-on-overlay detection  |
| Cgroup root override       | Done    | `MINIBOX_CGROUP_ROOT` env override      |
| Preflight cgroup probe     | Done    | `cgroup_subtree_delegatable` check      |
| Port forwarding            | Missing | No implementation anywhere              |
| User-facing docs           | Missing | No `docs/NESTING.md`                    |

### Two divergent test implementations

| Implementation       | Location                                            | Sets `ALLOW_*`? |
| -------------------- | --------------------------------------------------- | --------------- |
| Integration test     | `crates/miniboxd/tests/system_tests.rs:504`         | **no**          |
| Showcase scenario    | `crates/minibox-testsuite/src/showcase/mounts_privileged.rs` | yes, via `spawn_daemon_with_env` |

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

The inner daemon's data dir is on host tmpfs (via bind mount), not the outer
container's overlay, which avoids the kernel's overlay-on-overlay limitation.

---

## Enabling privileged mode (three levels)

Policy is deny-by-default. `validate_policy`
(`crates/minibox/src/daemon/handler/mod.rs:383`) rejects bind mounts and
privileged requests before the runtime adapter ever sees them.

| Mechanism              | Where                                                | Notes                     |
| ---------------------- | ---------------------------------------------------- | ------------------------- |
| Env vars               | `MINIBOX_ALLOW_BIND_MOUNTS`, `MINIBOX_ALLOW_PRIVILEGED` | Accepts `1/true/yes`; highest |
| Config file            | `/etc/minibox/config.toml`, `~/.config/minibox/config.toml` | `crates/miniboxd/README.md:60` |
| **Project config (new)** | `./minibox.toml`, walked up from CWD                 | Lowest precedence; gitignored |

```toml
# ./minibox.toml — gitignored, per-checkout
adapter = "smolvm"
log_level = "info"

[policy]
allow_privileged = true
allow_bind_mounts = true
```

Load order is project -> system -> user -> env, so a checked-in file can never
weaken a system-level lockdown, and `MINIBOX_ALLOW_PRIVILEGED=0` always wins.
Verified at runtime: project alone enabled policy from a nested subdirectory; a
user config then overrode it to `false`; an env var also overrode it.

### Dead code: the `dev` profile

`DaemonConfig::profile("dev")` (`config.rs:89`) sets `allow_privileged` and
`allow_bind_mounts` to `true` — exactly the DinD opt-in — but **it is never
called outside its own unit tests**. `main.rs:61` calls `DaemonConfig::load()`,
and the only CLI flag is `--adapter`. There is no `--profile` flag and no
`MINIBOX_PROFILE` env var, so the profile is unreachable at runtime.

### The xtask cannot see the config file

`xtask/src/test_in_vm.rs:350-365` (`minibox_policy_allows`) reads **only** env
vars. With policy granted via config file alone, `detect_backend` silently
selects the unprivileged smolvm backend and skips `system_tests` — the DinD
test never runs and nothing reports why. The helper is unit-tested but has no
non-test caller.

---

## Regression: `test_e2e_dind_pull_and_run` vs. the policy gate

`DaemonFixture::start()` (`crates/miniboxd/tests/helpers/mod.rs:181-190`) spawns
`miniboxd` with `MINIBOX_DATA_DIR`, `MINIBOX_RUN_DIR`, `MINIBOX_SOCKET_PATH`,
`MINIBOX_CGROUP_ROOT`, and `RUST_LOG` — but **not** the two `ALLOW_*` vars.

Under deny-by-default the `--privileged` + `-v` request should be rejected at
`validate_policy`. Supporting evidence:

- The generated VM test script never exports the vars (`test_in_vm.rs:605-650`).
- `daemon_handler_coverage_tests.rs:668-669` explicitly `remove_var`s both,
  confirming the gate is live.
- `crux/dev/smoke.crux:67` — the real DinD smoke test — *does* export
  `MINIBOX_ALLOW_BIND_MOUNTS=true MINIBOX_ALLOW_PRIVILEGED=true`, and it also
  creates its own delegated cgroup slice (`smoke.crux:43-47`) before running.
  So `just smoke` is the one path that satisfies the gate today.

Note: an earlier revision of this document cited `.crux/promote.crux:85,180,269`
as the env-var source. That is stale — `promote.crux:6-9` now deliberately
excludes the container smoke tests, because they need root and cgroup
delegation that GitHub-hosted runners do not provide. The smoke test moved to
`crux/dev/smoke.crux`.

**Not confirmed by execution** — on macOS the test cannot run at all (adapter
capability), so the policy gate is unobservable there. It remains a live concern
only for Linux hosts where the native adapter is in play. Fix would be an
`ALLOW_*` env variant on the fixture, or having the fixture read the project
config.

Note this supersedes the earlier framing in this document: the policy gate is
**not** the reason the DinD test is unrunnable in general. Adapter capability is.

---

## Next Steps

### P0 — Automatic cgroup delegation — CLOSED

Both the doc's recommended option A and its "option B" now exist.

**A) `--cgroup-parent` flag** (explicit path):

- CLI: `crates/mbx/src/main.rs:184`, `crates/mbx/src/commands/run.rs:72`
- Protocol: `crates/minibox-core/src/protocol.rs:199,954`
- Validation: `validate_cgroup_parent` (`crates/minibox/src/container/cgroups.rs:71`)
  rejects relative paths, anything outside `/sys/fs/cgroup/`, and `..`
  components. Tests at `cgroups.rs:396-418`.
- Wiring: `crates/minibox/src/daemon/handler/run/preparation.rs:164-195` builds
  `CgroupManager::with_root(...)` then `create()`, which does `create_dir_all` +
  `enable_subtree_controllers(parent)` (`cgroups.rs:126-139`). Non-Linux builds
  reject the flag outright.

**Automatic delegation on `--privileged`** (the "more magic" option, now the
default):

- `preparation.rs:198-211` calls `delegate_subtree` whenever `privileged` is set
- `cgroups.rs:354-380` creates the subtree, enables controllers, and creates an
  `init` leaf cgroup so the container process satisfies the cgroups v2 "no
  internal processes" rule
- Failure is non-fatal (debug log), so a privileged run still succeeds where
  delegation is impossible

### P1 — `/dev` population — CLOSED

`setup_container_dev` (`crates/minibox/src/container/filesystem.rs:764-873`):

- tmpfs at `/dev` (`nosuid,noexec`, 64 MB)
- Host device nodes **bind-mounted** from `default_device_nodes()`
- `devpts` at `/dev/pts` with `newinstance,ptmxmode=0666,mode=0620`
- tmpfs at `/dev/shm` (`mode=1777`)
- Symlinks from `default_dev_symlinks()`

Deliberately **not** `mknod`: the child is cloned with `CLONE_NEWUSER`
unconditionally and the kernel refuses device-node creation inside a user
namespace (documented at `filesystem.rs:785-800`). Trade-off: node permission
bits are the host's, since the bind mount shares the inode.

Proc is mounted explicitly (`filesystem.rs:348-363`) with
`nosuid,nodev,noexec`. Neither path is gated behind `--privileged` or a new
flag — the doc's suggestion to gate behind `--init-dev` is moot.

### P2 — User-facing documentation — OPEN

No `docs/NESTING.md`. It must now also cover the three policy mechanisms, the
project-level `minibox.toml`, nesting depth, `--cgroup-parent`, and the fact
that privileged delegation is automatic. Existing design docs predate all of
this: `docs/plans/2026-05-26-nested-containers.md`,
`docs/plans/2026-05-26-nested-containers-impl.md`,
`docs/ideas/nested-workflows.md`, `docs/ideas/dind-sysbox.md`.

### P3 — Port forwarding — OPEN

No implementation. `minibox-domain/src/capability.rs:30` references
port-forward *conformance tests* as future work; the live capability matrix
reports `port_forwarding: unsupported` for every backend.

### P4 — CI coverage — OPEN, and mis-signalled

- `.github/workflows/ci.yml` runs `cargo xtask verify` on `ubuntu-latest` — no
  privileged path, no DinD
- `test_linux.rs:243-248` stages `system_tests` for the VM path, but
  `test-in-vm` cannot actually run it on macOS (see above)
- `justfile:134` → `crux run crux/dev/test_linux.crux` does exist and is
  wired; it is not the problem. The gap is that the path it drives cannot
  deliver on macOS (adapter capability + virtiofs), and CI cannot host the
  privileged path at all.

### P5 (new) — Reject unsupported capabilities instead of no-op'ing

Highest-value fix. The adapter should refuse a request it cannot honor rather
than silently dropping the capability:

- `--privileged` on smolvm currently reports success while doing nothing
- `-v` on smolvm exits 1 with no diagnostic
- The DinD test hangs in `Created` with no error

`validate_policy` is the wrong layer for this — it is a policy question, not a
capability question. The capability check belongs at admission against the
adapter's own matrix, and the CLI should surface a named error
("adapter `smolvm` does not support bind mounts").

### P6 (new) — Fix `is_privileged()` in the test harness

`VmBackend::is_privileged()` should consult the adapter capability matrix rather
than the policy env vars, so `test-in-vm` does not claim to run privileged
suites on a backend that cannot deliver them.

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
| ---- | ---------- | ------ | ---------- |
| Adapter silently ignores a requested capability | **High** | High | P5 — reject at admission with a named error |
| `test-in-vm` claims privileged on a non-capable backend | **High** | High | P6 — derive from capability matrix |
| DinD test unrunnable on macOS | Certain | Medium | Needs a Linux host with root; no macOS path exists |
| Test harness hides that fact | **High** | Medium | Suite list implies privileged; nothing warns |
| Policy gate blocks DinD setup | Medium | Medium | Three mechanisms now documented; fixture still omits `ALLOW_*` |
| Overlay-on-overlay kernel bug | Low | High | tmpfs bind; `supports_nested_overlay()` probe |
| Cgroup controller missing | Medium | Medium | Preflight probe; delegation failure non-fatal |
| Inner daemon orphaned on outer crash | Medium | Medium | PID namespace + kill in cleanup |
| Privilege escalation via nested privileged | Low | High | Cap exclusion list; no SYS_MODULE/SYS_BOOT |
| `/dev` absence causes silent failures | Low | Low | `setup_container_dev` binds real device nodes |

---

## Incidental Findings (out of scope, not fixed)

- `crates/miniboxd/src/main.rs:1501,1516` — `native_network_provider` is called
  under `#[cfg(target_os = "linux")]` but **is not defined anywhere**. The
  miniboxd binary's unit tests therefore fail to compile on Linux:
  `error[E0425]: cannot find function native_network_provider in this scope`.
  macOS skips them via cfg, so a macOS-only dev loop never sees it. This blocks
  `cargo test --no-run -p miniboxd --target aarch64-unknown-linux-musl`
  (the exact command `test-in-vm` runs), so cross-compiling the test suite for
  Linux currently fails outright.
- `crates/minibox/src/container/filesystem.rs:342,346` —
  `setup_runtime_directories(new_root)` is called twice back-to-back in
  `pivot_root_to`. Harmless but redundant.
- `xtask/src/test_in_vm.rs:350-365` — `minibox_policy_allows()` is unit tested
  but has no non-test caller; a half-wired guard. It also reads env vars only,
  so it cannot see a policy granted through `minibox.toml` or the other config
  layers, which makes `test-in-vm` silently pick the unprivileged backend.

Corrections to earlier revisions of this document, for the record:

- An earlier revision claimed `justfile:134` referenced a nonexistent
  `crux/dev/test_linux.crux`. **It exists.** The mistake was looking in `.crux/`
  when the pipelines live in `crux/dev/`.
- An earlier revision cited `.crux/promote.crux` as the source of the `ALLOW_*`
  env vars. **Stale** — `promote.crux:6-9` excludes the container smoke tests
  entirely; the DinD smoke test is `crux/dev/smoke.crux`.
