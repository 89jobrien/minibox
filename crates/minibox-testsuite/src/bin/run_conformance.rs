//! `run-conformance` — execute all minibox conformance tests and report results.
//!
//! Exits 0 on success, 1 on any failure.
//!
//! Optional env vars:
//!   `CONFORMANCE_ADAPTER`  — run only the named adapter (e.g. `registry`)
//!   `CONFORMANCE_VERBOSE`  — set to `1` to print every test result, not just failures

use minibox_testsuite::adapters;
use minibox_testsuite::harness::{ReportConfig, ReportGenerator, TestRunner};

fn main() {
    let adapter_filter = std::env::var("CONFORMANCE_ADAPTER").ok();
    let verbose = std::env::var("CONFORMANCE_VERBOSE").is_ok_and(|v| v == "1");

    let mut runner = TestRunner::new();
    runner.add_all(adapters::all());

    let runner = if let Some(ref name) = adapter_filter {
        runner.filter_adapter(name)
    } else {
        runner
    };

    eprintln!("Running {} conformance tests...", runner.filtered_count());

    let summary = runner.run();

    let cfg = ReportConfig {
        verbose,
        summary_only: false,
        show_timing: true,
    };
    let mut stdout = std::io::stdout();
    ReportGenerator::text(&mut stdout, &summary, &cfg).expect("write report");

    // Emit GitHub Actions annotations when running in CI.
    if std::env::var("GITHUB_ACTIONS").is_ok() {
        ReportGenerator::github_actions(&mut stdout, &summary).expect("write GH annotations");
    }

    if !summary.is_success() {
        std::process::exit(1);
    }
}
