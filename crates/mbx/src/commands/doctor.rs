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
            name: "colima",
            description: "Colima/Lima VM via limactl + nerdctl",
            available: cfg!(unix),
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

/// Run the `doctor` subcommand.
pub fn execute() -> anyhow::Result<()> {
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
            std::env::set_var("MINIBOX_ADAPTER", "colima");
        }
        let adapter = selected_adapter();
        unsafe {
            std::env::remove_var("MINIBOX_ADAPTER");
        }
        assert_eq!(adapter, "colima");
    }
}
