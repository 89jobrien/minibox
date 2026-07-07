//! `run-conformance` -- execute all conformance tests and report results.
#![allow(clippy::expect_used)]
//!
//! Exits 0 on success, 1 on any failure.
//!
//! Optional env vars:
//!   `CONFORMANCE_ADAPTER` -- run only the named adapter (e.g. `registry`)
//!   `CONFORMANCE_VERBOSE` -- set to `1` to print every test result, not just failures

use minibox_testsuite::harness::{ReportConfig, ReportGenerator, TestRunner};

fn main() {
    let adapter_filter = std::env::var("CONFORMANCE_ADAPTER").ok();
    let verbose = std::env::var("CONFORMANCE_VERBOSE").is_ok_and(|v| v == "1");

    let runner = TestRunner::collect_inventory();

    if runner.count() == 0 {
        eprintln!(
            "error: conformance runner collected 0 tests -- inventory registration is broken \
             (dropped adapter module or stripped linker section)"
        );
        std::process::exit(1);
    }

    let runner = if let Some(ref name) = adapter_filter {
        runner.filter_adapter(name)
    } else {
        runner
    };

    if runner.filtered_count() == 0 {
        eprintln!(
            "error: conformance runner matched 0 tests after filtering (CONFORMANCE_ADAPTER={})",
            adapter_filter.as_deref().unwrap_or("<unset>")
        );
        std::process::exit(1);
    }

    eprintln!("Running {} conformance tests...", runner.filtered_count());

    let summary = runner.run();

    let cfg = ReportConfig {
        verbose,
        summary_only: false,
        show_timing: true,
    };
    let mut stdout = std::io::stdout();
    ReportGenerator::text(&mut stdout, &summary, &cfg).expect("write report");

    if std::env::var("GITHUB_ACTIONS").is_ok() {
        ReportGenerator::github_actions(&mut stdout, &summary).expect("write GH annotations");
    }

    if !summary.is_success() {
        std::process::exit(1);
    }
}
