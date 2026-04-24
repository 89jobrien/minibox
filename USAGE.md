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

After `cargo xtask build-vm-image`, `/etc/minibox-cas-refs` is written into the rootfs (one line
per ref, tab-separated: `<name>\t<sha256>`). Run `/sbin/check-drift.sh` inside the VM to verify
installed files match their expected hashes.

```bash
# Inside the Alpine VM shell
/sbin/check-drift.sh
```

## Benchmark

```bash
cargo xtask bench                        # run locally, save to bench/results/
cargo xtask bench-vps                    # run on VPS, fetch results
./target/release/minibox-bench --suite codec    # protocol benchmarks
./target/release/minibox-bench --suite adapter  # adapter overhead benchmarks
```

> Results are written to `bench/results/bench.jsonl` (append-only history) and `bench/results/latest.json`.

```mmd
graph TD
  subgraph BUGS ["Bugs"]
      B60["#60 bug·p1\nfork() in Tokio runtime"]
      B61["#61 bug·vz·blocked\nVZErrorInternal macOS 26"]
  end

  subgraph COLIMA ["Colima path"]
      C90["#90 feat·colima·p1\nWire macbox Colima adapters"]
      C89["#89 feat·colima·e2e·p2\nDogfood create→commit→push"]
      C80["#80 testing·p2\nRegression tests rootfs metadata"]
      C90 --> C89
      C90 --> C80
  end

  subgraph VZ ["VZ / Virtualization.framework (all blocked on #61)"]
      V84["#84 feat·vz·blocked\nProvision Linux VM via VF"]
      V88["#88 feat·vz·blocked\nminibox-agent in-VM daemon"]
      V93["#93 feat·vz·blocked\nvsock I/O bridge"]
      V75["#75 feat·vz·blocked\nvirtiofs host-path mounts"]
      V85["#85 feat·vz·p2\nEncode VZ commit/build/push behavior"]
      B61 --> V84
      V84 --> V88
      V88 --> V93
      V84 --> V75
  end

  subgraph CONFORMANCE ["Conformance suite"]
      CF82["#82 ✓ closed\nConformance boundary spec"]
      CF92["#92 ✓ closed\nFixture helpers"]
      CF67["#67 ✓ closed\nCommit conformance tests"]
      CF71["#71 testing·conformance\nBuild conformance tests"]
      CF62["#62 testing·conformance\nPush conformance tests"]
      CF79["#79 testing·conformance\nValidate on Colima + Linux CI"]
      CF77["#77 feat·conformance\nMarkdown/JSON reports"]
      CF82 --> CF67
      CF82 --> CF71
      CF82 --> CF62
      CF92 --> CF67
      CF92 --> CF71
      CF92 --> CF62
      CF67 --> CF79
      CF71 --> CF79
      CF62 --> CF79
      C90 --> CF79
  end

  subgraph NET ["Networking"]
      N94["#94 feat·networking·p2\nveth/bridge"]
  end

  subgraph PTY ["Interactive I/O"]
      P83["#83 feat·p2\nPTY/stdio piping"]
  end

  subgraph DAGU ["Dagu"]
      D86["#86 fix·dagu·p2\nTier 2 mbx-dagu fixes"]
  end
```
