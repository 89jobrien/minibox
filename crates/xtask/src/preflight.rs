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
        anyhow::bail!("preflight failed — missing tools:\n  {}", failures.join("\n  "))
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
}
