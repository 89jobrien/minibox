//! Windows preflight checks — detects available container backend.
//!
//! The [`preflight`] function probes for Windows Containers (HCS) and WSL2
//! independently and returns a [`WinboxStatus`] reflecting what is available.
//! The injectable [`Executor`] type lets tests substitute fake subprocess
//! results without running PowerShell or `wsl.exe`.

use anyhow::Result;

/// The Windows container backend(s) detected during preflight.
#[derive(Debug, Clone, PartialEq)]
pub enum WinboxStatus {
    /// The Windows Containers optional feature (HCS) is enabled, but WSL2
    /// is not available.
    Hcs,
    /// WSL2 is available, but the Windows Containers feature is not enabled.
    Wsl2,
    /// Both HCS and WSL2 are available. Phase 2 will prefer HCS for native
    /// Windows Containers and WSL2 for Linux containers.
    HcsAndWsl2,
    /// Neither backend was found. The daemon cannot start; the user must
    /// enable Windows Containers or install WSL2.
    NoBackendAvailable,
}

/// Injectable command executor for preflight checks.
///
/// `args[0]` is the program name; the remaining elements are its arguments.
/// Returns the captured stdout of the subprocess as a `String`.
///
/// The real implementation (from [`default_executor`]) spawns a child process.
/// Tests inject a closure that returns a predetermined string without forking,
/// keeping preflight tests hermetic and fast.
pub type Executor = Box<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;

/// Build the real subprocess executor.
///
/// Spawns a child process using [`std::process::Command`] and returns its
/// stdout. Stderr is not captured. Returns an error if the process cannot
/// be spawned or if stdout is not valid UTF-8.
pub fn default_executor() -> Executor {
    Box::new(|args: &[&str]| {
        let out = std::process::Command::new(args[0])
            .args(&args[1..])
            .output()?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

/// Check whether the Windows Containers optional feature (HCS) is enabled.
///
/// Runs `powershell -Command "Get-WindowsOptionalFeature -Online
/// -FeatureName Containers | Select-Object -ExpandProperty State"` and checks
/// whether the trimmed output equals `"Enabled"`. Returns `false` on any
/// executor error (e.g., PowerShell not found, feature not present).
fn check_hcs(exec: &Executor) -> bool {
    exec(&[
        "powershell",
        "-Command",
        "Get-WindowsOptionalFeature -Online -FeatureName Containers | Select-Object -ExpandProperty State",
    ])
    .map(|o| o.trim() == "Enabled")
    .unwrap_or(false)
}

/// Check whether WSL2 is installed and available.
///
/// Runs `wsl --status` and treats any non-empty output as a signal that WSL2
/// is present. Returns `false` on any executor error (e.g., `wsl.exe` not
/// found or WSL not enabled in Windows Features).
fn check_wsl2(exec: &Executor) -> bool {
    exec(&["wsl", "--status"])
        .map(|o| !o.is_empty())
        .unwrap_or(false)
}

/// Probe for available container backends and return the combined status.
///
/// Runs [`check_hcs`] and [`check_wsl2`] independently and maps the results
/// to the appropriate [`WinboxStatus`] variant. Both checks are always
/// performed regardless of the outcome of the first.
pub fn preflight(exec: &Executor) -> WinboxStatus {
    match (check_hcs(exec), check_wsl2(exec)) {
        (true, true) => WinboxStatus::HcsAndWsl2,
        (true, false) => WinboxStatus::Hcs,
        (false, true) => WinboxStatus::Wsl2,
        _ => WinboxStatus::NoBackendAvailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fail() -> Executor {
        Box::new(|_| Err(anyhow::anyhow!("not found")))
    }

    #[test]
    fn no_backend_when_both_fail() {
        assert_eq!(preflight(&fail()), WinboxStatus::NoBackendAvailable);
    }
}
