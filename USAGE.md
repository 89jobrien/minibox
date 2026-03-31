# Usage

This document provides practical usage workflows for `minibox` across local Linux, ops/systemd, and other environments.

**Quick Start (Linux)**

```bash
# Build
cargo build --release

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
MINIBOX_ADAPTER=colima sudo ./target/release/miniboxd
```

The Docker Desktop adapter exists in `mbx` but is not yet wired into
`miniboxd`. Use `MINIBOX_ADAPTER=colima` for macOS.

**Integration Notes**
The CLI communicates with the daemon over a Unix socket at `/run/minibox/miniboxd.sock` on Linux. If `minibox ps` fails with “No such file or directory,” the daemon is not running or the socket has not been created yet. Ensure the daemon is started and healthy (`systemctl status miniboxd` or `journalctl -u miniboxd -f`).

## Benchmark

```bash
cargo xtask bench                        # run locally, save to bench/results/
cargo xtask bench-vps                    # run on VPS, fetch results
./target/release/minibox-bench --suite codec    # protocol benchmarks
./target/release/minibox-bench --suite adapter  # adapter overhead benchmarks
```

Results are written to `bench/results/bench.jsonl` (append-only history) and `bench/results/latest.json`.
