# Self-hosted `minibox` GHA runner

Replaces the previous self-hosted runner, which lived on a VPS that has since
been shut down. This version runs as an ephemeral, privileged container under
minibox itself, supervised by a loop that restarts it after every job.

## Why privileged

Only four jobs in `.github/workflows/merge.yml` use the `[self-hosted, minibox]`
label: `test-integration`, `test-e2e`, `test-protocol-e2e-linux`, and
`cleanup-linux`. All four need root + cgroup/namespace access to exercise
minibox's own container runtime. `test-unit` and `lint` already run on
GitHub-hosted `ubuntu-latest` and need nothing here.

This means the runner tests minibox's privileged-container path by running
_inside_ a privileged minibox container — accept that circularity knowingly. A
bug in minibox's privilege-drop or cgroup delegation could take the runner down
with it. Keep an eye on this; moving the Linux-root suite to bare
hardware/VM instead remains an option if this turns out to be too fragile.

## Build and push the image

No Docker/Colima/podman required — `mbx` itself builds and pushes OCI images
(`mbx build`, `mbx push`, `mbx commit`), confirmed present as of this repo's
`develop` branch (2026-10-14):

```bash
mbx build -t ghcr.io/89jobrien/minibox-ci-runner:latest ops/ci-runner/
mbx push ghcr.io/89jobrien/minibox-ci-runner:latest --token "$(gh auth token)"
```

**`mbx build`'s RUN-step chaining bug on the smolvm adapter is fixed and
verified (2026-10-14).** It used to boot a _fresh_ VM from a hardcoded
default image (`ubuntu:24.04`) for every `RUN` step after the first, instead
of continuing from the previous step's modified rootfs — root cause was
`execute_run_step` in `crates/minibox/src/adapters/builder.rs` passing
`image_ref: None` into `ContainerSpawnConfig`, and `SmolVmRuntime` having no
way to reuse one VM across steps in the same stage at all. The fix gives
`SmolVmRuntime` a persistent-named-machine path (`smolvm machine
create`/`start`/`exec`, mirroring the krun `KrunExecRuntime` pattern) keyed
by the hostname `builder.rs` already sets identically for every RUN step in
one stage, booted from the stage's actual `FROM` image via
`StageState.source_image_ref`. Confirmed by two live smoke tests against this
exact Dockerfile: step 2 (`apt-get install`) succeeds, step 3 (`curl | sh`
rustup install) now runs against the correct `debian:bookworm-slim` base
_with_ step 2's packages present, step 5 (`cargo install cargo-nextest`)
compiles successfully, and steps 6-8 (ARG/WORKDIR/the GH-runner tarball
download) all succeed in the same persistent VM. A matching cleanup method
(`cleanup_persistent_machine`, called unconditionally after the build loop —
success or failure) tears the VM down afterward; this needed a second fix
(`smolvm machine delete` is interactive by default and silently no-ops with
exit 0 when run non-interactively without `--force` — confirmed via manual
repro, then fixed and reverified: the persistent machine is gone from `smolvm
machine ls` immediately after the build request completes).

**COPY/VM-bridging gap is fixed and verified (2026-10-15).** Step 9 (`COPY
entrypoint.sh`) used to succeed against minibox's host-side layer-snapshot
mechanism while step 10 (`RUN chmod +x entrypoint.sh`) then failed —
`chmod: cannot access '/actions-runner/entrypoint.sh': No such file or
directory` — because the persistent build VM's own disk never received what
`COPY`/`ADD` wrote to that host-side snapshot. Fixed by adding a new
`ContainerRuntime::sync_persistent_machine_path` port method (default no-op;
`crates/minibox-domain/src/runtime.rs`), implemented on `SmolVmRuntime`
(`crates/minibox/src/adapters/smolvm.rs`): ensures the stage's persistent
build machine exists, `mkdir -p`s the destination's parent inside the VM via
`smolvm machine exec` (COPY/ADD destinations have no automatic mkdir the way
WORKDIR+RUN does), then pushes the file in via `smolvm machine cp`.
`builder.rs`'s `Instruction::Copy`/`Instruction::Add` arms call this right
after `execute_filesystem_step` succeeds, resolving the same destination
path `copy_sources` itself uses (`copy::resolve_container_path`, now
`pub(super)`) so the VM-side destination always matches where the file
actually landed on the host. Confirmed via a full rebuild+redeploy+rerun of
the real `ops/ci-runner/Dockerfile`: `mbx build` now reports `Successfully
built`/`Successfully tagged` through all 11 steps, and `smolvm machine ls`
shows no lingering `minibox-build-*` entry immediately after the build
completes — `cleanup_persistent_machine` tears the VM down the same as for
the RUN-chaining fix.

`mbx build` runs its `RUN` steps inside a real container via whatever adapter
the daemon has selected (krun/smolvm on macOS, native on Linux) — it is not a
host-side shell script, so it needs the same adapter capability as `mbx run`.
`mbx push --token`/`--username`+`--password` support credentialed pushes to
ghcr.io, so a private source repo during development is fine.

**The ghcr.io repo must be public once consumed via `mbx pull`/`mbx run`.**
Confirmed via `mbx pull --help` (2026-10-14): pull has no `--token`/
`--username`/`--password` option at all (`crates/minibox-core/src/image/registry.rs`,
`adapters/registry.rs`) — anonymous-only. Push can be authenticated; pull
cannot. There is no credentialed-pull path short of adding registry-auth
support to minibox itself.

## Running it

Two deployment targets, same image and supervisor script:

### Production (Linux host, e.g. a VPS)

```bash
install -m 0755 ops/ci-runner/supervisor.sh /usr/local/bin/minibox-ci-runner-supervisor.sh
install -m 0644 ops/ci-runner/minibox-ci-runner.service /etc/systemd/system/minibox-ci-runner.service
systemctl daemon-reload
systemctl enable --now minibox-ci-runner
```

**Status: no Linux host currently available.** The previous self-hosted
runner's VPS was shut down (see `.ctx/memory-bank/progress.mbx.md`, "Self-hosted
GHA runner is gone") and `jobrien-vm` is offline as of this writing. This path
is documented but cannot be exercised until one of those comes back.

### Local (macOS dev machine, via krun)

Validated this session: `mbx run --privileged` and `mbx exec` both work
correctly against the krun adapter on macOS, once the **daemon** (not the
client) has:

```bash
launchctl bootout gui/501/com.joebrien.miniboxd   # stop the default-policy daemon
env MINIBOX_ALLOW_PRIVILEGED=1 MINIBOX_ADAPTER=krun /Users/joe/.cargo/bin/miniboxd &
```

Then run `supervisor.sh` directly in a terminal (not systemd — there's no
systemd on macOS):

```bash
RUNNER_REPO=89jobrien/minibox ops/ci-runner/supervisor.sh
```

This is a reasonable dev-loop substitute while no Linux host exists, but is
**not** a production deployment: it requires leaving a foreground daemon and
loop running, has no restart-on-crash supervision, and competes with the
launchd-managed daemon for the same Unix socket
(`/tmp/minibox/miniboxd.sock`) — boot the launchd daemon back out before
starting the manual one, and restore it (`launchctl bootstrap gui/501
<plist>`) when done. See `.claude.local.md` for the exact commands.

## Not yet done / open questions

- `mbx build` now completes all 11 steps of this Dockerfile successfully
  (both the RUN-chaining bug and the COPY/VM-bridging gap are fixed and
  verified as of 2026-10-15). Not yet checked whether the krun/native
  adapters ever had either bug — only smolvm was exercised.
- Not yet exercised end-to-end against a live GitHub repo — `mbx run
--privileged` and `mbx exec` are confirmed working (2026-10-14 smoke tests),
  and `mbx build` now completes the full image build, but no container has
  yet actually registered as a runner, picked up a real job, and reported
  results back to `.github/workflows/merge.yml`.
- `gh` in this environment needs an interactive 1Password unlock that Claude
  cannot perform — the human needs to run `supervisor.sh`'s `gh api
.../registration-token` step themselves, or pre-authenticate `gh` in a
  session Claude can shell into.
- No monitoring/alerting if the supervisor loop itself crashes, beyond
  systemd's `Restart=on-failure` (Linux-only; no equivalent configured for the
  local macOS path).
- Decide whether to actually run this long-term, or keep the Linux-root suite
  on a plain VM/bare host instead (see "Why privileged" above) once one is
  available again.
