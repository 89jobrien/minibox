---
source_sha: 78e6b888e7c43b7d93ac244c4123295ea59d9f89
sources:
  - crates/mbx
  - crates/miniboxd
  - ops/install-systemd.sh
  - crates/minibox/src/adapters/docker_desktop.rs
  - xtask/src/main.rs
  - crates/minibox-bench
  - crates/macbox
  - crates/smolbox
  - crates/mcp
  - crates/minibox-tui
generated: 2026-09-04
---

# Usage

This document provides practical usage workflows for `minibox` across local Linux, systemd ops, macOS/Colima dogfooding, and experimental controller-driven flows.

**Quick Start (Linux)**

```bash
# Build
cargo build --release

# Optional host sanity check
cargo xtask doctor

# Start daemon (requires root)
sudo ./target/release/miniboxd

# Pull and run
sudo ./target/release/mbx pull alpine
sudo ./target/release/mbx run alpine -- /bin/echo "Hello from minibox!"
```

**Local Ops (systemd)**

```bash
# Build
cargo build --release

# Install binary + systemd unit + minibox.slice
sudo ./ops/install-systemd.sh

# Enable and start
sudo systemctl enable --now miniboxd

# Verify
sudo systemctl status miniboxd --no-pager
sudo /usr/local/bin/mbx ps
```

**Common CLI Workflows**

```bash
# List containers
sudo /usr/local/bin/mbx ps

# Pull image
sudo /usr/local/bin/mbx pull alpine

# Run container
sudo /usr/local/bin/mbx run alpine -- /bin/echo "Hello from minibox!"

# Name it for later exec/logs/stop calls
sudo /usr/local/bin/mbx run --name demo alpine -- /bin/sh

# Exec into an existing container
sudo /usr/local/bin/mbx exec demo -- /bin/sh

# Pause / resume
sudo /usr/local/bin/mbx pause demo
sudo /usr/local/bin/mbx resume demo

# Inspect logs and lifecycle events
sudo /usr/local/bin/mbx logs demo
sudo /usr/local/bin/mbx logs --follow demo
sudo /usr/local/bin/mbx events

# Load a local OCI tarball and run it
sudo /usr/local/bin/mbx load ./mbx-tester.tar --name mbx-tester
sudo /usr/local/bin/mbx run mbx-tester -- /run-tests.sh

# Clean up images
sudo /usr/local/bin/mbx prune
sudo /usr/local/bin/mbx rmi alpine:latest
```

**Environment-Specific Usage**

**GKE (Unprivileged Pods)**

```bash
# Select GKE adapter at daemon startup
MINIBOX_ADAPTER=gke sudo ./target/release/miniboxd

# Or specify proot binary location
MINIBOX_PROOT_PATH=/usr/local/bin/proot MINIBOX_ADAPTER=gke sudo ./target/release/miniboxd
```

**Windows (planned)**

The `winbox` server loop and WSL2 runtime are stubs and return errors. Windows and WSL2 are not
currently runnable deployment targets; use a supported Linux host or a macOS VM adapter.

**macOS (preferred and fallback adapters)**

`smolvm` is preferred when its binary is on `PATH`; with no explicit adapter, miniboxd falls
back to `krun` on macOS. Colima remains an alternative.

```bash
# Preferred automatic selection: smolvm, then krun fallback
./target/release/miniboxd

# Explicit fallback selection
MINIBOX_ADAPTER=krun ./target/release/miniboxd
```

**macOS (Colima alternative)**

```bash
# Requires Colima running on the host
colima start
MINIBOX_ADAPTER=colima sudo ./target/release/miniboxd
```

Colima is an alternative macOS dogfood path. The Docker Desktop adapter exists in
`crates/minibox/src/adapters/docker_desktop.rs` but is not yet wired into `miniboxd`.

**macOS Dogfood Flow**

```bash
# Build the Linux test image tarball
cargo xtask build-test-image

# Start the daemon against Colima
MINIBOX_ADAPTER=colima sudo ./target/release/miniboxd

# Run the end-to-end Linux suite inside minibox
cargo xtask test-linux
```

**macOS (VZ.framework)**

VZ support has been restored behind the `vz` feature and must be selected explicitly. It is
currently nonfunctional: `VZLinuxBootLoader` fails during VM boot on macOS 26.4.

```bash
cargo build --release -p miniboxd --features vz
MINIBOX_ADAPTER=vz ./target/release/miniboxd
```

**Experimental Controller (`mbxctl`) — REMOVED**

> NOTE: The `mbxctl` crate no longer exists in the workspace. It was extracted to the
> `minibox-plugins` workspace. The HTTP/SSE controller surface is not currently available
> from this workspace.

**Integration Notes**
The CLI communicates with the daemon over a Unix socket at `/run/minibox/miniboxd.sock` on Linux. If `mbx ps` fails with “No such file or directory,” the daemon is not running or the socket has not been created yet. Ensure the daemon is started and healthy (`systemctl status miniboxd` or `journalctl -u miniboxd -f`).

On macOS the default socket path is `/tmp/minibox/miniboxd.sock`.

## Forward Direction

Near-term work is concentrated in a few areas:

- extend the existing `minibox-mcp` stdio control surface while retaining its policy gates
- run AI-generated scripts and tests inside disposable minibox containers
- push commit/build/push parity further across `linux-native` and `colima`
- let CI agents create and tear down their own minibox-managed test environments

See `docs/core/ROADMAP.mbx.md` for the active roadmap.

## Terminal Dashboard

The read-only TUI is an optional `minibox-cli` feature:

```bash
cargo build -p minibox-cli --features tui
./target/debug/mbx tui
```

## VM Image CAS Overlay

The VM image build supports a content-addressed overlay at `~/.mbx/vm/overlay/`. Files placed here
are copied into the rootfs at build time. CAS (content-addressed storage) tracking lets you detect
drift between what was installed and what is running.

**Layout**

```
~/.mbx/vm/overlay/
  cas/<sha256>   ← file content, named by sha256 of content
  refs/<name>    ← text file containing a sha256, maps a name to a CAS object
```

**Adding a file to the CAS store**

```bash
# Add a file; optionally create a named ref
cargo xtask cas-add /path/to/myconfig --ref myconfig
# Output:
#   cas: <sha256>  /path/to/myconfig
#   ref: myconfig -> <sha256>
```

**Checking for drift**

```bash
cargo xtask cas-check
# Output per ref:
#   OK  myconfig
#   DRIFT  other  expected=<hash>  got=<actual>
```

Exits non-zero if any drift is found.

**In-VM drift check**

After `cargo xtask build-test-image`, `/etc/minibox-cas-refs` is written into the rootfs (one line
per ref, tab-separated: `<name>\t<sha256>`). Run `/sbin/check-drift.sh` inside the VM to verify
installed files match their expected hashes.

```bash
# Inside the Alpine VM shell
/sbin/check-drift.sh
```

## Benchmark

All criterion benchmarks live in `crates/minibox-bench`.

```bash
just bench                               # run benches, save to bench/results/
just bench-check                         # compare against the per-env baseline
just bench-baseline                      # save results as the new per-env baseline
cargo xtask bench                        # underlying command (--check / --save-baseline / --env)
```

> Results are written to `bench/results/` (gitignored). Tracked per-env baselines live at
> `bench/baseline.{local,selfhosted,hosted}.json`; the nightly CI bench job produces the
> canonical numbers.
