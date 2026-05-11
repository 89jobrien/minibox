use anyhow::{Context, Result};
use std::{env, fs, path::Path};
use xshell::{Shell, cmd};

use crate::{bump, docs_lint};

/// Pre-commit gate: version bump → fmt → clippy --fix → lint check (macOS-safe, fast)
///
/// Release build and conformance suite run at pre-push time, not here.
pub fn pre_commit(sh: &Shell) -> Result<()> {
    let rust_staged = staged_rust_files(sh)?;

    if rust_staged {
        auto_bump(sh)?;
        cmd!(sh, "cargo fmt --all").run().context("fmt failed")?;
        // Re-stage any files rustfmt modified so the commit includes the formatted versions.
        // Exclude .worktrees/ to avoid git trying to lock index files inside worktree .git files.
        cmd!(sh, "git add -u -- . :!.worktrees")
            .run()
            .context("git add -u after fmt failed")?;
        cmd!(
            sh,
            "cargo clippy -p minibox -p minibox-macros -p mbx -p minibox-core -p macbox -p miniboxd --fix --allow-dirty --allow-staged"
        )
        .run()
        .context("clippy --fix failed")?;
        // Re-stage any files clippy --fix modified.
        cmd!(sh, "git add -u -- . :!.worktrees")
            .run()
            .context("git add -u after clippy --fix failed")?;
        cmd!(sh, "cargo fmt --all --check")
            .run()
            .context("fmt-check failed")?;
        cmd!(
            sh,
            "cargo clippy -p minibox -p minibox-macros -p mbx -p minibox-core -p macbox -p miniboxd -- -D warnings"
        )
        .run()
        .context("lint failed")?;
    }

    // Docs frontmatter lint (fast, no external tools).
    let root = sh.current_dir();
    docs_lint::lint_docs(&root).context("docs-lint failed")?;
    // Warn (non-fatal) if generated artifacts are tracked by git.
    check_repo_cleanliness(sh);
    eprintln!("pre-commit checks passed");
    Ok(())
}

/// Pre-push gate: release build → lib tests → conformance suite
///
/// One release compile covers both nextest (--release) and the conformance
/// harness, so the full gate costs a single incremental release build.
pub fn prepush(sh: &Shell) -> Result<()> {
    if !pushed_rust_files(sh)? {
        eprintln!("pre-push: no Rust files in push range, skipping build and tests");
        return Ok(());
    }
    cmd!(
        sh,
        "cargo build --release -p minibox -p minibox-macros -p mbx -p minibox-core -p miniboxd"
    )
    .run()
    .context("release build failed")?;
    cmd!(
        sh,
        "cargo nextest run --release -p minibox -p minibox-macros -p mbx -p minibox-core --lib"
    )
    .run()
    .context("nextest failed")?;
    test_conformance(sh)?;
    Ok(())
}

/// Unit + integration tests (any platform).
///
/// Runs the full workspace test suite via nextest, excluding test files that
/// require Linux root/cgroups (those are gated with `#[cfg(target_os = "linux")]`
/// and skipped automatically on macOS) and the protocol e2e tests (which need
/// a pre-built binary and run separately via `test-e2e`).
pub fn test_unit(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo nextest run --workspace --exclude miniboxd")
        .run()
        .context("nextest workspace tests failed")?;
    // Run miniboxd lib tests (excludes integration test files that need Linux root).
    cmd!(sh, "cargo nextest run -p miniboxd --lib")
        .run()
        .context("miniboxd lib tests failed")?;
    Ok(())
}

/// Conformance suite: builds and runs the `minibox-conformance` harness.
///
/// The harness executes all adapter conformance tests and emits JSON + JUnit XML
/// reports to `artifacts/conformance/` via the `generate-report` binary.
///
/// Set `CONFORMANCE_ADAPTER=<name>` to restrict to a single adapter.
/// Set `CONFORMANCE_ARTIFACT_DIR=<path>` to override the output directory.
pub fn test_conformance(sh: &Shell) -> Result<()> {
    // Build the harness binaries first so errors surface before test execution.
    cmd!(sh, "cargo build -p minibox-conformance --bins")
        .run()
        .context("failed to build minibox-conformance")?;

    // Run the full suite via `run-conformance` (fast, exits 1 on failure).
    let output = cmd!(sh, "cargo run -p minibox-conformance --bin run-conformance")
        .output()
        .context("run-conformance failed to launch")?;

    // Surface test output regardless of pass/fail.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.trim().is_empty() {
        eprint!("{stdout}");
    }
    if !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }

    if !output.status.success() {
        anyhow::bail!("conformance tests failed");
    }

    // Generate JSON + JUnit XML reports.
    let report_output = cmd!(sh, "cargo run -p minibox-conformance --bin generate-report")
        .output()
        .context("generate-report failed to launch")?;

    if !report_output.status.success() {
        let code = report_output
            .status
            .code()
            .map_or("signal".to_string(), |c| c.to_string());
        let stderr = String::from_utf8_lossy(&report_output.stderr);
        let stdout = String::from_utf8_lossy(&report_output.stdout);
        anyhow::bail!("generate-report exited with {code}\nstderr: {stderr}\nstdout: {stdout}");
    }

    let report_stdout = String::from_utf8_lossy(&report_output.stdout);
    for line in report_stdout.lines() {
        if line.starts_with("conformance:") {
            if let Some(rest) = line.strip_prefix("conformance:json=") {
                eprintln!("  report.json  : {rest}");
            } else if let Some(rest) = line.strip_prefix("conformance:junit=") {
                eprintln!("  report.junit : {rest}");
            } else if let Some(rest) = line.strip_prefix("conformance:summary ") {
                eprintln!("  summary      : {rest}");
            }
        }
    }

    eprintln!("conformance suite passed");
    Ok(())
}

/// krun adapter conformance tests (macOS HVF / Linux KVM, requires MINIBOX_KRUN_TESTS=1).
///
/// Run serially — parallel krun invocations collide on the VM hypervisor socket.
pub fn test_krun_conformance(sh: &Shell) -> Result<()> {
    let _env = sh.push_env("MINIBOX_KRUN_TESTS", "1");
    cmd!(
        sh,
        "cargo test -p macbox --test krun_conformance_tests -- --test-threads=1"
    )
    .run()
    .context("krun_conformance_tests failed")?;
    cmd!(
        sh,
        "cargo test -p macbox --test krun_adapter_conformance -- --test-threads=1"
    )
    .run()
    .context("krun_adapter_conformance tests failed")?;
    eprintln!("krun conformance suite passed");
    Ok(())
}

/// Property-based tests (proptest)
pub fn test_property(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo test --release -p minibox --test proptest_suite")
        .run()
        .context("minibox property tests failed")?;
    cmd!(
        sh,
        "cargo test --release -p minibox --test daemon_proptest_suite"
    )
    .run()
    .context("daemon property tests failed")?;
    Ok(())
}

/// Cgroup + integration tests (Linux, root required)
pub fn test_integration(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo test --release -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture"
    )
    .run()
    .context("cgroup tests failed")?;
    cmd!(
        sh,
        "cargo test --release -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture"
    )
    .run()
    .context("integration tests failed")?;
    Ok(())
}

/// Protocol e2e tests — any platform, no root required.
///
/// Starts a real `miniboxd` process and exercises the JSON-over-Unix-socket
/// protocol without Linux namespaces, cgroups, or root. On macOS the daemon
/// dispatches to macbox; on Linux it uses the native adapter (but avoids
/// operations that require root).
pub fn test_e2e(sh: &Shell) -> Result<()> {
    // Build the daemon binary first so find_binary() can locate it.
    cmd!(sh, "cargo build -p miniboxd")
        .run()
        .context("failed to build miniboxd for protocol e2e tests")?;
    cmd!(
        sh,
        "cargo nextest run -p miniboxd --test protocol_e2e_tests -j 1"
    )
    .run()
    .context("protocol e2e tests failed")?;
    eprintln!("protocol e2e tests passed");
    Ok(())
}

/// System tests: full-stack daemon+CLI tests (Linux, root, cgroups v2 required).
///
/// Renamed from `test_e2e_suite` — these tests exercise real kernel facilities
/// (namespaces, overlay FS, cgroups v2) and live above integration tests in
/// the tier hierarchy.
pub fn test_system_suite(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo build --release")
        .run()
        .context("build failed")?;

    cmd!(
        sh,
        "cargo test -p miniboxd --test system_tests --release --no-run"
    )
    .run()
    .context("failed to build system test binary")?;

    let binary = find_test_binary("target/release/deps", "system_tests")
        .context("could not locate system test binary in target/release/deps")?;

    let bin_dir = env::current_dir()?.join("target/release");
    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {binary} --test-threads=1 --nocapture"
    )
    .run()
    .context("system tests failed")?;
    Ok(())
}

/// Daemon+CLI e2e tests (Linux, root required)
///
/// Deprecated alias for `test_system_suite`. Kept for backward compatibility
/// with existing CI jobs that reference `test-e2e-suite`.
pub fn test_e2e_suite(sh: &Shell) -> Result<()> {
    test_system_suite(sh)
}

/// Sandbox contract tests (Linux, root, Docker Hub required)
pub fn test_sandbox(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo build --release")
        .run()
        .context("build failed")?;

    cmd!(
        sh,
        "cargo test -p miniboxd --test sandbox_tests --release --no-run"
    )
    .run()
    .context("failed to build sandbox test binary")?;

    let binary = find_test_binary("target/release/deps", "sandbox_tests")
        .context("could not locate sandbox test binary in target/release/deps")?;

    let bin_dir = env::current_dir()?.join("target/release");
    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {binary} --test-threads=1 --ignored --nocapture"
    )
    .run()
    .context("sandbox tests failed")?;
    Ok(())
}

/// Coverage-check gate: run llvm-cov on minibox, parse handler.rs function coverage,
/// and exit non-zero when it falls below the 80% threshold.
///
/// The function scrapes the per-file summary line emitted by `cargo llvm-cov` in its
/// default text output, which looks like:
///
/// ```text
/// handler.rs          |  80.00 |  ...  |  82.35 |  ...
/// ```
///
/// Column order (0-based): Filename | Line% | Lines | Fns% | Fns | ...
/// We look for the function-coverage column (index 3) on the `handler.rs` row.
pub fn coverage_check(sh: &Shell) -> Result<()> {
    const THRESHOLD: f64 = 80.0;

    // Run llvm-cov with text output so we can parse per-file function coverage.
    let output = cmd!(sh, "cargo llvm-cov nextest --package minibox --text")
        .output()
        .context("cargo llvm-cov nextest failed")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("cargo llvm-cov nextest failed:\n{stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let coverage = parse_handler_fn_coverage(&stdout)
        .context("could not find handler.rs function coverage in llvm-cov output")?;

    let status = if coverage >= THRESHOLD {
        "PASS"
    } else {
        "FAIL"
    };
    eprintln!(
        "handler.rs function coverage: {coverage:.2}% (threshold: {THRESHOLD:.2}%) [{status}]"
    );

    if coverage < THRESHOLD {
        anyhow::bail!(
            "handler.rs function coverage {coverage:.2}% is below the {THRESHOLD:.2}% threshold"
        );
    }

    Ok(())
}

/// Warn (non-fatal) if generated artifacts are tracked by git.
///
/// Checks for files under `target/`, `artifacts/`, `traces/`, or with `.profraw`/`.crate`
/// extensions that should never be committed. Prints a warning for each found file but does
/// not fail — callers that need a hard failure should use `check_repo_cleanliness_strict`.
pub fn check_repo_cleanliness(sh: &Shell) {
    let patterns = &[
        "target/",
        "artifacts/conformance/",
        "traces/",
        "*.profraw",
        "*.crate",
    ];

    let output = cmd!(sh, "git ls-files")
        .output()
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: vec![],
            stderr: vec![],
        });

    let tracked = String::from_utf8_lossy(&output.stdout);
    let mut found: Vec<&str> = Vec::new();

    for line in tracked.lines() {
        for pat in patterns {
            let matches = if pat.ends_with('/') {
                line.starts_with(pat) || line.contains(&format!("/{pat}"))
            } else if pat.starts_with("*.") {
                let ext = pat.trim_start_matches('*');
                line.ends_with(ext)
            } else {
                line == *pat
            };
            if matches {
                found.push(line);
                break;
            }
        }
    }

    if !found.is_empty() {
        eprintln!("warning: the following generated artifacts are tracked by git:");
        for f in &found {
            eprintln!("  {f}");
        }
        eprintln!("warning: run `git rm -r --cached <path>` to untrack them (see issue #154)");
    }
}

/// Parse the function-coverage percentage for `handler.rs` from `cargo llvm-cov --text` output.
///
/// The text table has pipe-delimited columns. We look for a row whose first column contains
/// `handler.rs` and return the value from the "Fns%" column (column index 3, 1-based: 4th).
fn parse_handler_fn_coverage(output: &str) -> Option<f64> {
    for line in output.lines() {
        // Only process lines that mention the target file.
        if !line.contains("handler.rs") {
            continue;
        }
        // Columns are pipe-separated; trim whitespace around each segment.
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        // Expected layout: Filename | Line% | Lines | Fns% | Fns | ...
        // Index:              0         1       2       3      4
        if cols.len() >= 4
            && let Ok(pct) = cols[3].trim_end_matches('%').parse::<f64>()
        {
            return Some(pct);
        }
    }
    None
}

/// Returns true if any `.rs` or `.toml` files (excluding `Cargo.lock`) differ between
/// HEAD and the upstream tracking branch. Falls back to `true` when upstream is absent
/// (new branch) so tests always run in that case.
fn pushed_rust_files(sh: &Shell) -> Result<bool> {
    let range = "@{u}..HEAD";
    let out = cmd!(sh, "git diff --name-only {range}").output();
    match out {
        Err(_) => Ok(true), // no upstream — run tests
        Ok(out) => {
            let diff = String::from_utf8_lossy(&out.stdout);
            Ok(diff
                .lines()
                .any(|l| (l.ends_with(".rs") || l.ends_with(".toml")) && l != "Cargo.lock"))
        }
    }
}

/// Returns true if any `.rs` or `.toml` files (excluding `Cargo.lock`) are staged.
fn staged_rust_files(sh: &Shell) -> Result<bool> {
    let staged = cmd!(sh, "git diff --cached --name-only")
        .output()
        .context("git diff --cached failed")?;
    let staged = String::from_utf8_lossy(&staged.stdout);
    Ok(staged
        .lines()
        .any(|l| (l.ends_with(".rs") || l.ends_with(".toml")) && l != "Cargo.lock"))
}

/// Auto-bump workspace version based on staged Rust changes.
///
/// - New `.rs` or `.toml` files → minor bump (rate-limited to once per day)
/// - Modified `.rs` or `.toml` files → patch bump
///
/// After bumping, re-stages `Cargo.toml` so the version change is included
/// in the commit.
fn auto_bump(sh: &Shell) -> Result<()> {
    let new_files = cmd!(sh, "git diff --cached --name-only --diff-filter=A")
        .output()
        .context("git diff --cached --diff-filter=A failed")?;
    let new_files = String::from_utf8_lossy(&new_files.stdout);
    let has_new_rust = new_files
        .lines()
        .any(|l| l.ends_with(".rs") || l.ends_with(".toml"));

    let level = if has_new_rust { "minor" } else { "patch" };
    let root = sh.current_dir();
    bump::bump(&root, level)?;

    cmd!(sh, "git add Cargo.toml")
        .run()
        .context("git add Cargo.toml after bump failed")?;

    Ok(())
}

/// Find the most recently modified test binary matching a name prefix (no `.d` extension)
pub fn find_test_binary(deps_dir: &str, prefix: &str) -> Option<std::path::PathBuf> {
    let dir = Path::new(deps_dir);
    let mut candidates: Vec<_> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let is_file = e.file_type().is_ok_and(|t| t.is_file());
            name.starts_with(prefix) && !name.ends_with(".d") && is_file
        })
        .collect();
    candidates.sort_by_key(|e| e.metadata().ok()?.modified().ok());
    candidates.last().map(|e| e.path())
}
