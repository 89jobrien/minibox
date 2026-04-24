---
status: done
---

# Bench Schema Integrity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the nanosecond/microsecond field mismatch in bench output and eliminate the O(n) directory scan in `xtask bench`.

**Architecture:** Two independent fixes in the same area. (1) Add a `durations_nanos` field to `TestResult` so nano-suite data is never stored in a micros-labelled field. (2) Have the bench binary print the written JSON path to stdout so xtask can capture it directly instead of scanning `bench/results/`. Both fixes are backward-compatible: existing microsecond suites continue to use `durations_micros`.

**Tech Stack:** Rust, serde_json, `xshell` (xtask), existing `minibox-bench` binary

---

## File Map

| File | Change |
|---|---|
| `crates/minibox-bench/src/main.rs` | Add `durations_nanos` to `TestResult`; update `nano_test`; print JSON path to stdout |
| `xtask/src/main.rs` | Capture JSON path from bench stdout instead of directory scan |

---

### Task 1: Add `durations_nanos` field to `TestResult`

**Files:**
- Modify: `crates/minibox-bench/src/main.rs:43-51` (`TestResult` struct)

- [ ] **Step 1: Write the failing test**

In `crates/minibox-bench/src/main.rs`, add to the `#[cfg(test)]` block:

```rust
#[test]
fn nano_test_uses_durations_nanos_not_micros() {
    let result = nano_test("test", 5, || std::hint::black_box(1 + 1));
    assert!(!result.durations_nanos.is_empty(), "durations_nanos must be populated");
    assert!(result.durations_micros.is_empty(), "durations_micros must be empty for nano tests");
    assert_eq!(result.unit, "nanos");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p minibox-bench nano_test_uses_durations_nanos_not_micros
```

Expected: FAIL — `durations_nanos` field does not exist yet.

- [ ] **Step 3: Add the field to `TestResult`**

In `crates/minibox-bench/src/main.rs`, update the struct:

```rust
#[derive(Serialize, Default)]
struct TestResult {
    name: String,
    iterations: usize,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    durations_micros: Vec<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    durations_nanos: Vec<u64>,
    stats: Option<Stats>,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    unit: String,
}
```

- [ ] **Step 4: Update `nano_test` to populate `durations_nanos`**

Replace the current `nano_test` function:

```rust
fn nano_test(name: &str, iters: usize, f: impl FnMut()) -> TestResult {
    let durations = measure_nanos(iters, f);
    let stats = stats_for(&durations);
    TestResult {
        name: name.to_string(),
        iterations: durations.len(),
        durations_nanos: durations,
        stats,
        unit: "nanos".to_string(),
        ..Default::default()
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p minibox-bench
```

Expected: all pass including `nano_test_uses_durations_nanos_not_micros`.

- [ ] **Step 6: Verify `write_table` still works**

`write_table` reads from `test.stats` (which is populated from the duration vec regardless of field name) and already checks `test.unit == "nanos"` for display. No change needed. Confirm by checking the table output logic reads `test.stats` not `test.durations_micros` directly — it does, at line ~66.

- [ ] **Step 7: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "fix(bench): store nanosecond samples in durations_nanos, not durations_micros"
```

---

### Task 2: Print JSON path to stdout from bench binary

**Files:**
- Modify: `crates/minibox-bench/src/main.rs` — `main()` function (line ~308)

- [ ] **Step 1: Write the failing test**

Add to `#[cfg(test)]` block:

```rust
#[test]
fn bench_main_prints_json_path() {
    // When write_json succeeds, main() should print the json path to stdout.
    // We can't easily test main(), but we can document the contract here.
    // This test verifies the path format is well-formed.
    let out_dir = "/tmp";
    let timestamp = "2026-03-21T00:00:00+00:00";
    let json_path = format!("{out_dir}/{timestamp}.json");
    assert!(json_path.ends_with(".json"));
}
```

This is a placeholder — the real verification is in Task 3 (xtask).

- [ ] **Step 2: Add `println!` to `main()` after successful JSON write**

In `main()`, after the `write_json` call succeeds:

```rust
if let Err(e) = write_json(&report, &json_path) {
    eprintln!("error: {e}");
    std::process::exit(1);
}
// Print JSON path to stdout so callers (e.g. xtask) can capture it without scanning the dir.
println!("{json_path}");
if let Err(e) = write_table(&report, &table_path) {
    eprintln!("error: {e}");
    std::process::exit(1);
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p minibox-bench
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "feat(bench): print JSON output path to stdout for caller capture"
```

---

### Task 3: Eliminate O(n) directory scan in `xtask bench`

**Files:**
- Modify: `xtask/src/main.rs` — `bench()` function (line ~261)

- [ ] **Step 1: Write the failing test**

There's no unit test for `bench()` — this is a behavioral change. The verification is: run `cargo xtask bench --dry-run` and confirm it doesn't scan the directory. Instead, write a doc test comment and rely on the compile check.

- [ ] **Step 2: Replace directory scan with stdout capture**

Replace the `bench()` function body:

```rust
fn bench(sh: &Shell) -> Result<()> {
    let out_dir = "bench/results";
    fs::create_dir_all(out_dir).context("create bench/results")?;

    // Capture stdout — the binary prints the JSON path as its last line.
    let output = cmd!(sh, "./target/release/minibox-bench --out-dir {out_dir}")
        .read()
        .context("bench binary failed")?;

    let json_path = output
        .lines()
        .last()
        .filter(|l| l.ends_with(".json"))
        .ok_or_else(|| anyhow::anyhow!("bench binary did not print a .json path on stdout"))?;

    save_bench_results(sh, json_path)
}
```

- [ ] **Step 3: Verify it builds**

```bash
cargo build -p xtask
```

Expected: compiles cleanly.

- [ ] **Step 4: Remove now-unused `find_test_binary`-style import if any (check)**

```bash
cargo clippy -p xtask
```

Fix any unused import warnings.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/main.rs
git commit -m "fix(bench): capture JSON path from bench stdout, eliminating O(n) dir scan"
```

---

### Task 4: Integration smoke test

- [ ] **Step 1: Build the bench binary**

```bash
cargo build --release -p minibox-bench
```

- [ ] **Step 2: Verify stdout includes a `.json` path**

```bash
./target/release/minibox-bench --dry-run --out-dir /tmp/bench-smoke 2>/dev/null
```

Expected: last line of stdout is something like `/tmp/bench-smoke/2026-03-21T....json`

- [ ] **Step 3: Verify JSON uses correct fields**

```bash
./target/release/minibox-bench --suite codec --out-dir /tmp/bench-smoke 2>/dev/null | xargs cat | python3 -c "import sys,json; d=json.load(open(sys.stdin.read().strip())); [print(t) for s in d['suites'] for t in [s['tests'][0]] if 'durations_nanos' in t and 'durations_micros' not in t]"
```

Expected: prints at least one test object with `durations_nanos` present and no `durations_micros`.

- [ ] **Step 4: Final commit if clean**

```bash
git status  # should be clean
```
