use anyhow::{Context, Result};
use std::{env, fs, path::Path};
use xshell::{Shell, cmd};

/// Pre-commit gate: fmt → clippy --fix → lint check → release build (macOS-safe)
pub fn pre_commit(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo fmt --all").run().context("fmt failed")?;
    cmd!(
        sh,
        "cargo clippy -p linuxbox -p minibox-macros -p mbx -p minibox-client -p minibox-core -p minibox-oci -p daemonbox -p macbox -p miniboxd --fix --allow-dirty --allow-staged"
    )
    .run()
    .context("clippy --fix failed")?;
    cmd!(sh, "cargo fmt --all --check")
        .run()
        .context("fmt-check failed")?;
    cmd!(
        sh,
        "cargo clippy -p linuxbox -p minibox-macros -p mbx -p minibox-client -p minibox-core -p minibox-oci -p daemonbox -p macbox -p miniboxd -- -D warnings"
    )
    .run()
    .context("lint failed")?;
    cmd!(sh,
        "cargo build --release -p linuxbox -p minibox-macros -p mbx -p minibox-client -p minibox-core -p minibox-oci -p daemonbox -p miniboxd"
    ).run().context("build-release failed")?;
    eprintln!("pre-commit checks passed");
    Ok(())
}

/// Pre-push gate: nextest + coverage + ai-review
pub fn prepush(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo nextest run --release -p minibox -p minibox-macros -p mbx -p minibox-client -p minibox-core -p minibox-oci -p daemonbox"
    )
    .run()
    .context("nextest failed")?;
    cmd!(
        sh,
        "cargo llvm-cov nextest -p minibox -p minibox-macros -p mbx -p minibox-client -p minibox-core -p minibox-oci -p daemonbox --html"
    )
    .run()
    .context("coverage failed")?;
    eprintln!("coverage: target/llvm-cov/html/index.html");
    eprintln!("running ai-review...");
    if let Err(e) = cmd!(sh, "uv run scripts/ai-review.py --base main").run() {
        eprintln!("warning: ai-review failed (non-fatal): {e}");
    }
    Ok(())
}

/// All unit + conformance tests (any platform)
pub fn test_unit(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo test --release -p minibox -p minibox-macros -p mbx -p minibox-client -p minibox-core -p minibox-oci -p daemonbox --lib"
    )
    .run()
    .context("lib tests failed")?;
    cmd!(sh, "cargo test --release -p daemonbox --test handler_tests")
        .run()
        .context("handler_tests failed")?;
    cmd!(
        sh,
        "cargo test --release -p daemonbox --test conformance_tests"
    )
    .run()
    .context("conformance_tests failed")?;
    // Colima adapter conformance tests
    cmd!(
        sh,
        "cargo test --release -p minibox --test colima_conformance_tests"
    )
    .run()
    .context("colima_conformance_tests failed")?;
    // GKE adapter isolation tests (platform-agnostic)
    cmd!(
        sh,
        "cargo test --release -p minibox --test gke_adapter_isolation_tests"
    )
    .run()
    .context("gke_adapter_isolation_tests failed")?;
    // Container lifecycle failure tests (daemonbox handler error paths)
    cmd!(
        sh,
        "cargo test --release -p daemonbox --test container_lifecycle_failure_tests"
    )
    .run()
    .context("container_lifecycle_failure_tests failed")?;
    // commit / build / push conformance + artifact report (wires #68)
    test_conformance(sh)?;
    Ok(())
}

/// Conformance suite: commit + build + push backends, then emit MD + JSON reports.
///
/// All three conformance test binaries must pass before the report is emitted.
/// After a successful run this function prints the paths to:
///   `artifacts/conformance/report.md`
///   `artifacts/conformance/report.json`
///
/// Set `CONFORMANCE_PUSH_REGISTRY=localhost:5000` (and run a local OCI registry)
/// to activate tier-2 push tests.
pub fn test_conformance(sh: &Shell) -> Result<()> {
    // --- Commit conformance ---
    cmd!(
        sh,
        "cargo test --release -p minibox --test conformance_commit"
    )
    .run()
    .context("conformance_commit tests failed")?;

    // --- Build conformance ---
    cmd!(
        sh,
        "cargo test --release -p minibox --test conformance_build"
    )
    .run()
    .context("conformance_build tests failed")?;

    // --- Push conformance ---
    cmd!(
        sh,
        "cargo test --release -p minibox --test conformance_push"
    )
    .run()
    .context("conformance_push tests failed")?;

    // --- Emit reports ---
    // Run the report emitter with `--nocapture` so the artifact paths are visible.
    let output = cmd!(
        sh,
        "cargo test --release -p minibox --test conformance_report -- --nocapture"
    )
    .output()
    .context("conformance_report failed")?;

    // Surface conformance: lines from stdout.
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.starts_with("conformance:") {
            if let Some(rest) = line.strip_prefix("conformance:md=") {
                eprintln!("  report.md   : {rest}");
            } else if let Some(rest) = line.strip_prefix("conformance:json=") {
                eprintln!("  report.json : {rest}");
            } else if let Some(rest) = line.strip_prefix("conformance:summary ") {
                eprintln!("  summary     : {rest}");
            }
        }
    }

    if !output.status.success() {
        anyhow::bail!("conformance_report test failed");
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
        "cargo test --release -p daemonbox --test proptest_suite"
    )
    .run()
    .context("daemonbox property tests failed")?;
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

/// Daemon+CLI e2e tests (Linux, root required)
pub fn test_e2e_suite(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo build --release")
        .run()
        .context("build failed")?;

    cmd!(
        sh,
        "cargo test -p miniboxd --test e2e_tests --release --no-run"
    )
    .run()
    .context("failed to build e2e test binary")?;

    let binary = find_test_binary("target/release/deps", "e2e_tests")
        .context("could not locate e2e test binary in target/release/deps")?;

    let bin_dir = env::current_dir()?.join("target/release");
    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {binary} --test-threads=1 --nocapture"
    )
    .run()
    .context("e2e tests failed")?;
    Ok(())
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

/// Coverage-check gate: run llvm-cov on daemonbox, parse handler.rs function coverage,
/// and exit non-zero when it falls below the 80% threshold.
///
/// The function scrapes the per-file summary line emitted by `cargo llvm-cov` in its
/// default text output, which looks like:
///
/// ```text
/// handler.rs          |  80.00 |  ...  |  82.35 |  ...
/// ```
///
/// Column order (0-based): Filename | Line% | Line hits/total | Function% | ...
/// We look for the function-coverage column (index 3) on the `handler.rs` row.
pub fn coverage_check(sh: &Shell) -> Result<()> {
    const THRESHOLD: f64 = 80.0;

    // Run llvm-cov with text output so we can parse per-file function coverage.
    let output = cmd!(sh, "cargo llvm-cov nextest --package daemonbox --text")
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
