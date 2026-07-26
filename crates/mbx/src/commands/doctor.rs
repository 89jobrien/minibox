//! `mbx doctor` — show adapter suite diagnostics without connecting to the daemon.
//!
//! Displays which adapter suites are compiled into this build, which would be
//! selected by the current environment, and basic host capability information.

/// Metadata about a single adapter suite.
///
/// Mirrors `miniboxd::adapter_registry::AdapterInfo` but is duplicated here
/// so that `mbx` does not need a dependency on `miniboxd`.
#[derive(Debug)]
struct AdapterEntry {
    name: &'static str,
    description: &'static str,
    available: bool,
}

/// Return adapter entries for the current build platform.
fn adapter_entries() -> Vec<AdapterEntry> {
    vec![
        AdapterEntry {
            name: "native",
            description: "Linux namespaces, overlay FS, cgroups v2 (requires root)",
            available: cfg!(target_os = "linux"),
        },
        AdapterEntry {
            name: "gke",
            description: "proot (ptrace), copy FS, no-op limiter (unprivileged GKE)",
            available: cfg!(target_os = "linux"),
        },
        AdapterEntry {
            name: "smolvm",
            description: "SmolVM lightweight Linux VMs with subsecond boot",
            available: cfg!(unix),
        },
        AdapterEntry {
            name: "krun",
            description: "libkrun micro-VM (KVM on Linux, HVF on macOS)",
            available: true,
        },
    ]
}

/// Return adapter names available in the current build.
pub fn compiled_adapters() -> Vec<&'static str> {
    adapter_entries()
        .into_iter()
        .filter(|a| a.available)
        .map(|a| a.name)
        .collect()
}

/// Determine which adapter would be selected given the current environment.
///
/// Mirrors the logic in `miniboxd::adapter_registry::adapter_from_env` without
/// the binary probe — the probe requires running `smolvm --version` which is
/// a side effect we avoid in a diagnostic command.
pub fn selected_adapter() -> String {
    match std::env::var("MINIBOX_ADAPTER") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            // Mirror the default: smolvm with krun fallback
            // (We can't probe for smolvm here without running it, so we
            // report the configured default and note the fallback.)
            "smolvm (or krun if smolvm binary absent)".to_string()
        }
    }
}

/// Result of checking a single CNI plugin binary's presence.
pub struct CniPluginStatus {
    /// Plugin binary name (e.g. `"bridge"`).
    pub plugin: &'static str,
    /// Whether the binary was found on `MINIBOX_CNI_PATH`.
    pub found: bool,
}

/// Check whether the standard CNI plugin binaries minibox's native adapter
/// needs (when built with the `cni` feature) are present on
/// `MINIBOX_CNI_PATH` (defaulting to `/opt/cni/bin`).
///
/// Advisory only — does not invoke the binaries, just checks presence,
/// matching this module's existing checks' style.
pub fn cni_plugin_status() -> Vec<CniPluginStatus> {
    let cni_path = std::env::var("MINIBOX_CNI_PATH").unwrap_or_else(|_| "/opt/cni/bin".to_string());
    let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&cni_path).collect();
    ["bridge", "host-local", "portmap", "dnsname"]
        .into_iter()
        .map(|plugin| CniPluginStatus {
            plugin,
            found: dirs.iter().any(|dir| dir.join(plugin).is_file()),
        })
        .collect()
}

/// Run `cargo xtask doctor` and stream its output to stdout.
///
/// This surfaces tool / environment preflight results before the adapter
/// section so that `mbx doctor` is the single entry point for both concerns.
/// Failure is advisory — a non-zero exit from xtask is printed but does not
/// prevent the adapter section from running.
fn run_xtask_doctor() {
    println!("=== environment preflight (cargo xtask doctor) ===");
    println!("(canonical command: `cargo xtask doctor`)");
    println!();
    let result = std::process::Command::new("cargo")
        .args(["xtask", "doctor"])
        .status();
    match result {
        Ok(status) if !status.success() => {
            println!();
            println!(
                "[warn] `cargo xtask doctor` exited with {} — see output above",
                status.code().unwrap_or(-1)
            );
        }
        Err(e) => {
            println!("[warn] could not run `cargo xtask doctor`: {e}");
            println!("       Install cargo and build the workspace to enable full preflight.");
        }
        Ok(_) => {}
    }
    println!();
}

/// Run the `doctor` subcommand.
pub fn execute() -> anyhow::Result<()> {
    run_xtask_doctor();

    println!("minibox adapter diagnostics");
    println!("{}", "=".repeat(40));
    println!();

    let compiled = compiled_adapters();
    let entries = adapter_entries();
    let available: Vec<_> = entries.iter().filter(|a| a.available).collect();
    let unavailable: Vec<_> = entries.iter().filter(|a| !a.available).collect();

    println!("compiled adapters ({}):", compiled.len());
    for a in &available {
        println!("  [x] {} — {}", a.name, a.description);
    }

    if !unavailable.is_empty() {
        println!();
        println!("known but unavailable in this build:");
        for a in &unavailable {
            println!("  [ ] {} — {}", a.name, a.description);
        }
    }

    println!();
    println!("selected adapter:  {}", selected_adapter());
    println!("(override with:    MINIBOX_ADAPTER=<name> miniboxd)");

    println!();
    println!("CNI plugins (opt-in native bridge networking, cni feature):");
    for status in cni_plugin_status() {
        let marker = if status.found { "[x]" } else { "[ ]" };
        println!("  {marker} {}", status.plugin);
    }
    println!("(configure with:   MINIBOX_CNI_PATH, defaults to /opt/cni/bin)");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_adapters_is_non_empty() {
        assert!(
            !compiled_adapters().is_empty(),
            "compiled_adapters() must return at least one adapter"
        );
    }

    #[test]
    fn compiled_adapters_includes_krun() {
        // krun is always available (available: true unconditionally)
        assert!(
            compiled_adapters().contains(&"krun"),
            "krun must always be in compiled_adapters"
        );
    }

    #[test]
    fn execute_returns_ok() {
        let result = execute();
        assert!(result.is_ok(), "doctor execute should not fail: {result:?}");
    }

    #[test]
    fn selected_adapter_respects_env_var() {
        // SAFETY: serialized by process-level isolation in unit tests
        unsafe {
            std::env::set_var("MINIBOX_ADAPTER", "krun");
        }
        let adapter = selected_adapter();
        // SAFETY: same unique env var set above; remove_var restores the absent state.
        unsafe {
            std::env::remove_var("MINIBOX_ADAPTER");
        }
        assert_eq!(adapter, "krun");
    }

    #[test]
    fn cni_plugin_status_reports_missing_binaries_when_path_unset() {
        // SAFETY: serialized by process-level isolation in unit tests;
        // MINIBOX_CNI_PATH is unset in this test process by default.
        let statuses = cni_plugin_status();
        assert!(statuses.iter().any(|s| !s.found));
    }

    #[test]
    fn cni_plugin_status_reports_found_when_binaries_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["bridge", "host-local", "portmap", "dnsname"] {
            std::fs::write(dir.path().join(name), "#!/bin/sh\n").expect("write fixture");
        }
        // SAFETY: serialized by process-level isolation in unit tests.
        unsafe {
            std::env::set_var("MINIBOX_CNI_PATH", dir.path());
        }
        let statuses = cni_plugin_status();
        // SAFETY: same unique env var set above; remove_var restores the absent state.
        unsafe {
            std::env::remove_var("MINIBOX_CNI_PATH");
        }
        assert!(statuses.iter().all(|s| s.found));
    }
}
