---
status: archived
completed: "2026-03-16"
superseded_by: 2026-03-16-minibox-benchmark.md
note: Design doc; superseded by implementation plan
---

# Minibox Benchmark Design

Date: 2026-03-16
Status: Draft

## Goal

Create a first-class benchmark for minibox that covers runtime performance, image operations, resource limits overhead, and an end-to-end workflow. The benchmark should be deterministic, easy to run on a VPS, and produce both a human-readable summary and machine-readable JSON results.

## Approach (Selected)

Implement a small Rust CLI under `bench/` that shells out to `/usr/local/bin/minibox` and collects timings. Results are written to timestamped files in `bench/results/` with both JSON and table output. No baseline comparison targets yet; this focuses on stabilizing minibox’s own metrics first.

## Scope

Included suites:

- **Image pull**: `minibox pull alpine` (cold vs warm)
- **Image extract/cache**: derived from first pull and subsequent pulls
- **Container run**: `minibox run alpine -- /bin/true`
- **Exec latency**: `minibox run alpine -- /bin/echo ok` (short-lived command)
- **End-to-end**: pull + run + cleanup sequence

Excluded (for now): comparisons against Docker/containerd, destructive cleanup of state, and long-running workloads.

## CLI Interface

Proposed binary: `bench/minibox_bench.rs`

Flags:

- `--iters <N>`: iterations per sub-test (default: 20)
- `--cold`: include cold runs (default: true)
- `--warm`: include warm runs (default: true)
- `--suite <name>`: run only selected suites (repeatable)
- `--out-dir <path>`: output directory (default: `bench/results`)
- `--dry-run`: validate environment without running containers

Defaults are tuned for ~5 minutes total runtime on typical VPS hardware.

## Output

Write two files per run:

- `bench/results/<timestamp>.json`
- `bench/results/<timestamp>.txt`

JSON schema (simplified):

- `metadata`: timestamp, hostname, git SHA, minibox version
- `suites[]`: list of suite results
- `summary`: rollups across suites
- `errors[]`: fatal or per-test failures

Table output:

- One row per sub-test with min/avg/p95 and failures
- Suite totals
- Cold/warm labels

## Architecture

### Runner

- Parses flags
- Builds a `RunContext` (timestamp, hostname, git SHA, minibox version)
- Selects suites and orchestrates execution

### Suite

Each suite implements `run(ctx) -> SuiteResult` and performs:

- Iteration loop
- `Command` invocation of `/usr/local/bin/minibox`
- Capture stdout/stderr + exit code
- Record timing in microseconds
- Compute stats (min/avg/p95)

### Reporter

- Serializes `BenchReport` to JSON
- Generates a plain-text table summary

## Data Flow

1. Runner parses flags
2. Suites execute per-test iteration loops
3. Results collected into `BenchReport`
4. Reporter writes JSON and text outputs

## Error Handling

- Per-iteration failures recorded in the test result
- Non-fatal failures do not stop the run
- Fatal failure (daemon unreachable) short-circuits the benchmark with clear output

## Statistics

- Store timing in microseconds
- Compute min/avg/p95 on successful iterations
- If <5 successful iterations, set p95 = null and emit a warning

## Testing

- Unit tests for statistics aggregation (min/avg/p95)
- Unit test for JSON serialization
- `--dry-run` smoke check verifies `minibox` is available and daemon is reachable

## Results Management

Results are timestamped and appended under `bench/results/`. No automatic cleanup; users can archive or purge manually.

## Risks / Notes

- Running benchmarks while other containers are active may skew results
- Network variance affects cold pulls; warm runs mitigate this
- For reproducibility, future work can add a local test image/rootfs

## Next Steps

1. Implement `bench/` structure and CLI
2. Add statistics helper and JSON schema
3. Wire suites and report generation
4. Add tests for stats and serialization
5. Document usage in README and USAGE
