//! Tool, environment, and host-capability checks for `cargo xtask doctor`.

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
#[derive(Debug, PartialEq, Eq)]
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
// Selected-adapter operational prerequisites
// ---------------------------------------------------------------------------

/// Result of checking the adapter that will actually be selected at runtime.
#[derive(Debug, PartialEq, Eq)]
struct AdapterReport {
    adapter: String,
    messages: Vec<String>,
}

/// Probe whether `tool` is on `PATH` via `which`.
fn tool_on_path(tool: &str) -> bool {
    std::process::Command::new("which")
        .arg(tool)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Determine the adapter that will be selected given platform and overrides,
/// and report whether its concrete prerequisite binary is present.
///
/// Mirrors the precedence in `minibox_core::adapter_registry::adapter_from_env`:
/// an explicit `MINIBOX_ADAPTER` is honoured outright (native requires no
/// extra binary — it is built in); otherwise smolvm is preferred when present,
/// falling back to native on Linux or krun on macOS.
fn selected_adapter_report(
    is_linux: bool,
    adapter_env: Option<&str>,
    smolvm_present: bool,
    krun_present: bool,
) -> AdapterReport {
    let fallback = if is_linux { "native" } else { "krun" };

    let adapter = match adapter_env {
        Some(explicit) => explicit.to_string(),
        None if smolvm_present => "smolvm".to_string(),
        None => fallback.to_string(),
    };

    let messages = match adapter.as_str() {
        "smolvm" if smolvm_present => {
            vec!["[ok]   smolvm on PATH — selected adapter available".to_string()]
        }
        "smolvm" => {
            vec!["[fail] smolvm selected (MINIBOX_ADAPTER=smolvm) but not on PATH".to_string()]
        }
        "krun" if krun_present => {
            vec!["[ok]   krun on PATH — selected adapter available".to_string()]
        }
        "krun" => vec!["[fail] krun selected but not on PATH".to_string()],
        "native" if is_linux => {
            vec!["[info] native adapter selected — see Linux capability checks below".to_string()]
        }
        "native" => {
            vec!["[fail] native adapter selected but is Linux-only on this platform".to_string()]
        }
        other => vec![format!(
            "[info] {other} adapter selected — no local binary prerequisite tracked by doctor"
        )],
    };

    AdapterReport { adapter, messages }
}

// ---------------------------------------------------------------------------
// macOS: Virtualization.framework signing (`vz` adapter)
// ---------------------------------------------------------------------------

/// Entitlement key Virtualization.framework requires to create a virtual machine.
#[cfg(target_os = "macos")]
const VZ_ENTITLEMENT: &str = "com.apple.security.virtualization";

/// Whether `codesign -d --entitlements :-` output grants the virtualization entitlement.
///
/// The value is read from the window between the key and the next key so a `<true/>`
/// belonging to a later entitlement cannot be mistaken for this one.
#[cfg(target_os = "macos")]
fn codesign_output_grants_virtualization(stdout: &str) -> bool {
    let Some(idx) = stdout.find(VZ_ENTITLEMENT) else {
        return false;
    };
    let rest = &stdout[idx + VZ_ENTITLEMENT.len()..];
    let window = rest.split("<key>").next().unwrap_or(rest);
    window.contains("<true/>")
}

/// Report whether local `miniboxd` builds can start a VM under the `vz` adapter.
///
/// macOS validates the entitlement before any configuration is accepted and rejects an
/// unsigned process with `VZErrorDomain code=2` (`invalidVirtualMachineConfiguration`),
/// which no daemon-level setting can work around. Ad-hoc signing is enough (verified on
/// macOS 26.4), so the remedy is always `just sign-vz` plus a restart. When `vz` is not the
/// selected adapter the finding is advisory: a missing entitlement is not a failure for
/// users running smolvm/krun.
#[cfg(target_os = "macos")]
fn check_vz_signing(failures: &mut Vec<String>, required: bool) {
    let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let mut checked = 0usize;

    for profile in ["debug", "release"] {
        let bin = Path::new(&target_dir).join(profile).join("miniboxd");
        if !bin.is_file() {
            continue;
        }
        checked += 1;
        // A binary with no signature at all makes `codesign -d` exit non-zero, so an
        // unreadable signature is reported as "not entitled" rather than a warning: it is
        // the same runtime outcome, and `just sign-vz` fixes both cases.
        let verdict: Result<(), String> = match std::process::Command::new("codesign")
            .args(["-d", "--entitlements", ":-"])
            .arg(&bin)
            .output()
        {
            Ok(out) if out.status.success() => {
                let plist = String::from_utf8_lossy(&out.stdout);
                if codesign_output_grants_virtualization(&plist) {
                    Ok(())
                } else {
                    Err(format!("is not signed with {VZ_ENTITLEMENT}"))
                }
            }
            Ok(out) => {
                let reason = String::from_utf8_lossy(&out.stderr);
                Err(format!(
                    "has no readable code signature ({})",
                    reason.trim()
                ))
            }
            Err(e) => Err(format!("could not be inspected: {e}")),
        };

        match verdict {
            Ok(()) => println!("[ok]   {} carries {VZ_ENTITLEMENT}", bin.display()),
            Err(reason) if required => {
                println!("[fail] {} {reason} — run `just sign-vz`", bin.display());
                failures.push(format!("{} {reason}", bin.display()));
            }
            Err(reason) => println!(
                "[info] {} {reason} — `vz` would fail; run `just sign-vz` when needed",
                bin.display()
            ),
        }
    }

    if checked == 0 {
        println!(
            "[info] no local miniboxd binary to check — build it, then run `just sign-vz` for `vz`"
        );
    }
}

// ---------------------------------------------------------------------------
// Doctor: full preflight checks including env vars and system capabilities
// ---------------------------------------------------------------------------

/// Run the full doctor suite and print results.
///
/// Checks all tools from `scripts/preflight.nu`, `CARGO_TARGET_DIR` env var,
/// 1Password auth status, smolvm availability, git working-tree state, and on
/// Linux, cgroups v2 + overlay filesystem availability. Returns `Ok(())` if all
/// required checks pass (advisory warns are non-fatal), or an error listing
/// failures.
///
/// This is the canonical single-source-of-truth for environment validation.
/// `scripts/preflight.nu` remains a lightweight `SessionStart` hook that runs at
/// shell startup; running `cargo xtask` at startup would add build latency, so
/// the script stays independent and simply references this command in its banner.
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

    // --- 1Password auth (advisory) ---
    // Mirrors `check "1Password authed" (op account list)` from preflight.nu.
    // Non-fatal: missing op auth blocks secret resolution but not building/testing.
    {
        let authed = std::process::Command::new("op")
            .arg("account")
            .arg("list")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if authed {
            println!("[ok]   1Password authed");
        } else {
            println!(
                "[warn] 1Password not authed — `op account list` failed (secret resolution may fail)"
            );
        }
    }

    // --- selected-adapter operational prerequisites ---
    // Determine which adapter will actually be selected at runtime
    // (MINIBOX_ADAPTER override, else platform default) and report whether its
    // concrete prerequisite is satisfied, rather than always reporting on smolvm
    // regardless of what will actually run.
    {
        let smolvm_present = tool_on_path("smolvm");
        let krun_present = tool_on_path("krun");
        let adapter_env = std::env::var("MINIBOX_ADAPTER").ok();
        let report = selected_adapter_report(
            cfg!(target_os = "linux"),
            adapter_env.as_deref(),
            smolvm_present,
            krun_present,
        );
        println!("[info] selected adapter: {}", report.adapter);
        for line in &report.messages {
            println!("{line}");
            if line.starts_with("[fail]") {
                failures.push(line.trim_start_matches("[fail] ").to_string());
            }
        }
        #[cfg(target_os = "macos")]
        check_vz_signing(&mut failures, report.adapter == "vz");
    }

    // --- git working-tree state (info, mirrors preflight.nu) ---
    // Dirty tree is expected during development; surfaced as info, not a failure.
    {
        let output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .output();
        match output {
            Ok(out) if out.stdout.is_empty() => println!("[info] git repo clean"),
            Ok(_) => {
                println!(
                    "[info] git repo has local changes (startup doctor ignores working tree dirtiness)"
                );
            }
            Err(e) => println!("[warn] could not run `git status`: {e}"),
        }
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
    use std::path::Path;
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
        if parts.len() >= 2 && parts[0] >= 5 {
            println!("[ok]   kernel version {version_str} (>= 5.0)");
        } else {
            println!(
                "[warn] kernel version {version_str} — 5.0+ recommended for cgroups v2 support"
            );
        }
    } else {
        println!("[warn] could not read /proc/version");
    }

    // privileges: the native adapter needs UID 0 for namespace creation,
    // overlay mounts, and cgroup v2 management (see
    // `adapter_registry::warn_if_native_without_root`). Advisory here since
    // doctor also runs for non-native adapters that don't need root.
    match std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| parse_effective_uid(&s))
    {
        Some(0) => println!("[ok]   running as root (UID 0) — native adapter operations permitted"),
        Some(uid) => println!(
            "[warn] running as UID {uid} — native adapter requires root for namespaces, overlay mounts, and cgroup v2 management"
        ),
        None => println!("[warn] could not determine effective UID from /proc/self/status"),
    }
}

/// Parse the effective UID from the `Uid:` line of `/proc/self/status` content.
///
/// The line has the form `Uid:\t<real>\t<effective>\t<saved>\t<filesystem>`.
/// Only called from the Linux-only privilege check, but kept test-visible
/// cross-platform so the parsing logic is covered without a Linux-only run.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_effective_uid(status_contents: &str) -> Option<u32> {
    status_contents
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().nth(1))
        .and_then(|effective| effective.parse().ok())
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
    // Selected-adapter prerequisite tests (pure, no IO)
    // ---------------------------------------------------------------------------

    #[test]
    fn selected_adapter_defaults_to_smolvm_when_present() {
        let report = selected_adapter_report(true, None, true, false);
        assert_eq!(report.adapter, "smolvm");
        assert!(report.messages.iter().any(|m| m.starts_with("[ok]")));
    }

    #[test]
    fn selected_adapter_falls_back_to_native_on_linux_without_smolvm() {
        let report = selected_adapter_report(true, None, false, false);
        assert_eq!(report.adapter, "native");
    }

    #[test]
    fn selected_adapter_falls_back_to_krun_on_macos_without_smolvm() {
        let report = selected_adapter_report(false, None, false, false);
        assert_eq!(report.adapter, "krun");
    }

    #[test]
    fn selected_adapter_explicit_smolvm_fails_when_absent() {
        let report = selected_adapter_report(true, Some("smolvm"), false, false);
        assert_eq!(report.adapter, "smolvm");
        assert!(report.messages.iter().any(|m| m.starts_with("[fail]")));
    }

    #[test]
    fn selected_adapter_explicit_krun_fails_when_absent() {
        let report = selected_adapter_report(false, Some("krun"), false, false);
        assert!(report.messages.iter().any(|m| m.starts_with("[fail]")));
    }

    #[test]
    fn selected_adapter_native_on_non_linux_fails() {
        let report = selected_adapter_report(false, Some("native"), false, false);
        assert!(report.messages.iter().any(|m| m.starts_with("[fail]")));
    }

    #[test]
    fn selected_adapter_unknown_name_reports_info_only() {
        let report = selected_adapter_report(true, Some("colima"), false, false);
        assert_eq!(report.adapter, "colima");
        assert!(report.messages.iter().any(|m| m.starts_with("[info]")));
    }

    #[test]
    fn parse_effective_uid_reads_root() {
        let status = "Name:\tcargo\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\n";
        assert_eq!(parse_effective_uid(status), Some(0));
    }

    #[test]
    fn parse_effective_uid_reads_non_root() {
        let status = "Name:\tcargo\nUid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_effective_uid(status), Some(1000));
    }

    #[test]
    fn parse_effective_uid_differs_from_real_uid() {
        // setuid/sudo-with-cap-drop style line: real UID 1000, effective UID 0.
        // Pins the field index — a regression to .next() (real UID) would
        // wrongly return 1000 here instead of 0.
        let status = "Name:\tcargo\nUid:\t1000\t0\t0\t1000\n";
        assert_eq!(parse_effective_uid(status), Some(0));
    }

    #[test]
    fn parse_effective_uid_missing_line_returns_none() {
        assert_eq!(parse_effective_uid("Name:\tcargo\n"), None);
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

    #[cfg(target_os = "macos")]
    mod vz_signing {
        use super::super::codesign_output_grants_virtualization;

        const ENTITLED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>com.apple.security.virtualization</key><true/></dict></plist>"#;

        #[test]
        fn grants_when_key_is_true() {
            assert!(codesign_output_grants_virtualization(ENTITLED));
        }

        #[test]
        fn denies_when_key_is_false() {
            let plist = r#"<plist version="1.0"><dict><key>com.apple.security.virtualization</key><false/></dict></plist>"#;
            assert!(!codesign_output_grants_virtualization(plist));
        }

        #[test]
        fn denies_when_key_absent() {
            let plist = r#"<plist version="1.0"><dict><key>com.apple.security.cs.allow-jit</key><true/></dict></plist>"#;
            assert!(!codesign_output_grants_virtualization(plist));
        }

        #[test]
        fn denies_empty_codesign_output() {
            assert!(!codesign_output_grants_virtualization(""));
        }

        #[test]
        fn ignores_true_belonging_to_a_later_key() {
            let plist = r#"<plist version="1.0"><dict><key>com.apple.security.virtualization</key><false/><key>com.apple.security.cs.allow-jit</key><true/></dict></plist>"#;
            assert!(!codesign_output_grants_virtualization(plist));
        }
    }
}
