---
name: performance
description: Performance analysis for minibox — container init latency, codec/adapter microbenchmarks, memory profiling
---

# Performance Analysis

Systematic performance analysis for minibox, focusing on **container init latency**, **protocol codec throughput**, **adapter trait overhead**, and **daemon memory usage**.

## When to Use

- After changes to container init, overlay mount, or cgroup setup
- After protocol serialization changes
- When investigating daemon memory growth under load
- Before cutting a release, to confirm benchmarks are stable

## Minibox Performance Targets

| Metric                 | Target           | Verification                                          | Failure Threshold              |
| ---------------------- | ---------------- | ----------------------------------------------------- | ------------------------------ |
| Protocol encode/decode | nanosecond scale | `cargo bench -p minibox-bench --bench protocol_codec` | regression over baseline       |
| Adapter trait overhead | nanosecond scale | `cargo bench -p minibox-bench --bench trait_dispatch` | regression over baseline       |
| Container init latency | <500 ms          | `hyperfine 'mbx run alpine -- /bin/true'`             | >1 s is a blocker              |
| Daemon idle memory     | <50 MB RSS       | `/usr/bin/time -v miniboxd`                           | >100 MB warrants investigation |
| Image pull latency     | network-bound    | manual timing                                         | measure, don't gate            |

## Benchmark Workflow

### 1. Establish Baseline

Before making changes, capture current results:

```bash
# Run microbenchmarks and save to bench/results/
cargo xtask bench

# Capture current snapshot
cp bench/results/latest.json /tmp/baseline.json

# Also run criterion benches for HTML reports
cargo bench -p minibox-bench
```

### 2. Make Changes

Implement the optimization or feature.

### 3. Rebuild and Measure

```bash
cargo build --release

# Microbenchmarks
cargo xtask bench

# Compare against baseline
diff /tmp/baseline.json bench/results/latest.json
```

### 4. Container Init Latency

```bash
# Requires a running daemon and a pulled image
sudo ./target/release/miniboxd &
sudo ./target/release/mbx pull alpine

# Benchmark init time
hyperfine 'sudo ./target/release/mbx run alpine -- /bin/true' \
  --warmup 3 --runs 10
```

### 5. Daemon Memory Profile

```bash
# Idle RSS
/usr/bin/time -v sudo ./target/release/miniboxd &
sleep 1
grep VmRSS /proc/$(pgrep miniboxd)/status

# Under load — run 20 containers in parallel
for i in $(seq 1 20); do
  sudo ./target/release/mbx run alpine -- /bin/true &
done
wait
grep VmRSS /proc/$(pgrep miniboxd)/status
```

## Common Performance Issues

### Issue: Container Init Regression

**Symptom**: `hyperfine` shows init time increased by >100 ms

**Detection**:

```bash
# Profile with flamegraph
sudo cargo flamegraph -- ./target/release/mbx run alpine -- /bin/true
open flamegraph.svg

# Look for:
# - overlay mount taking longer than expected
# - cgroup file writes in a loop
# - image layer extraction on every run (should be cached)
```

**Common causes**:

| Cause                            | Symptom in flamegraph        | Fix                                           |
| -------------------------------- | ---------------------------- | --------------------------------------------- |
| Image cache miss on every run    | `tar::unpack` wide bar       | Fix cache key in `image/reference.rs`         |
| Cgroup hierarchy not pre-created | `create_dir_all` in hot path | Create `minibox.slice` at daemon start        |
| Overlay workdir not on same FS   | VFS copy instead of reflink  | Ensure upper/work on same filesystem as lower |
| Blocking I/O in async handler    | `spawn_blocking` absent      | Add `tokio::task::spawn_blocking`             |

### Issue: Codec Regression

**Symptom**: the `protocol_codec` Criterion target regresses against the selected baseline

**Detection**:

```bash
cargo bench -p minibox-bench --bench protocol_codec
```

**Common causes**:

- New serde `Deserialize` derive on a hot type with unexpected allocations
- `#[serde(rename_all)]` causing string copies where zero-copy was possible
- New protocol variant added to a large enum — match dispatch overhead

**Fix pattern**:

```rust
// For hot protocol types, prefer borrowed deserialization where possible
#[derive(Serialize, Deserialize)]
pub struct ContainerOutput<'a> {
    pub id: &'a str,       // borrow from input buffer
    pub data: &'a [u8],    // borrow from input buffer
}
```

### Issue: Adapter Trait Overhead

**Symptom**: the `trait_dispatch` Criterion target shows overhead growth

Adapter dispatch goes through `dyn Trait` trait objects. If a hot path is hitting `dyn ContainerRuntime` thousands of times per second, evaluate whether the dispatch is avoidable.

```bash
cargo bench -p minibox-bench --bench trait_dispatch
```

This bench measures the overhead of `dyn ResourceLimiter`, `dyn FilesystemProvider`, `dyn ContainerRuntime`, and `dyn ImageRegistry` calls against no-op mock implementations. Regression here indicates vtable dispatch cost increased — usually from adding more indirection or heap allocation to the trait methods.

### Issue: Memory Growth Under Load

**Symptom**: Daemon RSS grows as containers are created and destroyed

```bash
# Watch RSS while running containers
while true; do
  grep VmRSS /proc/$(pgrep miniboxd)/status
  sleep 1
done &

# Run 100 containers serially
for i in $(seq 1 100); do
  sudo ./target/release/mbx run alpine -- /bin/true
done
```

**Common causes**:

- Container state not removed from `DaemonState` HashMap after stop
- Image manifest cached without eviction
- `Vec<u8>` output buffers held in `ContainerRecord` after container exits

## Benchmark Result Pipeline

The bench pipeline collects Criterion output and maintains per-environment baselines:

```text
cargo xtask bench
  → runs cargo bench -p minibox-bench
  → writes JSON/CSV snapshots and an HTML dashboard under bench/results/

bench/baseline.<env>.json   ← tracked comparison baseline
bench/results/              ← generated local/CI artifacts
```

To commit bench results:

```bash
just bench-baseline
```

Do not hand-edit generated results or baseline snapshots.

## Profiling Tools Reference

| Tool                  | Purpose                         | Command                                      |
| --------------------- | ------------------------------- | -------------------------------------------- |
| **minibox-bench**     | Criterion benchmark crate       | `cargo bench -p minibox-bench`               |
| **cargo xtask bench** | Result collection and baselines | `cargo xtask bench --check`                  |
| **hyperfine**         | Container init wall-clock       | `hyperfine 'mbx run alpine -- /bin/true'`    |
| **flamegraph**        | CPU hotspot profiling           | `sudo cargo flamegraph -- mbx run ...`       |
| **/proc/PID/status**  | Daemon RSS                      | `grep VmRSS /proc/$(pgrep miniboxd)/status`  |
| **/usr/bin/time -v**  | Peak RSS                        | `/usr/bin/time -v sudo miniboxd`             |
| **strace -c**         | Syscall frequency               | `sudo strace -c mbx run alpine -- /bin/true` |
| **perf stat**         | CPU instruction counts          | `sudo perf stat mbx run alpine -- /bin/true` |

Install:

```bash
# flamegraph
cargo install flamegraph

# hyperfine
brew install hyperfine   # macOS
cargo install hyperfine  # Linux
```

## Performance Testing Checklist

Before committing changes that touch container init, protocol, or adapter code:

- [ ] `cargo xtask bench --check` shows no regression against the selected baseline
- [ ] `hyperfine` container init time within target if init path changed
- [ ] `grep VmRSS` shows stable daemon memory after 50 container runs
- [ ] Flamegraph reviewed if init time increased by more than 50 ms
- [ ] Updated `bench/baseline.<env>.json` reviewed when intentionally accepting a new baseline
