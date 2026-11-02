//! Signing probes for the Virtualization.framework entitlement.
//!
//! macOS requires `com.apple.security.virtualization` on the process that creates a
//! `VZVirtualMachine`. Without it, `VZVirtualMachineConfiguration::validateWithError`
//! fails with `VZErrorDomain code=2` (`invalidVirtualMachineConfiguration`) and a message
//! naming the missing entitlement, which is easy to misread as a boot-loader or image
//! problem. That code also covers other invalid configurations, so the message — not the
//! code — identifies the entitlement as the cause.
//!
//! The entitlement is read from the running executable's code signature with `codesign`
//! rather than Security.framework: one boolean does not justify linking another framework,
//! and [`crate::vz::bindings`] already shells out to `sw_vers` for its version check.

use anyhow::{Context, Result};
use std::path::Path;

/// Entitlement key Virtualization.framework requires.
pub const VIRTUALIZATION_ENTITLEMENT: &str = "com.apple.security.virtualization";

/// Whether the running executable is signed with [`VIRTUALIZATION_ENTITLEMENT`].
///
/// Returns `Err` only when the signature cannot be read (for example `codesign` is
/// missing); a readable signature without the entitlement returns `Ok(false)`.
pub fn process_is_virtualization_entitled() -> Result<bool> {
    let exe = std::env::current_exe().context("resolve current executable")?;
    executable_is_virtualization_entitled(&exe)
}

/// Whether the executable at `path` is signed with [`VIRTUALIZATION_ENTITLEMENT`].
pub fn executable_is_virtualization_entitled(path: &Path) -> Result<bool> {
    let output = std::process::Command::new("codesign")
        .args(["-d", "--entitlements", ":-"])
        .arg(path)
        .output()
        .context("run codesign to read entitlements")?;
    if !output.status.success() {
        anyhow::bail!(
            "codesign could not read entitlements for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    // codesign writes the entitlement plist to stdout; an unsigned binary with no
    // entitlements emits an empty plist, which parses as "not entitled".
    Ok(entitlements_grant_virtualization(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Whether an entitlement plist grants [`VIRTUALIZATION_ENTITLEMENT`].
///
/// Deliberately a substring scan rather than a plist parser: the input comes from
/// `codesign`, its shape is fixed, and the crate has no plist dependency. The value is
/// taken from the window between the key and the next key so a `<true/>` belonging to a
/// later entitlement cannot be mistaken for this one.
pub fn entitlements_grant_virtualization(plist: &str) -> bool {
    let Some(idx) = plist.find(VIRTUALIZATION_ENTITLEMENT) else {
        return false;
    };
    let rest = &plist[idx + VIRTUALIZATION_ENTITLEMENT.len()..];
    let window = rest.split("<key>").next().unwrap_or(rest);
    window.contains("<true/>")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTITLED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>com.apple.security.virtualization</key><true/></dict></plist>"#;

    const OTHER_ONLY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>com.apple.security.cs.allow-jit</key><true/></dict></plist>"#;

    #[test]
    fn grants_when_key_is_true() {
        assert!(entitlements_grant_virtualization(ENTITLED));
    }

    #[test]
    fn denies_when_key_is_false() {
        let plist = r#"<plist version="1.0"><dict><key>com.apple.security.virtualization</key><false/></dict></plist>"#;
        assert!(!entitlements_grant_virtualization(plist));
    }

    #[test]
    fn denies_when_key_absent() {
        assert!(!entitlements_grant_virtualization(OTHER_ONLY));
    }

    #[test]
    fn denies_empty_output() {
        assert!(!entitlements_grant_virtualization(""));
    }

    #[test]
    fn ignores_true_belonging_to_a_later_key() {
        let plist = r#"<plist version="1.0"><dict><key>com.apple.security.virtualization</key><false/><key>com.apple.security.cs.allow-jit</key><true/></dict></plist>"#;
        assert!(!entitlements_grant_virtualization(plist));
    }

    #[test]
    fn ignores_key_named_with_a_prefix() {
        let plist = r#"<plist version="1.0"><dict><key>com.apple.security.virtualization.debug</key><false/></dict></plist>"#;
        // A prefixed key still contains the substring; the check must not report true
        // because the value window holds `<false/>`.
        assert!(!entitlements_grant_virtualization(plist));
    }
}
