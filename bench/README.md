# Bench

Benchmarking for minibox. Results are stored in `bench/results/` and committed after VPS runs.

## Suites

| Suite       | Tests | Requires daemon  | Platform |
| ----------- | ----- | ---------------- | -------- |
| `codec`     | 36    | No               | Any      |
| `adapter`   | 10    | No               | Any      |
| `cli`       | Multiple command cases (`pull`, `run`, `ps`, `stop`, `rm`, `exec`, `e2e`) | Yes (Linux+root) | Linux    |

## Run via xtask (saves results)

```bash
# Run all suites (lifecycle skipped unless daemon available)
cargo xtask bench

# Run specific suites
cargo xtask bench --suite codec
cargo xtask bench --suite adapter
cargo xtask bench --suite codec,adapter

# Dry run (no timing, just schema/structure check)
./target/debug/minibox-bench --dry-run
```

Results are appended to `bench/results/bench.jsonl` and written to `bench/results/latest.json`.

## Criterion HTML reports (local only)

```bash
cargo bench -p linuxbox
# Opens target/criterion/report/index.html
```

These use the same logic as the `codec` and `adapter` xtask suites but produce HTML flamegraph reports. Results are not saved to bench.jsonl.

## VPS runs

```bash
cargo xtask bench-vps
```

Runs on the remote VPS via SSH, saves results, and commits + pushes `bench/results/`.

## Results format

`bench.jsonl` — one JSON object per run (append-only history):

```json
{
  "git_sha": "abc1234",
  "timestamp": "2026-03-21T10:00:00Z",
  "host": "vps",
  "suites": [
    {
      "name": "codec",
      "tests": [
        {
          "name": "encode_run_container",
          "median_us": 0.42,
          "p99_us": 0.61,
          "unit": "nanos"
        }
      ]
    }
  ]
}
```

`latest.json` — same format, always the most recent run (used by devloop).
