//! `ReportGenerator` — text, JSON, and JUnit XML output formats.

use std::io::Write;

use super::runner::TestSummary;
use super::traits::TestResult;

/// Configuration for the text report format.
#[derive(Debug, Clone, Default)]
pub struct ReportConfig {
    /// Print every individual test result, not just failures.
    pub verbose: bool,
    /// Omit individual test rows; print only per-adapter totals + grand total.
    pub summary_only: bool,
    /// Include per-test timing.
    pub show_timing: bool,
}

pub struct ReportGenerator;

impl ReportGenerator {
    /// Human-readable text report.
    pub fn text<W: Write>(
        w: &mut W,
        summary: &TestSummary,
        cfg: &ReportConfig,
    ) -> std::io::Result<()> {
        let bar = "═".repeat(65);
        let sep = "─".repeat(65);
        writeln!(w, "{bar}")?;
        writeln!(w, "  MINIBOX CONFORMANCE TEST RESULTS")?;
        writeln!(w, "{bar}")?;
        writeln!(w)?;

        let mut adapter_names: Vec<&str> = summary.by_adapter().keys().copied().collect();
        adapter_names.sort();

        for adapter in adapter_names {
            let entries = &summary.by_adapter()[adapter];
            let pass = entries.iter().filter(|r| r.result.is_pass()).count();
            let fail = entries.iter().filter(|r| r.result.is_fail()).count();
            let skip = entries.iter().filter(|r| r.result.is_skipped()).count();
            let icon = if fail > 0 { '✗' } else { '✓' };

            write!(w, "{icon} {adapter}: {pass} pass, {fail} fail, {skip} skip")?;
            if cfg.show_timing {
                let ms: u64 = entries.iter().map(|r| r.duration_ms).sum();
                write!(w, " ({ms}ms)")?;
            }
            writeln!(w)?;

            if !cfg.summary_only && (cfg.verbose || fail > 0) {
                for r in entries.iter() {
                    let (icon, suffix) = match &r.result {
                        TestResult::Pass => ("  ✓", String::new()),
                        TestResult::Fail { reason } => ("  ✗", format!(" FAIL: {reason}")),
                        TestResult::Skipped { reason } => ("  ○", format!(" (skipped: {reason})")),
                    };
                    if cfg.verbose || r.result.is_fail() {
                        write!(w, "{icon} {}{suffix}", r.name)?;
                        if cfg.show_timing {
                            write!(w, " ({}ms)", r.duration_ms)?;
                        }
                        writeln!(w)?;
                    }
                }
            }
        }

        writeln!(w)?;
        writeln!(w, "{sep}")?;
        write!(
            w,
            "TOTAL: {} pass, {} fail, {} skip ({} tests)",
            summary.passed, summary.failed, summary.skipped, summary.total
        )?;
        if cfg.show_timing {
            write!(w, " in {}ms", summary.duration_ms)?;
        }
        writeln!(w)?;
        writeln!(w)?;
        if summary.is_success() {
            writeln!(w, "RESULT: PASSED")?;
        } else {
            writeln!(w, "RESULT: FAILED")?;
        }
        Ok(())
    }

    /// JSON report for machine consumption.
    pub fn json<W: Write>(w: &mut W, summary: &TestSummary) -> std::io::Result<()> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let report = serde_json::json!({
            "report_version": "1.0",
            "generated_at": now,
            "summary": {
                "total": summary.total,
                "passed": summary.passed,
                "failed": summary.failed,
                "skipped": summary.skipped,
                "duration_ms": summary.duration_ms,
                "success": summary.is_success(),
            },
            "results": summary.results,
        });
        writeln!(w, "{}", serde_json::to_string_pretty(&report).unwrap())?;
        Ok(())
    }

    /// JUnit XML for CI artifact ingestion.
    pub fn junit_xml<W: Write>(w: &mut W, summary: &TestSummary) -> std::io::Result<()> {
        writeln!(w, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        writeln!(
            w,
            r#"<testsuites name="minibox_testsuite" tests="{}" failures="{}" skipped="{}" time="{:.3}">"#,
            summary.total,
            summary.failed,
            summary.skipped,
            summary.duration_ms as f64 / 1000.0
        )?;

        let mut adapter_names: Vec<&str> = summary.by_adapter().keys().copied().collect();
        adapter_names.sort();

        for adapter in adapter_names {
            let entries = &summary.by_adapter()[adapter];
            let failures = entries.iter().filter(|r| r.result.is_fail()).count();
            let skipped = entries.iter().filter(|r| r.result.is_skipped()).count();
            let ms: u64 = entries.iter().map(|r| r.duration_ms).sum();

            writeln!(
                w,
                r#"  <testsuite name="{adapter}" tests="{}" failures="{failures}" skipped="{skipped}" time="{:.3}">"#,
                entries.len(),
                ms as f64 / 1000.0
            )?;

            for r in entries.iter() {
                writeln!(
                    w,
                    r#"    <testcase name="{}" classname="{adapter}" time="{:.3}">"#,
                    xml_escape(&r.name),
                    r.duration_ms as f64 / 1000.0
                )?;
                match &r.result {
                    TestResult::Pass => {}
                    TestResult::Fail { reason } => {
                        writeln!(
                            w,
                            r#"      <failure message="{0}">{0}</failure>"#,
                            xml_escape(reason)
                        )?;
                    }
                    TestResult::Skipped { reason } => {
                        writeln!(w, r#"      <skipped message="{}"/>"#, xml_escape(reason))?;
                    }
                }
                writeln!(w, "    </testcase>")?;
            }
            writeln!(w, "  </testsuite>")?;
        }

        writeln!(w, "</testsuites>")?;
        Ok(())
    }

    /// GitHub Actions annotation format — errors inline in PR diffs.
    pub fn github_actions<W: Write>(w: &mut W, summary: &TestSummary) -> std::io::Result<()> {
        for r in &summary.results {
            if let TestResult::Fail { reason } = &r.result {
                writeln!(
                    w,
                    "::error title=Conformance Failed::{}::{} - {reason}",
                    r.adapter, r.name
                )?;
            }
        }
        if summary.is_success() {
            writeln!(
                w,
                "::notice::Conformance passed: {}/{} tests, {} skipped in {}ms",
                summary.passed, summary.total, summary.skipped, summary.duration_ms
            )?;
        } else {
            writeln!(
                w,
                "::error::Conformance failed: {}/{} passed, {} failed, {} skipped",
                summary.passed, summary.total, summary.failed, summary.skipped
            )?;
        }
        Ok(())
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
