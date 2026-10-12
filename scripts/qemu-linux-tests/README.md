# qemu-linux-tests

Run the minibox **lib** test binary on a real aarch64 Linux kernel.

## Why

Two independent gaps make the `container` module's unit tests unrunnable on a
macOS dev machine:

1. `crates/minibox/src/container/` is gated on `target_os = "linux"`, so those
   tests are not compiled into a darwin build at all.
2. They exercise ID-mapped bind mounts, which need privileges and filesystems a
   macOS host cannot provide.

And `cargo xtask test-in-vm` does **not** close the gap:
`build_test_script()` (`xtask/src/test_in_vm.rs`) collects a hardcoded list of
_integration_ test binaries — `integration_tests`, `cli_e2e_tests`, plus
`cgroup_tests` / `system_tests` / `sandbox_tests` when privileged. The
`minibox` **lib** test binary is not in that list, so no `minibox` unit test
ever runs in a VM through the stock path.

(Related: `xtask/src/test_in_vm.rs` also looks for a binary named `minibox`, but
the CLI package `minibox-cli` produces `mbx`, so its minibox backend can never
be selected. Only the smolvm path is live.)

## Usage

```sh
# whole lib suite
rust-script scripts/qemu-linux-tests/run.rs

# one module
rust-script scripts/qemu-linux-tests/run.rs --filter container::filesystem

# substring match
rust-script scripts/qemu-linux-tests/run.rs --filter bind_mount_target

# reuse the last cross-build, just re-pack and boot
rust-script scripts/qemu-linux-tests/run.rs --no-build

# rebuild the initramfs without booting
rust-script scripts/qemu-linux-tests/run.rs --pack-only

# bound the guest (default 300s)
rust-script scripts/qemu-linux-tests/run.rs --timeout 120
```

The filter is passed on the kernel command line (`mbx_filter=`), so changing it
does not rebuild or repack. The script exits non-zero if the tests fail.

## How it works

1. Cross-builds the lib test binary for `aarch64-unknown-linux-musl` using the
   local `aarch64-linux-musl-gcc`. The result is statically linked, so the
   guest needs no libc.
2. Extracts a **copy** of the Alpine initramfs from `~/.minibox/vm/boot` into
   `target/qemu-linux-tests/tree/`, stages the test binary in, and replaces its
   `init` with [`init`](init) from this directory.
3. Repacks the copy and boots it under `qemu-system-aarch64 -M virt`, streaming
   the serial console to stdout unbuffered.

The original boot assets are never touched. Scratch state lives under
`target/`, which is gitignored.

Requires: `qemu` (`brew install qemu`), `aarch64-linux-musl-gcc` (from
`messense/macos-cross-toolchains`), `cpio`, `gzip`, and the VM assets from
`cargo xtask build-vm-image`.

## Two traps this harness already hit

Both were silent failures that cost real time, recorded so they are not
rediscovered:

- **A non-executable `init` fails as `error -13` (EACCES), not a clear error.**
  The kernel prints `Failed to execute /init (error -13)`, falls through
  `/sbin/init`, `/etc/init`, `/bin/init`, and finally drops to an interactive
  `/bin/sh` — so the harness sits at a shell prompt waiting for input forever.
  `run.rs` therefore sets mode `0755` on the staged `init` explicitly instead
  of trusting the checkout's permission bit.
- **Never block forever waiting for the guest to exit.** `init` tries
  `sysrq` then `poweroff`; if both fail it just returns, and the kernel panics
  on the dead init, which `-no-reboot` turns into a clean QEMU exit. An
  infinite-sleep fallback in `init` prevents that panic and hangs QEMU
  indefinitely. `run.rs` additionally enforces `--timeout` and streams the
  console live, so a wedged guest is visible rather than silent.

## Expected failures

Running the whole suite in this harness gives **381 passed, 9 failed, 2
ignored**. All 9 failures are understood and fall into three groups.

### 1. Blocked by the kernel (3) — needs a real Linux host

minibox's own `virt` kernel asset does not permit user-namespace creation, and
`mount_setattr(MOUNT_ATTR_IDMAP)` is meaningless without one. These **cannot**
pass here, on any filesystem, at any privilege level:

```
container::filesystem::tests::bind_mount_tests::apply_bind_mounts_creates_target_dir
container::filesystem::tests::bind_mount_tests::apply_bind_mounts_mounts_directory
container::filesystem::tests::bind_mount_tests::apply_bind_mounts_read_only
```

Ruled out as causes, by measurement inside the guest:

| Hypothesis             | Measured                                            |
| ---------------------- | --------------------------------------------------- |
| Insufficient privilege | `CapEff: 000001ffffffffff` (full root)              |
| Kernel lockdown        | `lockdown: [none]`                                  |
| Seccomp                | `Seccomp: 0`, `Seccomp_filters: 0`                  |
| Filesystem type        | identical `EPERM` on tmpfs, overlay, initramfs root |
| `mount`/`bind` broken  | both succeed — only the idmap step fails            |
| **No user namespaces** | **`unshare -U` fails**                              |

Reproduced identically under both smolvm (Firecracker) and QEMU, which is what
ruled out "the VM" as a variable. They need a real distro kernel or a real
Linux host.

### 2. Needs network (5)

```
adapters::ghcr::tests::http::authenticate_succeeds_with_versioned_tag_no_latest
adapters::ghcr::tests::http::has_image_returns_true_after_pull
adapters::ghcr::tests::http::pull_image_cleans_tmp_on_digest_mismatch
adapters::ghcr::tests::http::pull_image_rejects_digest_mismatch
adapters::ghcr::tests::http::pull_image_stores_layer_on_disk
```

The harness deliberately configures no networking, which removes the network as
a variable. These pass with connectivity.

### 3. A real bug this harness surfaced (1)

```
adapters::colima::bind_mount_tests::validate_lima_paths_rejects_opt
```

`validate_lima_paths` allowlists host paths against `$HOME`, and in Rust
`Path::new("").starts_with(anything)` is `true` — as is `Path::new("/")`. With
`HOME` empty or `/`, the allowlist silently accepts **every** host path. The
initramfs sets `HOME=/`, which is why this fails here; it passes on a dev
machine only because `$HOME` happens to be a real directory. Confirmed by
running the test with `HOME` unset, `HOME=/root`, and `HOME=`: only the last
fails.
