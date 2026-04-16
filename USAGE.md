# Usage

This document provides practical usage workflows for `minibox` across local Linux, systemd ops, macOS/Colima dogfooding, and experimental controller-driven flows.

**Quick Start (Linux)**

```bash
# Build
cargo build --release

# Optional host sanity check
just doctor

# Start daemon (requires root)
sudo ./target/release/miniboxd

# Pull and run
sudo ./target/release/minibox pull alpine
sudo ./target/release/minibox run alpine -- /bin/echo "Hello from minibox!"
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
sudo /usr/local/bin/minibox ps
```

**Common CLI Workflows**

```bash
# List containers
sudo /usr/local/bin/minibox ps

# Pull image
sudo /usr/local/bin/minibox pull alpine

# Run container
sudo /usr/local/bin/minibox run alpine -- /bin/echo "Hello from minibox!"

# Name it for later exec/logs/stop calls
sudo /usr/local/bin/minibox run --name demo alpine -- /bin/sh

# Exec into an existing container
sudo /usr/local/bin/minibox exec demo -- /bin/sh

# Pause / resume
sudo /usr/local/bin/minibox pause demo
sudo /usr/local/bin/minibox resume demo

# Inspect logs and lifecycle events
sudo /usr/local/bin/minibox logs demo
sudo /usr/local/bin/minibox logs --follow demo
sudo /usr/local/bin/minibox events

# Load a local OCI tarball and run it
sudo /usr/local/bin/minibox load ./mbx-tester.tar --name mbx-tester
sudo /usr/local/bin/minibox run mbx-tester -- /run-tests.sh

# Clean up images
sudo /usr/local/bin/minibox prune
sudo /usr/local/bin/minibox rmi alpine:latest
```

**Environment-Specific Usage**

**GKE (Unprivileged Pods)**

```bash
# Select GKE adapter at daemon startup
MINIBOX_ADAPTER=gke sudo ./target/release/miniboxd

# Or specify proot binary location
MINIBOX_PROOT_PATH=/usr/local/bin/proot MINIBOX_ADAPTER=gke sudo ./target/release/miniboxd
```

**Windows (WSL2)**

```bash
# Build and run inside WSL2
cargo build --release
sudo ./target/release/miniboxd
sudo ./target/release/minibox ps
```

**macOS (Colima)**

```bash
# Requires Colima running on the host
colima start
MINIBOX_ADAPTER=colima sudo ./target/release/miniboxd
```

Colima is the current macOS dogfood path. The Docker Desktop adapter exists in `mbx` but is not yet wired into `miniboxd`.

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

`MINIBOX_ADAPTER=vz` is wired, but the isolation test path is currently blocked by an upstream Apple `VZErrorInternal(code=1)` bug on macOS 26 ARM64. Treat Colima as the stable path for now.

**Experimental Controller (`mbxctl`)**

`mbxctl` is a small HTTP/SSE controller around `miniboxd` for job-style orchestration.

```bash
# Run from source
cargo run -p mbxctl -- --listen 127.0.0.1:9999

# Point it at a non-default daemon socket if needed
cargo run -p mbxctl -- --listen 127.0.0.1:9999 --socket /tmp/minibox/miniboxd.sock
```

Today `mbxctl` is job-oriented. The next planned step is an MCP-friendly control surface for agent-driven container orchestration.

**Integration Notes**
The CLI communicates with the daemon over a Unix socket at `/run/minibox/miniboxd.sock` on Linux. If `minibox ps` fails with “No such file or directory,” the daemon is not running or the socket has not been created yet. Ensure the daemon is started and healthy (`systemctl status miniboxd` or `journalctl -u miniboxd -f`).

On macOS the default socket path is `/tmp/minibox/miniboxd.sock`.

## Forward Direction

Near-term work is concentrated in a few areas:

- dogfood minibox for agent control via an MCP server surface
- run AI-generated scripts and tests inside disposable minibox containers
- push commit/build/push parity further across `linux-native` and `colima`
- let CI agents create and tear down their own minibox-managed test environments

See `docs/ROADMAP.md` for the active roadmap.

## Benchmark

```bash
cargo xtask bench                        # run locally, save to bench/results/
cargo xtask bench-vps                    # run on VPS, fetch results
./target/release/minibox-bench --suite codec    # protocol benchmarks
./target/release/minibox-bench --suite adapter  # adapter overhead benchmarks
```

Results are written to `bench/results/bench.jsonl` (append-only history) and `bench/results/latest.json`.
