---
date: 2026-03-21
title: Firefox Profiler Integration for Benchmarks
status: approved
---

# Firefox Profiler Integration for Benchmarks

## Overview

Add optional profiling support to the minibox benchmark suite, enabling developers to capture performance profiles for codec, adapter, and lifecycle benchmarks. Profiles are captured using platform-native tools (perf on Linux), stored as pprof files, and can be converted to Firefox Profiler JSON format on-demand for visual analysis.

## Goals

1. **Identify bottlenecks** — Capture CPU/memory profiles during codec/adapter/lifecycle benches to see where time is spent
2. **Local developer workflows** — Enable developers to locally profile bench runs and inspect flamegraphs/traces in Firefox Profiler
3. **Foundation for regression detection** — Establish a profile storage and comparison mechanism; defer automated regression detection to follow-up work

## Design

### Invocation

```bash
cargo xtask bench --profile [--suite codec,adapter,lifecycle]
```

The `--profile` flag is optional and independent of suite selection. When enabled, profiles are captured for all specified suites. Without `--profile`, benching behavior is unchanged.

### Architecture

#### Profiler Abstraction

Create a trait-based `Profiler` interface in `minibox-bench` to abstract platform differences:

```rust
pub trait Profiler: Send {
    fn start(&mut self, suite: &str) -> Result<(), ProfileError>;
    fn stop(&mut self, suite: &str) -> Result<ProfilePath>;
}

pub struct LinuxPerfProfiler {
    results_dir: PathBuf,
    timestamp: String,
}

pub struct MacOSProfiler {
    // Placeholder; warns on use
}

pub fn create_profiler(results_dir: PathBuf, timestamp: &str) -> Box<dyn Profiler>
```

#### Profile Capture Flow

**Linux (perf):**

1. Before running each suite, spawn `perf record -p {bench_pid} -o {results_dir}/{timestamp}/{suite}.perf.data`
2. Run benchmark suite
3. Stop perf and convert `perf.data` → pprof using the `pprof` crate or CLI
4. Save result as `{suite}.pprof` in the timestamped results directory

**macOS:**

- Accept `--profile` flag but issue a warning: `"Profiling not supported on macOS; skipping"`
- Continue benchmarking without capturing profiles
- Keeps CLI consistent across platforms; unblocks Linux iteration

#### Result Storage

Profiles stored alongside existing bench results:

```
bench/results/{timestamp}/
├── codec.pprof
├── adapter.pprof
├── lifecycle.pprof
├── bench.json          (existing: timing results)
├── latest.json         (existing: canonical current)
└── metadata.json       (new: profile metadata)
```

`metadata.json` contains:
```json
{
  "timestamp": "2026-03-21T10:00:00Z",
  "git_sha": "abc1234",
  "suites_profiled": ["codec", "adapter"],
  "platform": "linux",
  "perf_available": true
}
```

#### Conversion to Firefox Profiler

Create `bench/convert-to-firefox.sh` — a utility script that converts pprof files to Firefox Profiler JSON:

```bash
#!/usr/bin/env bash
# Usage: ./bench/convert-to-firefox.sh <pprof-file>
# Output: {basename}.firefox.json in the same directory

set -e

pprof_file="$1"
output="${pprof_file%.pprof}.firefox.json"

# Use pprof CLI to convert to JSON format compatible with Firefox Profiler
pprof -json "$pprof_file" > "$output"

echo "Converted to $output"
echo "Import into Firefox Profiler: https://profiler.firefox.com/"
```

Manual workflow:
```bash
./bench/convert-to-firefox.sh bench/results/2026-03-21T10:00:00Z/codec.pprof
# → bench/results/2026-03-21T10:00:00Z/codec.firefox.json
# Import codec.firefox.json into https://profiler.firefox.com/
```

### Implementation Details

#### xtask Integration

Modify `xtask/src/main.rs` to:
- Add `--profile` flag to the `bench` subcommand
- Pass `--profile` to the `minibox-bench` binary
- Ensure `bench/results/{timestamp}/` directory exists before starting benches

#### minibox-bench Changes

1. Add `--profile` CLI argument (via clap or similar)
2. Create profiler instance based on platform and `--profile` flag
3. For each suite:
   - `profiler.start(suite_name)?`
   - Run benchmark tests
   - `profiler.stop(suite_name)?` → get `ProfilePath`
4. Save `metadata.json` with profiling info

#### Dependencies

- **Linux profiling:** Use `perf` binary (already available on typical Linux systems). If using `pprof` Rust crate: add dependency to xtask or minibox-bench
- **Conversion:** `pprof` CLI tool (users must have installed)

### Error Handling

| Scenario | Behavior |
|---|---|
| `perf` binary not found on Linux | Warn and skip profiling; continue benchmarking |
| `--profile` used on macOS | Warn "Profiling not supported on macOS"; continue benchmarking |
| perf.data capture fails (permission denied, etc.) | Log error, skip that suite's profile, continue |
| `bench/results/{timestamp}/` mkdir fails | Fail early with clear error |
| pprof conversion tool not installed | Skip conversion; profile stored as `.pprof` only |

### Backward Compatibility

- Benching without `--profile` is unchanged
- Existing `bench.json` and `latest.json` formats are unchanged
- Old result directories (without profiles) continue to work
- `metadata.json` is new and optional

### Success Criteria

- ✅ `cargo xtask bench --profile` captures profiles on Linux and stores them as pprof files
- ✅ Profiles stored in `bench/results/{timestamp}/` alongside bench results
- ✅ `convert-to-firefox.sh` converts pprof to Firefox Profiler JSON
- ✅ Firefox Profiler can ingest converted profiles
- ✅ macOS doesn't break; warns gracefully if `--profile` is used
- ✅ Existing bench workflow (without `--profile`) remains unchanged
- ✅ All three suites (codec, adapter, lifecycle) support profiling when `--profile` is enabled

### Testing

- **Unit tests:** Profiler trait implementation (mock + Linux)
- **Integration:** Run `cargo xtask bench --profile` on Linux; verify `.pprof` files exist
- **Manual:** Convert a profile with `bench/convert-to-firefox.sh`, load into Firefox Profiler, verify it's readable

### Future Work

- **Automated regression detection:** Compare profiles across commits to detect performance regressions
- **macOS profiling:** Implement lightweight sampler or Instruments integration for macOS
- **Profile storage in CI:** Optionally collect profiles on VPS and store in results
- **Web dashboard:** Build a dashboard to visualize profile trends over time
