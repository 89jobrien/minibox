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

**macOS (Colima / Docker Desktop)**

The Colima and Docker Desktop adapters exist in `minibox-lib` but are not yet
wired into `miniboxd`. The daemon currently only accepts `MINIBOX_ADAPTER=native`
(default) or `MINIBOX_ADAPTER=gke`. macOS development requires a Linux VM
(Colima, Lima, Docker Desktop) and running the daemon inside it.

**Integration Notes**
The CLI communicates with the daemon over a Unix socket at `/run/minibox/miniboxd.sock` on Linux. If `minibox ps` fails with “No such file or directory,” the daemon is not running or the socket has not been created yet. Ensure the daemon is started and healthy (`systemctl status miniboxd` or `journalctl -u miniboxd -f`).

## Benchmark

Build and run the benchmark CLI:

```
cargo build -p minibox-bench
./target/debug/minibox-bench --dry-run
./target/debug/minibox-bench
```

Results are written to `bench/results/<timestamp>.json` and `bench/results/<timestamp>.txt`.
