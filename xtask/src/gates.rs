use anyhow::{Context, Result};
use std::{fs, path::Path};
use xshell::{Shell, cmd};

use crate::{borrow_fixtures, bump, docs_lint, utils::cargo_target_dir};

/// Agent config directories that trigger agentlint.
const AGENT_DIRS: &[&str] = &[".claude/", ".codex/", ".agents/", ".cursor/"];

/// Lint gate: fmt --check + clippy + cargo check (matches CI lint jobs).
///
/// Includes all workspace crates. On macOS, macbox is included in clippy;
/// on Linux it compiles but has gated code — still linted for syntax.
pub fn lint(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo fmt --all --check")
        .run()
        .context("cargo fmt --check failed")?;
    cmd!(
        sh,
        "cargo clippy -p minibox -p minibox-macros -p mbx -p minibox-core -p macbox -p miniboxd -p winbox -- -D warnings"
    )
    .run()
    .context("cargo clippy failed")?;
    cmd!(sh, "cargo check --workspace")
        .run()
        .context("cargo check --workspace failed")?;
    eprintln!("lint gate passed");
    Ok(())
}

/// Read-only local verification gate: fmt check, workspace check, clippy,
/// borrow fixtures, and docs lint. Does not modify files.
pub fn verify(sh: &Shell, root: &Path) -> Result<()> {
    eprintln!("--- verify: fmt check ---");
    cmd!(sh, "cargo fmt --all --check")
        .run()
        .context("cargo fmt --check failed")?;

    eprintln!("--- verify: workspace check ---");
    cmd!(sh, "cargo check --workspace")
        .run()
        .context("cargo check --workspace failed")?;

    eprintln!("--- verify: clippy ---");
    cmd!(
        sh,
        "cargo clippy -p minibox -p minibox-macros -p mbx -p minibox-core -p macbox -p miniboxd -p winbox -- -D warnings"
    )
    .run()
    .context("cargo clippy failed")?;

    eprintln!("--- verify: borrow fixtures ---");
    borrow_fixtures::run(root)?;

    eprintln!("--- verify: docs lint ---");
    docs_lint::lint_docs(root)?;

    eprintln!("verify gate passed");
    Ok(())
}

/// Fix gate: version bump + fmt + clippy --fix + re-stage (macOS-safe, fast)
///
/// This mutates files and the git index. Use `pre-commit` for validation-only checks.
pub fn fix(sh: &Shell) -> Result<()> {
    let rust_staged = staged_rust_files(sh)?;

    if rust_staged {
        cmd!(sh, "cargo fmt --all").run().context("fmt failed")?;
        // Re-stage any files rustfmt modified so the commit includes the formatted versions.
        // Exclude .worktrees/ to avoid git trying to lock index files inside worktree .git files.
        cmd!(sh, "git add -u -- . :!.worktrees")
            .run()
            .context("git add -u after fmt failed")?;
        auto_bump(sh)?;
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
    }

    eprintln!("fix gate passed");
    Ok(())
}

/// Pre-commit gate: validation-only checks (macOS-safe, fast)
///
/// Never stages or edits files. Use `fix` for auto-formatting and clippy --fix.
/// Release build and conformance suite run at pre-push time, not here.
pub fn pre_commit(sh: &Shell) -> Result<()> {
    let rust_staged = staged_rust_files(sh)?;

    if rust_staged {
        cmd!(sh, "cargo fmt --all --check")
            .run()
            .context("fmt-check failed")?;
        cmd!(
            sh,
            "cargo clippy -p minibox -p minibox-macros -p mbx -p minibox-core -p macbox -p miniboxd -- -D warnings"
        )
        .run()
        .context("clippy failed")?;
    }

    // Agent config lint: validate .claude/, .codex/, .agents/, .cursor/ files.
    if staged_agent_files(sh)? {
        agentlint_staged(sh).context("agentlint failed")?;
    }

    // Workflow lint: run actionlint when .github/workflows/ files are staged.
    if staged_workflow_files(sh)? {
        let config = ".github/actionlint.yaml";
        // actionlint does not accept a bare directory path; collect .yml files explicitly.
        let wf_files: Vec<_> = fs::read_dir(".github/workflows")
            .context("read .github/workflows")?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "yml" || ext == "yaml")
            })
            .map(|e| e.path().to_string_lossy().into_owned())
            .collect();
        cmd!(sh, "actionlint -config-file {config} {wf_files...}")
            .run()
            .context("actionlint failed — fix workflow errors before committing")?;
    }

    // Docs frontmatter lint (fast, no external tools).
    let root = sh.current_dir();
    docs_lint::lint_docs(&root).context("docs-lint failed")?;
    // Keep the FEATURE_MATRIX Last-updated stamp current (idempotent).
    crate::feature_matrix_date::update_feature_matrix_date(&root)
        .context("update-feature-matrix-date failed")?;
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
    let fail_fast = fail_fast_flag();
    cmd!(
        sh,
        "cargo nextest run --release -p minibox -p minibox-macros -p mbx -p minibox-core --lib {fail_fast...}"
    )
    .run()
    .context("nextest failed")?;
    test_conformance(sh)?;
    Ok(())
}

/// Unit tests (any platform, matches CI).
///
/// Runs `--lib` tests only (no integration test files that require Linux root
/// or a running daemon). On Linux, excludes macbox (macOS-only crate) to match
/// CI behavior. Integration and e2e tests have dedicated gates.
///
/// Set `MINIBOX_FAIL_FAST=true` to stop on the first test failure.
pub fn test_unit(sh: &Shell) -> Result<()> {
    let fail_fast = fail_fast_flag();
    if cfg!(target_os = "macos") {
        cmd!(sh, "cargo nextest run --workspace --lib {fail_fast...}")
            .run()
            .context("nextest workspace --lib tests failed")?;
    } else {
        cmd!(
            sh,
            "cargo nextest run --workspace --exclude macbox --lib {fail_fast...}"
        )
        .run()
        .context("nextest workspace --lib tests failed")?;
    }
    Ok(())
}

/// Conformance suite: builds and runs the `minibox-testsuite` harness.
///
/// The harness executes all adapter conformance tests and emits JSON + JUnit XML
/// reports to `artifacts/conformance/` via the `generate-report` binary.
///
/// Set `CONFORMANCE_ADAPTER=<name>` to restrict to a single adapter.
/// Set `CONFORMANCE_ARTIFACT_DIR=<path>` to override the output directory.
pub fn test_conformance(sh: &Shell) -> Result<()> {
    // Build the harness binaries first so errors surface before test execution.
    cmd!(sh, "cargo build --release -p minibox-testsuite --bins")
        .run()
        .context("failed to build minibox-testsuite")?;

    // Run the full suite via `run-conformance` (fast, exits 1 on failure).
    let output = cmd!(
        sh,
        "cargo run --release -p minibox-testsuite --bin run-conformance"
    )
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
    let report_output = cmd!(
        sh,
        "cargo run --release -p minibox-testsuite --bin generate-report"
    )
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
    // smolvm must be on PATH for krun conformance tests to succeed.
    if cmd!(sh, "which smolvm").quiet().run().is_err() {
        eprintln!("skipping krun conformance: smolvm not found on PATH");
        return Ok(());
    }
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

/// Turmoil network simulation tests
pub fn test_turmoil(sh: &Shell) -> Result<()> {
    let fail_fast = fail_fast_flag();
    cmd!(
        sh,
        "cargo nextest run -p minibox --test turmoil_network_tests {fail_fast...}"
    )
    .run()
    .context("turmoil network simulation tests failed")?;
    eprintln!("turmoil network simulation tests passed");
    Ok(())
}

/// Property-based tests (proptest)
/// Shuttle concurrency tests (deterministic random scheduling).
pub fn test_shuttle(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo nextest run -p minibox --test shuttle_concurrency"
    )
    .run()
    .context("shuttle concurrency tests failed")?;
    Ok(())
}

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

/// Quickcheck property-based tests (cross-platform).
pub fn test_quickcheck(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo test --release -p minibox-core --test quickcheck_properties"
    )
    .run()
    .context("minibox-core quickcheck tests failed")?;
    cmd!(
        sh,
        "cargo test --release -p minibox --test quickcheck_properties"
    )
    .run()
    .context("minibox quickcheck tests failed")?;
    eprintln!("quickcheck property tests passed");
    Ok(())
}

/// Cgroup + integration tests (Linux, root required)
///
/// Builds release binaries first, then runs each test suite under `sudo -E`
/// so the tests have the kernel privileges they need (cgroups v2, namespaces).
/// `MINIBOX_TEST_BIN_DIR` is forwarded so helpers can locate `miniboxd`/`mbx`.
pub fn test_integration(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo build --release -p miniboxd -p mbx")
        .run()
        .context("release build failed")?;

    // Build cgroup test binary without running.
    cmd!(
        sh,
        "cargo test --release -p miniboxd --test cgroup_tests --no-run"
    )
    .run()
    .context("failed to build cgroup_tests binary")?;

    // Build integration test binary without running.
    cmd!(
        sh,
        "cargo test --release -p miniboxd --test integration_tests --no-run"
    )
    .run()
    .context("failed to build integration_tests binary")?;

    let target = cargo_target_dir();
    let bin_dir = target.join("release");

    let cgroup_bin = find_test_binary(
        &target.join("release/deps").to_string_lossy(),
        "cgroup_tests",
    )
    .context("could not locate cgroup_tests binary")?;

    let integration_bin = find_test_binary(
        &target.join("release/deps").to_string_lossy(),
        "integration_tests",
    )
    .context("could not locate integration_tests binary")?;

    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {cgroup_bin} --test-threads=1 --nocapture"
    )
    .run()
    .context("cgroup tests failed")?;

    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {integration_bin} --test-threads=1 --nocapture --ignored"
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
///
/// Uses `--release` to match CI behavior and catch optimisation-sensitive bugs.
pub fn test_e2e(sh: &Shell) -> Result<()> {
    // Build daemon + CLI in release mode so find_binary() can locate them.
    cmd!(sh, "cargo build --release -p miniboxd -p mbx")
        .run()
        .context("failed to build miniboxd/mbx for protocol e2e tests")?;
    cmd!(
        sh,
        "cargo test -p miniboxd --test protocol_e2e_tests --release -- --test-threads=1 --nocapture"
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

    let target = cargo_target_dir();
    let binary = find_test_binary(
        &target.join("release/deps").to_string_lossy(),
        "system_tests",
    )
    .context("could not locate system test binary in target/release/deps")?;

    let bin_dir = target.join("release");
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

    let target = cargo_target_dir();
    let binary = find_test_binary(
        &target.join("release/deps").to_string_lossy(),
        "sandbox_tests",
    )
    .context("could not locate sandbox test binary in target/release/deps")?;

    let bin_dir = target.join("release");
    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {binary} --test-threads=1 --ignored --nocapture"
    )
    .run()
    .context("sandbox tests failed")?;
    Ok(())
}

/// Full HTML + lcov coverage report for local inspection and CI upload.
///
/// Runs `cargo llvm-cov nextest` on the two main crates and writes:
/// - `target/coverage/html/` — browseable HTML (open `index.html`)
/// - `target/coverage/lcov.info` — lcov format for codecov/CI badge upload
///
/// Pass `--open` to open the HTML report on macOS after generation.
/// Pass `--lcov-only` to skip HTML (faster, for CI).
/// Pass `--html-only` to skip lcov (default for local dev).
pub fn coverage(sh: &Shell, open: bool, lcov_only: bool, html_only: bool) -> Result<()> {
    let cov_dir = sh.current_dir().join("target/coverage");
    std::fs::create_dir_all(&cov_dir).context("create target/coverage dir")?;

    if !lcov_only {
        let html_dir = cov_dir.join("html");
        cmd!(
            sh,
            "cargo llvm-cov nextest -p minibox -p minibox-core --html --output-dir {html_dir}"
        )
        .run()
        .context("cargo llvm-cov nextest --html failed (is cargo-llvm-cov installed?)")?;

        eprintln!(
            "coverage: HTML report → file://{}/index.html",
            html_dir.display()
        );

        if open && cfg!(target_os = "macos") {
            let index = html_dir.join("index.html");
            cmd!(sh, "open {index}").run().ok();
        }
    }

    if !html_only {
        let lcov_path = cov_dir.join("lcov.info");
        cmd!(
            sh,
            "cargo llvm-cov nextest -p minibox -p minibox-core --lcov --output-path {lcov_path}"
        )
        .run()
        .context("cargo llvm-cov nextest --lcov failed")?;

        eprintln!("coverage: lcov         → {}", lcov_path.display());
    }

    Ok(())
}

/// Coverage-check gate: run llvm-cov on the handler module and fail when
/// function coverage drops below the threshold.
///
/// Uses `--json --summary-only` which emits a JSON document to stdout
/// containing per-file function coverage summaries. We aggregate all files
/// under `daemon/handler/` (the module was split from a single `handler.rs`
/// into `handler/mod.rs` + submodules) and compute a combined function
/// coverage percentage.
///
/// # Threshold rationale
///
/// LLVM's function-coverage counter treats every await-point state-machine
/// closure in an `async fn` as a separate function symbol. A typical
/// `async fn` with N `.await` points generates N+1 coverage symbols even
/// though they map to a single logical function. For the handler module,
/// which is almost entirely `async fn`, this inflates the denominator.
/// The practical ceiling is ~65%.
///
/// Threshold is set at 61% — just below the measured baseline — to catch
/// regressions while remaining achievable on macOS CI.
pub fn coverage_check(sh: &Shell) -> Result<()> {
    const THRESHOLD: f64 = 61.0;

    let output = cmd!(
        sh,
        "cargo llvm-cov nextest --package minibox --json --summary-only"
    )
    .output()
    .context("failed to spawn cargo llvm-cov nextest")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "cargo llvm-cov nextest exited with {}: {}",
            output.status,
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = parse_handler_fn_coverage(&stdout).with_context(|| {
        // Include a snippet of the JSON to aid debugging when the schema changes.
        let preview: String = stdout.chars().take(500).collect();
        format!(
            "could not extract handler module coverage from llvm-cov output \
             (no files matching `daemon/handler/`). JSON preview:\n{preview}"
        )
    })?;

    let status = if result.percent >= THRESHOLD {
        "PASS"
    } else {
        "FAIL"
    };

    // Per-file breakdown for visibility.
    for (name, count, covered) in &result.per_file {
        let pct = if *count > 0 {
            *covered as f64 / *count as f64 * 100.0
        } else {
            0.0
        };
        eprintln!("  {name}: {covered}/{count} fns ({pct:.1}%)");
    }
    eprintln!(
        "handler module function coverage: {:.2}% ({}/{} fns) \
         [threshold: {THRESHOLD:.2}%] [{status}]",
        result.percent, result.covered, result.count,
    );

    if result.percent < THRESHOLD {
        anyhow::bail!(
            "handler module function coverage {:.2}% ({}/{} fns) \
             is below the {THRESHOLD:.2}% threshold",
            result.percent,
            result.covered,
            result.count,
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

/// Aggregated handler module coverage result.
struct HandlerCoverage {
    /// Total function symbols across all handler files.
    count: u64,
    /// Covered function symbols across all handler files.
    covered: u64,
    /// Combined percentage (covered / count * 100).
    percent: f64,
    /// Per-file breakdown: (short filename, count, covered).
    per_file: Vec<(String, u64, u64)>,
}

/// Parse function coverage for the `daemon/handler/` module from
/// `cargo llvm-cov nextest --json --summary-only` stdout.
///
/// The JSON schema (llvm.coverage.json.export v3) looks like:
/// ```json
/// {"data":[{"files":[
///   {"filename":"…/daemon/handler/mod.rs",
///    "summary":{"functions":{"count":80,"covered":50,"percent":62.5}}},
///   {"filename":"…/daemon/handler/run.rs",
///    "summary":{"functions":{"count":30,"covered":20,"percent":66.7}}}
/// ]}]}
/// ```
///
/// We aggregate `functions.count` and `functions.covered` across every
/// file whose path contains `daemon/handler/`, then compute a combined
/// percentage.
fn parse_handler_fn_coverage(output: &str) -> Option<HandlerCoverage> {
    let root: serde_json::Value = serde_json::from_str(output).ok()?;

    let files = root
        .get("data")?
        .as_array()?
        .first()?
        .get("files")?
        .as_array()?;

    let mut total_count: u64 = 0;
    let mut total_covered: u64 = 0;
    let mut per_file: Vec<(String, u64, u64)> = Vec::new();

    for file in files {
        let filename = file.get("filename")?.as_str()?;
        if !filename.contains("daemon/handler/") {
            continue;
        }

        let fns = file.get("summary")?.get("functions")?;
        let count = fns.get("count")?.as_u64()?;
        let covered = fns.get("covered")?.as_u64()?;

        // Short name: everything after the last `daemon/`.
        let short = filename
            .rfind("daemon/")
            .map(|i| &filename[i..])
            .unwrap_or(filename);

        total_count += count;
        total_covered += covered;
        per_file.push((short.to_string(), count, covered));
    }

    if total_count == 0 {
        return None;
    }

    let percent = total_covered as f64 / total_count as f64 * 100.0;

    Some(HandlerCoverage {
        count: total_count,
        covered: total_covered,
        percent,
        per_file,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_handler_fn_coverage;

    /// Multi-file handler module: aggregates counts across submodules.
    #[test]
    fn aggregates_across_handler_submodules() {
        let sample = r#"{"data":[{"files":[
            {"filename":"/src/daemon/handler/mod.rs","summary":{"functions":{"count":80,"covered":50,"percent":62.5}}},
            {"filename":"/src/daemon/handler/run.rs","summary":{"functions":{"count":20,"covered":14,"percent":70.0}}},
            {"filename":"/src/daemon/handler/exec.rs","summary":{"functions":{"count":100,"covered":60,"percent":60.0}}},
            {"filename":"/src/daemon/state.rs","summary":{"functions":{"count":50,"covered":50,"percent":100.0}}}
        ]}]}"#;
        let result =
            parse_handler_fn_coverage(sample).expect("should aggregate handler/ submodules");
        // 80+20+100 = 200 total, 50+14+60 = 124 covered → 62.0%
        assert_eq!(result.count, 200);
        assert_eq!(result.covered, 124);
        assert!(
            (result.percent - 62.0).abs() < 0.01,
            "expected 62.0%, got {:.2}%",
            result.percent
        );
        assert_eq!(result.per_file.len(), 3, "state.rs should be excluded");
    }

    /// Single handler/mod.rs file (legacy-compatible path).
    #[test]
    fn single_handler_mod_file() {
        let sample = r#"{"data":[{"files":[
            {"filename":"/path/to/daemon/handler/mod.rs","summary":{"branches":{"count":0,"covered":0,"notcovered":0,"percent":0.0},"functions":{"count":205,"covered":84,"percent":40.97},"lines":{"count":3748,"covered":1532,"percent":40.88}}}
        ]}]}"#;
        let result = parse_handler_fn_coverage(sample).expect("should parse single handler/mod.rs");
        assert!(
            (result.percent - 40.97).abs() < 0.02,
            "expected ~40.97%, got {:.2}%",
            result.percent
        );
    }

    /// Exactly 61% should satisfy the threshold.
    #[test]
    fn recognises_61_percent() {
        let sample = r#"{"data":[{"files":[
            {"filename":"/path/to/daemon/handler/mod.rs","summary":{"functions":{"count":100,"covered":61,"percent":61.0}}}
        ]}]}"#;
        let result = parse_handler_fn_coverage(sample).expect("should find 61.0%");
        assert!(
            (result.percent - 61.0).abs() < 0.001,
            "expected 61.0%, got {:.2}%",
            result.percent
        );
    }

    /// JSON without any handler/ files returns None.
    #[test]
    fn ignores_unrelated_files() {
        let sample = r#"{"data":[{"files":[
            {"filename":"/path/to/mocks.rs","summary":{"functions":{"count":71,"covered":66,"percent":92.96}}}
        ]}]}"#;
        assert!(parse_handler_fn_coverage(sample).is_none());
    }

    /// Empty or invalid input returns None.
    #[test]
    fn returns_none_on_empty_input() {
        assert!(parse_handler_fn_coverage("").is_none());
        assert!(parse_handler_fn_coverage("not json").is_none());
    }

    /// The 61% threshold contract from the doc comment.
    #[test]
    fn coverage_threshold_is_61_percent() {
        const THRESHOLD: f64 = 61.0;
        let sample = r#"{"data":[{"files":[
            {"filename":"/path/to/daemon/handler/mod.rs","summary":{"functions":{"count":100,"covered":61,"percent":61.0}}}
        ]}]}"#;
        let result = parse_handler_fn_coverage(sample).expect("should parse 61.0%");
        assert!(
            result.percent >= THRESHOLD,
            "61.0% must satisfy the {THRESHOLD}% threshold; got {:.2}%",
            result.percent
        );
    }
}

/// Returns true if any staged files live under an agent config directory.
fn staged_agent_files(sh: &Shell) -> Result<bool> {
    let staged = cmd!(sh, "git diff --cached --name-only")
        .output()
        .context("git diff --cached failed")?;
    let staged = String::from_utf8_lossy(&staged.stdout);
    Ok(staged
        .lines()
        .any(|l| AGENT_DIRS.iter().any(|d| l.starts_with(d))))
}

/// Lint staged agent config files:
///   - `.json`  → parse with serde_json and report errors
///   - `.md`    → check required frontmatter keys (name, description)
///   - `.yaml`/`.yml` inside agent dirs → check with actionlint if in `.github/`, else YAML parse
pub fn agentlint_staged(sh: &Shell) -> Result<()> {
    let staged = cmd!(sh, "git diff --cached --name-only")
        .output()
        .context("git diff --cached failed")?;
    let staged = String::from_utf8_lossy(&staged.stdout);

    let agent_files: Vec<String> = staged
        .lines()
        .filter(|l| AGENT_DIRS.iter().any(|d| l.starts_with(d)))
        .map(|s| s.to_string())
        .collect();

    agentlint_check(&agent_files)
}

/// Lint all agent config files on disk (not just staged).
pub fn agentlint_all() -> Result<()> {
    let mut files = Vec::new();
    for dir in AGENT_DIRS {
        let dir_path = Path::new(dir);
        if dir_path.is_dir() {
            collect_files_recursive(dir_path, &mut files);
        }
    }
    agentlint_check(&files)
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, out);
        } else {
            out.push(path.to_string_lossy().to_string());
        }
    }
}

fn agentlint_check(agent_files: &[String]) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();
    let mut linted: usize = 0;

    for file in agent_files {
        let path = Path::new(file);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Skip binary/non-lintable files.
        if matches!(ext, "zip" | "png" | "jpg" | "gif") || filename == ".DS_Store" {
            continue;
        }

        let Ok(content) = fs::read_to_string(path) else {
            // File deleted, binary, or unreadable — skip.
            continue;
        };

        match ext {
            "json" => {
                linted += 1;
                if let Err(e) = serde_json::from_str::<serde_json::Value>(&content) {
                    errors.push(format!("{file}: JSON parse error: {e}"));
                }
            }
            "md" => {
                linted += 1;
                // Skills and agent docs should declare name and description.
                if content.starts_with("---") {
                    let missing: Vec<&str> = ["name:", "description:"]
                        .iter()
                        .copied()
                        .filter(|key| !content.contains(key))
                        .collect();
                    if !missing.is_empty() {
                        errors.push(format!(
                            "{file}: skill frontmatter missing required keys: {}",
                            missing.join(", ")
                        ));
                    }
                }
            }
            "yaml" | "yml" => {
                linted += 1;
                if content.trim().is_empty() {
                    errors.push(format!("{file}: empty YAML file"));
                }
            }
            "sh" | "nu" | "" => {
                linted += 1;
                // Scripts and extensionless hooks must have a shebang.
                lint_script(file, &content, &mut errors);
            }
            "rs" => {
                linted += 1;
                // Rust helper files: check they parse (basic syntax).
                // Full compilation is left to cargo; just ensure no empty files.
                if content.trim().is_empty() {
                    errors.push(format!("{file}: empty Rust file"));
                }
            }
            "txt" => {
                linted += 1;
                if content.trim().is_empty() {
                    errors.push(format!("{file}: empty text file"));
                }
            }
            _ => {
                // Unrecognized extension — still count as scanned but not linted.
            }
        }
    }

    if errors.is_empty() {
        eprintln!(
            "agentlint: scanned {} file(s), linted {}, 0 error(s)",
            agent_files.len(),
            linted,
        );
    } else {
        for e in &errors {
            eprintln!("agentlint: {e}");
        }
        anyhow::bail!(
            "agentlint: scanned {} file(s), linted {}, {} error(s)",
            agent_files.len(),
            linted,
            errors.len()
        );
    }

    Ok(())
}

/// Lint a script file (`.sh`, `.nu`, or extensionless hook).
fn lint_script(file: &str, content: &str, errors: &mut Vec<String>) {
    if content.trim().is_empty() {
        errors.push(format!("{file}: empty script"));
        return;
    }
    // Must start with a shebang.
    if !content.starts_with("#!") {
        errors.push(format!("{file}: missing shebang (expected #!/...)"));
        return;
    }
    let first_line = content.lines().next().unwrap_or("");
    // Shebang must reference a known interpreter.
    let valid_interpreters = [
        "bash", "sh", "zsh", "nu", "python", "python3", "ruby", "perl", "node",
    ];
    if !valid_interpreters.iter().any(|i| first_line.contains(i)) {
        errors.push(format!(
            "{file}: shebang does not reference a known interpreter: {first_line}"
        ));
    }
    // Check executable permission (unix only).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(file) {
            let mode = meta.permissions().mode();
            if mode & 0o111 == 0 {
                errors.push(format!("{file}: script is not executable (mode {mode:o})"));
            }
        }
    }
}

/// Returns `["--fail-fast"]` when `MINIBOX_FAIL_FAST=true`, otherwise empty.
fn fail_fast_flag() -> Vec<&'static str> {
    if std::env::var("MINIBOX_FAIL_FAST").as_deref() == Ok("true") {
        vec!["--fail-fast"]
    } else {
        vec![]
    }
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

/// Returns true if any `.github/workflows/` files are staged.
fn staged_workflow_files(sh: &Shell) -> Result<bool> {
    let staged = cmd!(sh, "git diff --cached --name-only")
        .output()
        .context("git diff --cached failed")?;
    let staged = String::from_utf8_lossy(&staged.stdout);
    Ok(staged
        .lines()
        .any(|l| l.starts_with(".github/workflows/") || l == ".github/actionlint.yaml"))
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
    if workspace_version_already_staged(sh)? {
        eprintln!("[minibox] workspace version already staged — skipping auto bump");
        return Ok(());
    }
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

fn workspace_version_already_staged(sh: &Shell) -> Result<bool> {
    let head = match cmd!(sh, "git show HEAD:Cargo.toml").output() {
        Ok(output) => output,
        Err(_) => return Ok(false),
    };
    let index = match cmd!(sh, "git show :Cargo.toml").output() {
        Ok(output) => output,
        Err(_) => return Ok(false),
    };

    let head = String::from_utf8_lossy(&head.stdout);
    let index = String::from_utf8_lossy(&index.stdout);
    Ok(parse_workspace_version(&head) != parse_workspace_version(&index))
}

fn parse_workspace_version(content: &str) -> Option<&str> {
    let mut in_workspace_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[workspace.package]" {
            in_workspace_package = true;
            continue;
        }
        if in_workspace_package {
            if trimmed.starts_with('[') {
                break;
            }
            if let Some(v) = trimmed.strip_prefix("version = \"")
                && let Some(v) = v.strip_suffix('"')
            {
                return Some(v);
            }
        }
    }
    None
}

/// Check that every wired adapter has at least one integration test file.
///
/// Mirrors the `adapter-integration-tests` job in `stability-gates.yml`.
pub fn check_adapter_coverage(sh: &Shell) -> Result<()> {
    let adapters = ["native", "gke", "colima"];
    let test_dir = sh.current_dir().join("crates/minibox/tests");
    let mut missing = Vec::new();

    for adapter in &adapters {
        let has_test = fs::read_dir(&test_dir)
            .with_context(|| format!("cannot read {}", test_dir.display()))?
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(adapter));
        if has_test {
            eprintln!("OK: adapter '{adapter}' has integration test file(s)");
        } else {
            eprintln!(
                "ERROR: no integration test file for adapter '{adapter}' in {}",
                test_dir.display()
            );
            missing.push(*adapter);
        }
    }

    if !missing.is_empty() {
        anyhow::bail!(
            "missing integration tests for adapter(s): {}",
            missing.join(", ")
        );
    }
    eprintln!("adapter coverage check passed");
    Ok(())
}

/// Scan production Rust source for `.unwrap()` calls outside test infrastructure.
///
/// Mirrors the `no-unwrap-in-prod` job in `stability-gates.yml`. Advisory by default —
/// prints warnings but does not fail. Pass `strict = true` to fail on any hit.
pub fn check_no_unwrap(sh: &Shell, strict: bool) -> Result<()> {
    let root = sh.current_dir().join("crates");
    let skip_dirs = ["xtask", "testing", "tests", "examples", "benches"];
    let mut hits: Vec<String> = Vec::new();

    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(entry.path());
                continue;
            }
            if !ft.is_file() || entry.path().extension().is_none_or(|ext| ext != "rs") {
                continue;
            }

            let path = entry.path();
            let rel = path.strip_prefix(sh.current_dir()).unwrap_or(&path);
            let rel_str = rel.to_string_lossy();

            // Skip test infrastructure directories.
            if skip_dirs.iter().any(|d| {
                rel_str.contains(&format!("/{d}/")) || rel_str.starts_with(&format!("{d}/"))
            }) {
                continue;
            }
            // Skip adapter mock files.
            if rel_str.contains("adapters/") {
                continue;
            }

            let content =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;

            // Track top-level #[cfg(test)] modules by brace depth so we skip
            // .unwrap() inside ANY test module, not just the last one.
            // Nested #[cfg(test)] (e.g. proptest_tests inside tests) are
            // handled by only entering/exiting at the outermost level.
            let mut in_test_module = false;
            let mut test_brace_depth: i32 = 0;
            let mut saw_cfg_test = false;

            for (i, line) in content.lines().enumerate() {
                let trimmed = line.trim();

                if !in_test_module && trimmed.contains("#[cfg(test)]") {
                    saw_cfg_test = true;
                    continue;
                }

                if saw_cfg_test && !trimmed.is_empty() {
                    if trimmed.starts_with("mod ") || trimmed.starts_with("pub mod ") {
                        in_test_module = true;
                        test_brace_depth = 0;
                    }
                    saw_cfg_test = false;
                }

                if in_test_module {
                    test_brace_depth += trimmed.chars().filter(|&c| c == '{').count() as i32;
                    test_brace_depth -= trimmed.chars().filter(|&c| c == '}').count() as i32;
                    if test_brace_depth <= 0 {
                        in_test_module = false;
                    }
                    continue;
                }

                if line.contains(".unwrap()")
                    && !line.contains("// allow:unwrap")
                    && !trimmed.starts_with("///")
                {
                    hits.push(format!("{}:{}: {}", rel.display(), i + 1, line.trim()));
                }
            }
        }
    }

    if hits.is_empty() {
        eprintln!("OK: no .unwrap() calls in production code");
    } else {
        eprintln!(
            "WARNING: {} .unwrap() call(s) found outside test infrastructure:",
            hits.len()
        );
        for h in &hits {
            eprintln!("  {h}");
        }
        if strict {
            anyhow::bail!("{} .unwrap() calls in production code", hits.len());
        }
    }
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
