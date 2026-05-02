use std::path::Path;

/// Domain port: probe whether a named tool is functional.
///
/// Implementations call the real process; test doubles return canned results.
pub trait ToolProbe {
    /// Probe `tool` by running `tool --version`.
    /// Returns `Ok(())` if the tool exists and exits with code 0.
    /// Returns `Err(String)` with a human-readable message otherwise.
    fn probe(&self, tool: &str) -> Result<(), String>;
}

/// Domain: result of a preflight check for a single tool.
#[derive(Debug, PartialEq)]
pub enum ProbeResult {
    Ok,
    Missing(String),
}

/// Domain service: run preflight checks for all required tools.
///
/// Returns a list of `ProbeResult` — one per tool — in the same order as `tools`.
/// The caller decides what to do with failures.
pub fn check_tools<P: ToolProbe>(probe: &P, tools: &[&str]) -> Vec<(String, ProbeResult)> {
    tools
        .iter()
        .map(|&tool| {
            let result = match probe.probe(tool) {
                Result::Ok(()) => ProbeResult::Ok,
                Result::Err(msg) => ProbeResult::Missing(msg),
            };
            (tool.to_string(), result)
        })
        .collect()
}

/// Domain service: fail fast if any required tool is missing.
///
/// Returns `Ok(())` when all tools pass, or an error listing missing tools.
pub fn require_tools<P: ToolProbe>(probe: &P, tools: &[&str]) -> anyhow::Result<()> {
    let results = check_tools(probe, tools);
    let failures: Vec<_> = results
        .iter()
        .filter(|(_, r)| matches!(r, ProbeResult::Missing(_)))
        .map(|(t, r)| {
            if let ProbeResult::Missing(msg) = r {
                format!("{t}: {msg}")
            } else {
                unreachable!()
            }
        })
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "preflight failed — missing tools:\n  {}",
            failures.join("\n  ")
        )
    }
}

// ---------------------------------------------------------------------------
// Real adapter (process-based)
// ---------------------------------------------------------------------------

/// Adapter: probe tools by invoking `<tool> --version` as a subprocess.
pub struct ProcessProbe;

impl ToolProbe for ProcessProbe {
    fn probe(&self, tool: &str) -> Result<(), String> {
        let status = std::process::Command::new(tool)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Err(e) => Err(format!("could not execute `{tool} --version`: {e}")),
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!(
                "`{tool} --version` exited with {}",
                s.code().unwrap_or(-1)
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Xtask availability port + domain service
// ---------------------------------------------------------------------------

/// Domain port: probe whether `cargo xtask` itself is runnable.
///
/// Implementations invoke the real xtask binary; test doubles return canned results.
pub trait XtaskProbe {
    /// Probe xtask by running it with no arguments (which prints help and exits 0).
    /// Returns `Ok(())` if the binary responds successfully.
    /// Returns `Err(String)` with a human-readable message otherwise.
    fn probe_xtask(&self) -> Result<(), String>;
}

/// Domain service: check that `cargo xtask` is actually runnable.
///
/// This validates real capability (the binary compiles and responds), not just
/// structural signals like `Cargo.toml` existence.
pub fn check_xtask_available<P: XtaskProbe>(probe: &P) -> anyhow::Result<()> {
    probe
        .probe_xtask()
        .map_err(|msg| anyhow::anyhow!("xtask is not available: {msg}"))
}

// ---------------------------------------------------------------------------
// Real adapter (process-based)
// ---------------------------------------------------------------------------

/// Adapter: probe xtask availability by invoking `cargo xtask` with no args.
///
/// `cargo xtask` with no arguments prints the task list and exits 0, which
/// proves the binary compiles and is runnable without performing any side effects.
pub struct ProcessXtaskProbe;

impl XtaskProbe for ProcessXtaskProbe {
    fn probe_xtask(&self) -> Result<(), String> {
        let status = std::process::Command::new("cargo")
            .args(["xtask"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Err(e) => Err(format!("could not execute `cargo xtask`: {e}")),
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!(
                "`cargo xtask` exited with {}",
                s.code().unwrap_or(-1)
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Doctor: full preflight checks including env vars and system capabilities
// ---------------------------------------------------------------------------

/// Result of a single doctor check.
#[derive(Debug, PartialEq)]
pub enum CheckResult {
    Ok,
    Warn(String),
    Fail(String),
}

/// Run the full doctor suite and print results.
///
/// Checks tools from `preflight.nu`, `CARGO_TARGET_DIR` env var, and on Linux,
/// cgroups v2 + overlay filesystem availability. Returns `Ok(())` if all
/// checks pass (warns are non-fatal), or an error listing failures.
pub fn doctor<P: ToolProbe>(probe: &P) -> anyhow::Result<()> {
    let mut failures: Vec<String> = Vec::new();

    // --- Tool checks (from preflight.nu) ---
    let tools = &["cargo", "just", "rustup", "cargo-nextest", "gh", "op"];
    let tool_results = check_tools(probe, tools);
    for (tool, result) in &tool_results {
        match result {
            ProbeResult::Ok => println!("[ok]   {tool} on PATH"),
            ProbeResult::Missing(msg) => {
                // op and gh are advisory — warn, don't fail
                if tool == "op" || tool == "gh" {
                    println!("[warn] {tool} on PATH — {msg}");
                } else {
                    println!("[fail] {tool} on PATH — {msg}");
                    failures.push(format!("{tool}: {msg}"));
                }
            }
        }
    }

    // --- CARGO_TARGET_DIR ---
    if std::env::var("CARGO_TARGET_DIR").is_ok() {
        println!("[ok]   CARGO_TARGET_DIR set");
    } else {
        println!("[warn] CARGO_TARGET_DIR not set (optional but recommended)");
    }

    // --- Linux-only system checks ---
    #[cfg(target_os = "linux")]
    {
        check_linux_capabilities(&mut failures);
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("[info] Linux-only checks (cgroups v2, overlay) skipped on this platform");
    }

    if failures.is_empty() {
        println!("\ndoctor: all checks passed");
        Ok(())
    } else {
        anyhow::bail!(
            "doctor: {} check(s) failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        )
    }
}

/// Check Linux-specific system capabilities required by the native adapter.
#[cfg(target_os = "linux")]
fn check_linux_capabilities(failures: &mut Vec<String>) {
    // cgroups v2: /sys/fs/cgroup must be mounted as cgroup2
    let cgroup2_ok = Path::new("/sys/fs/cgroup/cgroup.controllers").exists();
    if cgroup2_ok {
        println!("[ok]   cgroups v2 unified hierarchy");
    } else {
        println!("[fail] cgroups v2 not found — /sys/fs/cgroup/cgroup.controllers missing");
        failures.push("cgroups v2: /sys/fs/cgroup/cgroup.controllers not found".to_string());
    }

    // overlay filesystem: check via /proc/filesystems
    let overlay_ok = std::fs::read_to_string("/proc/filesystems")
        .map(|s| s.contains("overlay"))
        .unwrap_or(false);
    if overlay_ok {
        println!("[ok]   overlay filesystem available");
    } else {
        println!(
            "[warn] overlay filesystem not listed in /proc/filesystems (may require modprobe overlay)"
        );
    }

    // kernel version: parse /proc/version for major.minor
    if let Ok(kver) = std::fs::read_to_string("/proc/version") {
        let version_str = kver.split_whitespace().nth(2).unwrap_or("unknown");
        // Require kernel 5.0+ for best cgroups v2 support
        let parts: Vec<u32> = version_str
            .split('.')
            .take(2)
            .filter_map(|p| p.parse().ok())
            .collect();
        if parts.len() >= 2 && (parts[0] > 5 || (parts[0] == 5 && parts[1] >= 0)) {
            println!("[ok]   kernel version {version_str} (>= 5.0)");
        } else {
            println!(
                "[warn] kernel version {version_str} — 5.0+ recommended for cgroups v2 support"
            );
        }
    } else {
        println!("[warn] could not read /proc/version");
    }
}

// ---------------------------------------------------------------------------
// Tests (in-memory test double — no real processes)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Test double: returns preset results without spawning processes.
    struct StubProbe {
        results: HashMap<String, Result<(), String>>,
    }

    impl StubProbe {
        fn new() -> Self {
            Self {
                results: HashMap::new(),
            }
        }

        fn with_tool(mut self, tool: &str, result: Result<(), String>) -> Self {
            self.results.insert(tool.to_string(), result);
            self
        }
    }

    impl ToolProbe for StubProbe {
        fn probe(&self, tool: &str) -> Result<(), String> {
            self.results
                .get(tool)
                .cloned()
                .unwrap_or_else(|| Err(format!("tool `{tool}` not registered in stub")))
        }
    }

    #[test]
    fn all_tools_present_returns_ok() {
        let probe = StubProbe::new()
            .with_tool("cargo", Ok(()))
            .with_tool("cargo-nextest", Ok(()));

        let result = require_tools(&probe, &["cargo", "cargo-nextest"]);
        assert!(result.is_ok());
    }

    #[test]
    fn missing_tool_returns_error() {
        let probe = StubProbe::new().with_tool("cargo", Ok(()));

        let result = require_tools(&probe, &["cargo", "gh"]);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("gh"), "error should mention the missing tool");
    }

    #[test]
    fn all_missing_lists_all_failures() {
        let probe = StubProbe::new()
            .with_tool("cargo", Err("not found".into()))
            .with_tool("gh", Err("not found".into()));

        let result = require_tools(&probe, &["cargo", "gh"]);
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("cargo"));
        assert!(msg.contains("gh"));
    }

    #[test]
    fn check_tools_returns_per_tool_results() {
        let probe = StubProbe::new()
            .with_tool("cargo", Ok(()))
            .with_tool("gh", Err("command not found".into()));

        let results = check_tools(&probe, &["cargo", "gh"]);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], ("cargo".to_string(), ProbeResult::Ok));
        assert!(matches!(&results[1].1, ProbeResult::Missing(_)));
    }

    #[test]
    fn process_probe_succeeds_for_cargo() {
        // This test calls the real `cargo --version` — it requires cargo on PATH.
        // It validates the real adapter wiring, not just the domain logic.
        let probe = ProcessProbe;
        assert!(
            probe.probe("cargo").is_ok(),
            "cargo must be on PATH for xtask to work"
        );
    }

    #[test]
    fn process_probe_fails_for_nonexistent_tool() {
        let probe = ProcessProbe;
        let result = probe.probe("__xtask_nonexistent_binary_xyz__");
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------------------
    // XtaskProbe tests (in-memory doubles)
    // ---------------------------------------------------------------------------

    struct StubXtaskProbe {
        result: Result<(), String>,
    }

    impl StubXtaskProbe {
        fn available() -> Self {
            Self { result: Ok(()) }
        }

        fn unavailable(reason: &str) -> Self {
            Self {
                result: Err(reason.to_string()),
            }
        }
    }

    impl XtaskProbe for StubXtaskProbe {
        fn probe_xtask(&self) -> Result<(), String> {
            self.result.clone()
        }
    }

    #[test]
    fn check_xtask_available_returns_ok_when_probe_succeeds() {
        let probe = StubXtaskProbe::available();
        assert!(check_xtask_available(&probe).is_ok());
    }

    #[test]
    fn check_xtask_available_returns_err_when_probe_fails() {
        let probe = StubXtaskProbe::unavailable("binary not found");
        let result = check_xtask_available(&probe);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("xtask is not available"),
            "error should mention xtask is not available; got: {msg}"
        );
        assert!(
            msg.contains("binary not found"),
            "error should include the probe reason; got: {msg}"
        );
    }

    #[test]
    fn process_xtask_probe_succeeds_in_workspace() {
        // This calls the real `cargo xtask` — it requires the workspace to be intact.
        // It validates the real adapter proves xtask is runnable, not just that
        // Cargo.toml exists.
        let probe = ProcessXtaskProbe;
        assert!(
            probe.probe_xtask().is_ok(),
            "cargo xtask must be runnable for this workspace"
        );
    }
}
