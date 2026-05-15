use anyhow::{Context, Result};
use std::{fs, path::Path};
use xshell::{Shell, cmd};

use crate::{bump, docs_lint, utils::cargo_target_dir};

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
    cmd!(
        sh,
        "cargo nextest run --release -p minibox -p minibox-macros -p mbx -p minibox-core --lib"
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
pub fn test_unit(sh: &Shell) -> Result<()> {
    if cfg!(target_os = "macos") {
        cmd!(sh, "cargo nextest run --workspace --lib")
            .run()
            .context("nextest workspace --lib tests failed")?;
    } else {
        cmd!(sh, "cargo nextest run --workspace --exclude macbox --lib")
            .run()
            .context("nextest workspace --lib tests failed")?;
    }
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
    cmd!(sh, "cargo build --release -p minibox-conformance --bins")
        .run()
        .context("failed to build minibox-conformance")?;

    // Run the full suite via `run-conformance` (fast, exits 1 on failure).
    let output = cmd!(
        sh,
        "cargo run --release -p minibox-conformance --bin run-conformance"
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
        "cargo run --release -p minibox-conformance --bin generate-report"
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

/// Coverage-check gate: run llvm-cov on minibox, parse handler.rs function coverage,
/// and exit non-zero when it falls below the 80% threshold.
///
/// Uses `--json --summary-only` which emits a JSON document to stdout containing
/// per-file function coverage summaries. We find the entry for `handler.rs` and
/// read `summary.functions.percent`.
pub fn coverage_check(sh: &Shell) -> Result<()> {
    const THRESHOLD: f64 = 80.0;

    // --json --summary-only emits the coverage JSON to stdout.
    let output = cmd!(
        sh,
        "cargo llvm-cov nextest --package minibox --json --summary-only"
    )
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

/// Parse the function-coverage percentage for `handler.rs` from
/// `cargo llvm-cov nextest --json --summary-only` stdout.
///
/// The JSON schema (llvm.coverage.json.export v3) looks like:
/// ```json
/// {"data":[{"files":[
///   {"filename":"…/daemon/handler.rs",
///    "summary":{"functions":{"count":205,"covered":84,"percent":40.97}}}
/// ]}]}
/// ```
///
/// We walk the JSON text with a simple substring search to avoid a JSON
/// dependency in xtask: find the first `"filename"` field whose value ends
/// with `handler.rs`, then find the `"functions"` summary block immediately
/// after and extract `"percent"`.
fn parse_handler_fn_coverage(output: &str) -> Option<f64> {
    // Find the segment of the JSON that belongs to handler.rs.
    let handler_pos = output.find("handler.rs\"")?;

    // The functions summary appears after the filename, e.g.:
    //   "summary":{"branches":{...},"functions":{"count":205,"covered":84,"percent":40.97},...}
    // Scan forward from handler_pos to find `"functions":`.
    let fns_key = "\"functions\":";
    let fns_pos = output[handler_pos..].find(fns_key)?;
    let after_fns = &output[handler_pos + fns_pos + fns_key.len()..];

    // Extract the percent value from the functions object.
    let pct_key = "\"percent\":";
    let pct_pos = after_fns.find(pct_key)?;
    let after_pct = &after_fns[pct_pos + pct_key.len()..];

    // Read digits (and optional decimal point) up to the next comma or `}`.
    let end = after_pct.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    after_pct[..end].parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::parse_handler_fn_coverage;

    /// The parser must extract the function percent from a realistic JSON snippet.
    #[test]
    fn parse_handler_fn_coverage_extracts_pct_from_json() {
        let sample = r#"{"data":[{"files":[{"filename":"/path/to/daemon/handler.rs","summary":{"branches":{"count":0,"covered":0,"notcovered":0,"percent":0.0},"functions":{"count":205,"covered":84,"percent":40.97560975609756},"lines":{"count":3748,"covered":1532,"percent":40.88}}}]}]}"#;
        let result =
            parse_handler_fn_coverage(sample).expect("should find function percent in JSON output");
        assert!(
            (result - 40.97).abs() < 0.01,
            "expected ~40.97, got {result}"
        );
    }

    /// A JSON snippet with handler.rs at exactly 80% should be recognised.
    #[test]
    fn parse_handler_fn_coverage_recognises_80_percent() {
        let sample = r#"{"data":[{"files":[{"filename":"/path/to/daemon/handler.rs","summary":{"functions":{"count":10,"covered":8,"percent":80.0}}}]}]}"#;
        let result = parse_handler_fn_coverage(sample).expect("should find 80.0 percent");
        assert!((result - 80.0).abs() < 0.001, "expected 80.0, got {result}");
    }

    /// JSON without handler.rs returns None.
    #[test]
    fn parse_handler_fn_coverage_ignores_unrelated_files() {
        let sample = r#"{"data":[{"files":[{"filename":"/path/to/mocks.rs","summary":{"functions":{"count":71,"covered":66,"percent":92.96}}}]}]}"#;
        assert!(parse_handler_fn_coverage(sample).is_none());
    }

    /// Returns None on empty input.
    #[test]
    fn parse_handler_fn_coverage_returns_none_on_empty_input() {
        assert!(parse_handler_fn_coverage("").is_none());
    }

    /// The 80% threshold is the documented contract.
    #[test]
    fn coverage_threshold_is_80_percent() {
        const THRESHOLD: f64 = 80.0;
        let sample = r#"{"data":[{"files":[{"filename":"/path/to/daemon/handler.rs","summary":{"functions":{"count":10,"covered":8,"percent":80.0}}}]}]}"#;
        let pct = parse_handler_fn_coverage(sample).expect("should parse 80.0%");
        assert!(
            pct >= THRESHOLD,
            "80.0% must satisfy the 80% threshold; got {pct}"
        );
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
