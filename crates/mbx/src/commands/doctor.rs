//! `mbx doctor` — built-in runtime health check. No daemon connection required.
//!
//! Renders the report produced by [`minibox_core::doctor`]. All probing lives
//! in `minibox-core`; this module is presentation only, which keeps the check
//! set testable without capturing stdout and guarantees the human and `--json`
//! outputs are describing the same checks.
//!
//! Exit code is non-zero when any check failed, so `mbx doctor` is usable in
//! CI and shell conditionals.

use std::io::Write;

use minibox_core::doctor::{self as engine, Options, Report};

/// Exit code returned when at least one check failed.
const EXIT_FAILURE: i32 = 1;

/// How to render the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Human-readable sections.
    Text,
    /// Machine-readable JSON, one object, newline-terminated.
    Json,
}

/// Run the `doctor` subcommand.
///
/// Returns the process exit code rather than exiting, so the handler stays
/// testable and `main` owns process lifetime.
pub async fn execute(include_tools: bool, format: Format) -> anyhow::Result<i32> {
    let report = engine::run(Options {
        include_tools,
        probe_daemon: true,
    })
    .await;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match format {
        Format::Json => writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?,
        Format::Text => render_text(&report, &mut out)?,
    }
    out.flush()?;

    Ok(if report.has_failures() {
        EXIT_FAILURE
    } else {
        0
    })
}

/// Render the report as aligned text sections with a summary footer.
fn render_text<W: Write>(report: &Report, out: &mut W) -> std::io::Result<()> {
    writeln!(out, "minibox doctor v{}", env!("CARGO_PKG_VERSION"))?;
    writeln!(out, "{}", "=".repeat(40))?;

    for section in &report.sections {
        if section.checks.is_empty() {
            continue;
        }
        writeln!(out)?;
        writeln!(out, "{}:", section.title)?;
        for check in &section.checks {
            writeln!(
                out,
                "  [{}] {:<22} {}",
                check.severity.marker(),
                check.name,
                check.detail
            )?;
        }
    }

    let [pass, info, warn, fail] = report.tally();
    writeln!(out)?;
    if report.has_failures() {
        writeln!(
            out,
            "doctor: {fail} failed, {warn} warning(s), {pass} ok, {info} info"
        )?;
    } else {
        writeln!(
            out,
            "doctor: all checks passed ({pass} ok, {warn} warning(s), {info} info)"
        )?;
        // Only worth suggesting when it was not already run.
        if !report
            .sections
            .iter()
            .any(|s| s.title.contains("toolchain"))
        {
            writeln!(
                out,
                "(build and test toolchain checks: `mbx doctor --tools`)"
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::doctor::{Check, Section};

    fn render(report: &Report) -> String {
        let mut buf: Vec<u8> = Vec::new();
        render_text(report, &mut buf).expect("render");
        String::from_utf8(buf).expect("utf8")
    }

    #[test]
    fn render_includes_version_header() {
        let report = Report {
            sections: vec![Section::new("host").with(Check::pass("platform", "linux x86_64"))],
        };
        let out = render(&report);
        assert!(out.contains("minibox doctor v"), "got: {out}");
        assert!(out.contains("platform"));
    }

    #[test]
    fn render_marks_failures() {
        let report = Report {
            sections: vec![Section::new("daemon").with(Check::fail("socket", "not found"))],
        };
        let out = render(&report);
        assert!(out.contains("FAIL"), "got: {out}");
        assert!(out.contains("1 failed"), "got: {out}");
    }

    #[test]
    fn render_marks_passes_warnings_and_info() {
        let report = Report {
            sections: vec![
                Section::new("mixed")
                    .with(Check::pass("a", "fine"))
                    .with(Check::warn("b", "missing"))
                    .with(Check::info("c", "fyi")),
            ],
        };
        let out = render(&report);
        assert!(out.contains("ok"), "got: {out}");
        assert!(out.contains("warn"), "got: {out}");
        assert!(out.contains("info"), "got: {out}");
    }

    #[test]
    fn render_skips_empty_sections() {
        let report = Report {
            sections: vec![
                Section::new("empty"),
                Section::new("full").with(Check::pass("a", "fine")),
            ],
        };
        let out = render(&report);
        assert!(
            !out.contains("empty:"),
            "empty section should not print: {out}"
        );
        assert!(out.contains("full:"), "got: {out}");
    }

    #[test]
    fn render_success_suggests_tools_flag() {
        let report = Report {
            sections: vec![Section::new("host").with(Check::pass("platform", "linux"))],
        };
        assert!(render(&report).contains("--tools"));
    }

    #[test]
    fn render_failure_does_not_suggest_tools_flag() {
        // The remediation hint is noise when something is already broken.
        let report = Report {
            sections: vec![Section::new("daemon").with(Check::fail("socket", "gone"))],
        };
        assert!(!render(&report).contains("--tools"));
    }

    #[test]
    fn failure_exit_code_is_non_zero() {
        assert_eq!(EXIT_FAILURE, 1);
    }
}
