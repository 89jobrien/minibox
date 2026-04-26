---
status: done
completed: "2026-03-16"
branch: main
note: minibox-bench crate fully shipped
---

> **ARCHIVED** — This document is not authoritative. See the current docs in the repo root.

# Minibox Benchmark Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement a Rust-based benchmark CLI that measures minibox performance and writes JSON + table results.

**Architecture:** A standalone Rust binary under `crates/minibox-bench` shells out to `minibox` (configurable via `MINIBOX_BIN`), times each iteration, aggregates stats, and writes structured reports to `bench/results/`.

**Tech Stack:** Rust (std + serde/serde_json), std::process::Command, std::time::Instant.

---

### Task 1: Create Bench Skeleton (Deviated)

**Files:**

- Create: `bench/README.md`
- Create: `crates/minibox-bench/src/main.rs` (crate-based implementation)

**Step 1: Write the failing test**
No tests yet; skip for skeleton creation.

**Step 2: Create minimal CLI skeleton**
Implement a minimal `main()` that parses `--help` and exits 0.

```rust
fn main() {
    println!("minibox-bench: not yet implemented");
}
```

**Step 3: Run to verify binary compiles**
Run: `cargo build -p minibox-bench`
Expected: Exit 0

**Step 4: Commit**

```bash
git add bench/README.md crates/minibox-bench
git commit -m "bench add skeleton"
```

---

### Task 2: Add Data Model + JSON Schema (Deviated)

**Files:**

- Modify: `crates/minibox-bench/src/main.rs`
- Create: `crates/minibox-bench/schema.json`

**Step 1: Write the failing test**
Create a minimal unit test within `crates/minibox-bench/src/main.rs` for JSON serialization.

```rust
#[test]
fn report_serializes() {
    let report = BenchReport::empty();
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"metadata\""));
}
```

**Step 2: Run test to verify it fails**
Run: `cargo test -p minibox-bench report_serializes` (temporary; expected FAIL)
Expected: FAIL (BenchReport not defined)

**Step 3: Write minimal implementation**
Define `BenchReport`, `Metadata`, `SuiteResult`, `TestResult` structs and derive `Serialize`.

**Step 4: Run test to verify it passes**
Run: `cargo test -p minibox-bench report_serializes`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/minibox-bench/src/main.rs crates/minibox-bench/schema.json
git commit -m "bench add report schema"
```

---

### Task 3: Implement Stats Helper

**Files:**

- Modify: `crates/minibox-bench/src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn stats_min_avg_p95() {
    let data = vec![10u64, 20, 30, 40, 50];
    let stats = Stats::from_samples(&data);
    assert_eq!(stats.min, 10);
    assert_eq!(stats.avg, 30);
    assert_eq!(stats.p95, 50);
}
```

**Step 2: Run test to verify it fails**
Run: `cargo test -p minibox-bench stats_min_avg_p95`
Expected: FAIL (Stats undefined)

**Step 3: Write minimal implementation**
Implement `Stats::from_samples` and p95 calculation.

**Step 4: Run test to verify it passes**
Run: `cargo test stats_min_avg_p95`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "bench add stats helper"
```

---

### Task 4: Implement CLI Flags + Runner

**Files:**

- Modify: `crates/minibox-bench/src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn default_iters_is_20() {
    let args = vec!["bench".to_string()];
    let cfg = BenchConfig::from_args(args).unwrap();
    assert_eq!(cfg.iters, 20);
}
```

**Step 2: Run test to verify it fails**
Run: `cargo test -p minibox-bench default_iters_is_20`
Expected: FAIL (BenchConfig undefined)

**Step 3: Write minimal implementation**
Implement `BenchConfig` parsing using `std::env::args` (no clap for now).

**Step 4: Run test to verify it passes**
Run: `cargo test default_iters_is_20`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "bench add cli config"
```

---

### Task 5: Implement Command Runner

**Files:**

- Modify: `crates/minibox-bench/src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn command_runner_captures_exit_status() {
    let result = run_cmd("/bin/true", &[]).unwrap();
    assert!(result.success);
}
```

**Step 2: Run test to verify it fails**
Run: `cargo test -p minibox-bench command_runner_captures_exit_status`
Expected: FAIL (run_cmd undefined)

**Step 3: Write minimal implementation**
Implement `run_cmd(path, args)` using `std::process::Command` and return timing + stdout/stderr.

**Step 4: Run test to verify it passes**
Run: `cargo test command_runner_captures_exit_status`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "bench add command runner"
```

---

### Task 6: Implement Suites (Pull, Run, Exec, E2E)

**Files:**

- Modify: `crates/minibox-bench/src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn suite_has_results() {
    let cfg = BenchConfig::default();
    let report = run_suites(&cfg, true).unwrap();
    assert!(!report.suites.is_empty());
}
```

**Step 2: Run test to verify it fails**
Run: `cargo test -p minibox-bench suite_has_results`
Expected: FAIL

**Step 3: Write minimal implementation**
Add suites that call:

- `minibox pull alpine`
- `minibox run alpine -- /bin/true`
- `minibox run alpine -- /bin/echo ok`
- `minibox pull alpine` + `minibox run alpine -- /bin/true`

**Step 4: Run test to verify it passes**
Run: `cargo test suite_has_results`
Expected: PASS (mocked or dry-run path)

**Step 5: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "bench add suites"
```

---

### Task 7: Implement Reporting (JSON + Table)

**Files:**

- Modify: `crates/minibox-bench/src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn report_writes_json() {
    let report = BenchReport::empty();
    let path = "/tmp/bench-report.json";
    write_json(&report, path).unwrap();
    assert!(std::path::Path::new(path).exists());
}
```

**Step 2: Run test to verify it fails**
Run: `cargo test -p minibox-bench report_writes_json`
Expected: FAIL

**Step 3: Write minimal implementation**
Implement JSON + table file writers, timestamped naming, output dir creation.

**Step 4: Run test to verify it passes**
Run: `cargo test report_writes_json`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "bench add report writers"
```

---

### Task 8: Wire CLI Entry + Dry Run

**Files:**

- Modify: `crates/minibox-bench/src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn dry_run_skips_execution() {
    let cfg = BenchConfig { dry_run: true, ..BenchConfig::default() };
    let report = run_benchmark(&cfg).unwrap();
    assert!(report.suites.is_empty());
}
```

**Step 2: Run test to verify it fails**
Run: `cargo test -p minibox-bench dry_run_skips_execution`
Expected: FAIL

**Step 3: Write minimal implementation**
Implement `--dry-run` behavior and wire `main()` to run benchmark and write outputs.

**Step 4: Run test to verify it passes**
Run: `cargo test dry_run_skips_execution`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "bench add dry-run and main"
```

---

### Task 9: Document Usage

**Files:**

- Modify: `bench/README.md`
- Modify: `USAGE.md`

**Step 1: Write doc updates**
Add how to build and run benchmarks, expected outputs, and runtime notes.

**Step 2: Commit**

```bash
git add bench/README.md USAGE.md
git commit -m "docs add benchmark usage"
```

---

### Task 10: Final Verification (Updated)

**Run:**

- `cargo test -p minibox-bench`
- `cargo build -p minibox-bench`
- `./target/debug/minibox-bench --dry-run`

**Expected:** All tests pass, dry-run succeeds.

---

Plan complete and saved to `docs/plans/2026-03-16-minibox-benchmark.md`.

## Implementation Notes (Applied)

- Implementation lives under `crates/minibox-bench` instead of `bench/minibox_bench.rs`.
- The `minibox` binary path is configurable via `MINIBOX_BIN`.
- `--suite`, `--cold/--warm`, and `--dry-run` control suite selection and execution.
- Failed commands are recorded in `errors` and excluded from stats.
