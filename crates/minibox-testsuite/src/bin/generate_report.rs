//! `generate-report` — run all conformance tests and write JSON + JUnit XML reports.
//!
//! Reports are written to `artifacts/conformance/` (created if absent).
//! Override with `CONFORMANCE_ARTIFACT_DIR`.
//!
//! Prints `conformance:json=<path>` and `conformance:junit=<path>` to stdout so
//! `cargo xtask test-conformance` can surface the paths.
//!
//! Exits 0 on success, 1 on any test failure.

use std::fs;
use std::path::PathBuf;

use minibox_testsuite::adapters;
use minibox_testsuite::harness::{ReportConfig, ReportGenerator, TestRunner};

fn main() {
    let artifact_dir = std::env::var("CONFORMANCE_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("artifacts/conformance"));

    fs::create_dir_all(&artifact_dir).expect("create artifact dir");

    let mut runner = TestRunner::new();
    runner.add_all(adapters::all());

    eprintln!("Running {} conformance tests...", runner.count());
    let summary = runner.run();

    // Text report to stderr for CI visibility.
    let cfg = ReportConfig {
        verbose: true,
        summary_only: false,
        show_timing: true,
    };
    let mut text_out = Vec::new();
    ReportGenerator::text(&mut text_out, &summary, &cfg).expect("text report");
    eprint!("{}", String::from_utf8_lossy(&text_out));

    // JSON report.
    let json_path = artifact_dir.join("conformance.json");
    let mut f = fs::File::create(&json_path).expect("create json report");
    ReportGenerator::json(&mut f, &summary).expect("write json");
    println!("conformance:json={}", json_path.display());

    // JUnit XML report.
    let junit_path = artifact_dir.join("conformance.xml");
    let mut f = fs::File::create(&junit_path).expect("create junit report");
    ReportGenerator::junit_xml(&mut f, &summary).expect("write junit");
    println!("conformance:junit={}", junit_path.display());

    println!(
        "conformance:summary {}/{} passed, {} failed, {} skipped in {}ms",
        summary.passed, summary.total, summary.failed, summary.skipped, summary.duration_ms
    );

    if !summary.is_success() {
        std::process::exit(1);
    }
}
