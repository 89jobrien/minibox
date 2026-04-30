//! Adapter registry — centralizes adapter suite discovery and validation.
//!
//! This module provides [`AdapterInfo`] descriptors and functions to enumerate
//! available adapter suites at compile time, validate user-provided adapter
//! names, and produce structured errors listing valid options.

use std::fmt;

/// Metadata about a single adapter suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    /// The string value accepted by `MINIBOX_ADAPTER`.
    pub name: &'static str,
    /// Human-readable one-line description.
    pub description: &'static str,
    /// Whether this adapter is available in the current build.
    pub available: bool,
    /// The platform this adapter targets (e.g. "linux", "macos").
    pub platform: &'static str,
}

/// Which set of adapters to use for container operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterSuite {
    /// Linux-native: namespaces, overlay FS, cgroups v2. Requires root.
    Native,
    /// GKE unprivileged: proot, copy FS, no-op limiter. No root needed.
    Gke,
    /// macOS via Colima/Lima: delegates to limactl, nerdctl, chroot in VM.
    Colima,
    /// macOS via SmolVM: lightweight Linux VMs with subsecond boot.
    SmolVm,
    /// krun: libkrun-based micro-VM (Linux via KVM, macOS via HVF).
    Krun,
}

impl fmt::Display for AdapterSuite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AdapterSuite {
    /// The string identifier for this suite (matches `MINIBOX_ADAPTER` values).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Gke => "gke",
            Self::Colima => "colima",
            Self::SmolVm => "smolvm",
            Self::Krun => "krun",
        }
    }
}

/// Default adapter suite when `MINIBOX_ADAPTER` is unset.
pub const DEFAULT_ADAPTER_SUITE: &str = "smolvm";

/// All known adapter suites with their compile-time availability.
///
/// Feature-gated and platform-gated adapters are included with
/// `available: false` when not compiled in, so error messages can
/// list them as "known but unavailable".
pub fn all_adapters() -> Vec<AdapterInfo> {
    vec![
        AdapterInfo {
            name: "native",
            description: "Linux namespaces, overlay FS, cgroups v2 (requires root)",
            available: cfg!(target_os = "linux"),
            platform: "linux",
        },
        AdapterInfo {
            name: "gke",
            description: "proot (ptrace), copy FS, no-op limiter (unprivileged GKE)",
            available: cfg!(target_os = "linux"),
            platform: "linux",
        },
        AdapterInfo {
            name: "colima",
            description: "Colima/Lima VM via limactl + nerdctl",
            available: cfg!(unix),
            platform: "any",
        },
        AdapterInfo {
            name: "smolvm",
            description: "SmolVM lightweight Linux VMs with subsecond boot",
            available: cfg!(unix),
            platform: "any",
        },
        AdapterInfo {
            name: "krun",
            description: "libkrun micro-VM (KVM on Linux, HVF on macOS)",
            available: true,
            platform: "any",
        },
    ]
}

/// Return only the names of adapters available in the current build.
pub fn available_adapter_names() -> Vec<&'static str> {
    all_adapters()
        .into_iter()
        .filter(|a| a.available)
        .map(|a| a.name)
        .collect()
}

/// Parse an adapter name string into an [`AdapterSuite`].
///
/// Returns a structured error listing all valid (and known-but-unavailable)
/// options when the name is not recognized or not available.
pub fn parse_adapter(name: &str) -> Result<AdapterSuite, AdapterSelectionError> {
    let suite = match name {
        "native" => AdapterSuite::Native,
        "gke" => AdapterSuite::Gke,
        "colima" => AdapterSuite::Colima,
        "smolvm" => AdapterSuite::SmolVm,
        "krun" => AdapterSuite::Krun,
        _ => {
            return Err(AdapterSelectionError {
                requested: name.to_string(),
                available: available_adapter_names(),
                all_known: all_adapters().into_iter().map(|a| a.name).collect(),
            });
        }
    };

    // Reject known-but-unavailable adapters in this build.
    let info = all_adapters();
    if let Some(adapter) = info.iter().find(|a| a.name == name)
        && !adapter.available
    {
        return Err(AdapterSelectionError {
            requested: name.to_string(),
            available: available_adapter_names(),
            all_known: info.into_iter().map(|a| a.name).collect(),
        });
    }

    Ok(suite)
}

/// Parse from the `MINIBOX_ADAPTER` environment variable.
///
/// Falls back to [`DEFAULT_ADAPTER_SUITE`] when the variable is unset.
pub fn adapter_from_env() -> Result<AdapterSuite, AdapterSelectionError> {
    let val =
        std::env::var("MINIBOX_ADAPTER").unwrap_or_else(|_| DEFAULT_ADAPTER_SUITE.to_string());
    parse_adapter(&val)
}

/// Structured error for invalid adapter selection.
#[derive(Debug, Clone)]
pub struct AdapterSelectionError {
    /// The value the user provided.
    pub requested: String,
    /// Adapter names available in this build.
    pub available: Vec<&'static str>,
    /// All known adapter names (including unavailable).
    pub all_known: Vec<&'static str>,
}

impl fmt::Display for AdapterSelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.all_known.contains(&self.requested.as_str()) {
            write!(
                f,
                "MINIBOX_ADAPTER {:?} is known but not available in this build. \
                 Available options: {}",
                self.requested,
                self.available.join(", ")
            )?;
        } else {
            write!(
                f,
                "unknown MINIBOX_ADAPTER value {:?}. Valid options: {}",
                self.requested,
                self.available.join(", ")
            )?;
        }
        let unavailable: Vec<_> = self
            .all_known
            .iter()
            .filter(|n| !self.available.contains(n))
            .collect();
        if !unavailable.is_empty() {
            write!(
                f,
                ". Known but unavailable in this build: {}",
                unavailable
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for AdapterSelectionError {}

#[cfg(test)]
mod tests {
    use super::*;

    // Serialize env-var-mutating tests to prevent parallel races.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    #[cfg(target_os = "linux")]
    fn parse_native_succeeds() {
        assert_eq!(
            parse_adapter("native").expect("should parse native"),
            AdapterSuite::Native
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn parse_gke_succeeds() {
        assert_eq!(
            parse_adapter("gke").expect("should parse gke"),
            AdapterSuite::Gke
        );
    }

    #[test]
    #[cfg(unix)]
    fn parse_colima_succeeds() {
        assert_eq!(
            parse_adapter("colima").expect("should parse colima on any unix"),
            AdapterSuite::Colima
        );
    }

    #[test]
    #[cfg(unix)]
    fn parse_smolvm_succeeds() {
        assert_eq!(
            parse_adapter("smolvm").expect("should parse smolvm on any unix"),
            AdapterSuite::SmolVm
        );
    }

    #[test]
    fn parse_unknown_returns_structured_error_with_valid_options() {
        let err = parse_adapter("invalid_adapter").expect_err("should fail for unknown adapter");
        assert_eq!(err.requested, "invalid_adapter");
        // Error message must list valid options
        let msg = err.to_string();
        assert!(
            msg.contains("native"),
            "error should list 'native' as valid option: {msg}"
        );
        assert!(
            msg.contains("gke"),
            "error should list 'gke' as valid option: {msg}"
        );
        assert!(
            msg.contains("colima"),
            "error should list 'colima' as valid option: {msg}"
        );
        assert!(
            msg.contains("smolvm"),
            "error should list 'smolvm' as valid option: {msg}"
        );
        assert!(
            msg.contains("invalid_adapter"),
            "error should echo the invalid value: {msg}"
        );
    }

    #[test]
    fn all_adapters_includes_native() {
        let adapters = all_adapters();
        assert!(
            adapters.iter().any(|a| a.name == "native"),
            "all_adapters must include 'native'"
        );
    }

    #[test]
    fn available_adapter_names_is_subset_of_all() {
        let available = available_adapter_names();
        let all: Vec<&str> = all_adapters().iter().map(|a| a.name).collect();
        for name in &available {
            assert!(
                all.contains(name),
                "available adapter {name} not in all_adapters"
            );
        }
    }

    #[test]
    fn adapter_suite_display_matches_parse_for_available() {
        let available = available_adapter_names();
        for suite in [
            AdapterSuite::Native,
            AdapterSuite::Gke,
            AdapterSuite::Colima,
            AdapterSuite::SmolVm,
            AdapterSuite::Krun,
        ] {
            let name = suite.to_string();
            if available.contains(&name.as_str()) {
                let parsed =
                    parse_adapter(&name).unwrap_or_else(|_| panic!("should round-trip: {name}"));
                assert_eq!(parsed, suite);
            } else {
                parse_adapter(&name).expect_err(&format!("unavailable suite should fail: {name}"));
            }
        }
    }

    #[test]
    fn adapter_from_env_defaults_to_smolvm() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: env var mutation serialized by ENV_LOCK
        unsafe {
            std::env::remove_var("MINIBOX_ADAPTER");
        }
        let suite = adapter_from_env().expect("smolvm default should parse on any unix platform");
        assert_eq!(suite, AdapterSuite::SmolVm);
    }

    #[test]
    fn parse_unavailable_adapter_returns_error() {
        // On macOS: native/gke are unavailable. On Linux: all adapters are available.
        // Only test on macOS where we know native is unavailable.
        if !cfg!(target_os = "macos") {
            return; // all adapters available on Linux — skip
        }
        let unavailable_name = "native";
        let err = parse_adapter(unavailable_name).expect_err("should reject unavailable adapter");
        assert_eq!(err.requested, unavailable_name);
        assert!(
            err.all_known.contains(&unavailable_name),
            "unavailable adapter should be in all_known"
        );
        assert!(
            !err.available.contains(&unavailable_name),
            "unavailable adapter should not be in available"
        );
    }

    #[test]
    fn unavailable_adapter_error_message_says_not_available() {
        if !cfg!(target_os = "macos") {
            return; // all adapters available on Linux — skip
        }
        let unavailable_name = "native";
        let err = parse_adapter(unavailable_name).expect_err("should reject unavailable adapter");
        let msg = err.to_string();
        assert!(
            msg.contains("not available"),
            "error for known-but-unavailable should say 'not available': {msg}"
        );
    }

    #[test]
    fn colima_metadata_targets_any() {
        let info = all_adapters();
        let colima = info
            .iter()
            .find(|a| a.name == "colima")
            .expect("colima entry");
        assert_eq!(colima.platform, "any");
        assert_eq!(colima.available, cfg!(unix));
    }

    #[test]
    fn smolvm_metadata_targets_any() {
        let info = all_adapters();
        let smolvm = info
            .iter()
            .find(|a| a.name == "smolvm")
            .expect("smolvm entry");
        assert_eq!(smolvm.platform, "any");
        assert_eq!(smolvm.available, cfg!(unix));
    }

    #[test]
    fn adapter_from_env_rejects_unknown() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: env var mutation serialized by ENV_LOCK
        unsafe {
            std::env::set_var("MINIBOX_ADAPTER", "bogus");
        }
        let err = adapter_from_env().expect_err("should reject bogus");
        assert_eq!(err.requested, "bogus");
        // Cleanup
        unsafe {
            std::env::remove_var("MINIBOX_ADAPTER");
        }
    }
}
