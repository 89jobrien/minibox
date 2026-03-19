//! Windows preflight checks — detects available container backend.

use anyhow::Result;

/// Detected Windows container backend.
#[derive(Debug, Clone, PartialEq)]
pub enum WinboxStatus {
    /// Windows Containers (HCS) is enabled.
    Hcs,
    /// WSL2 is available.
    Wsl2,
    /// Both HCS and WSL2 are available.
    HcsAndWsl2,
    /// No supported backend was found.
    NoBackendAvailable,
}

/// Injectable command executor.
pub type Executor = Box<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;

/// Build the real subprocess executor.
pub fn default_executor() -> Executor {
    Box::new(|args: &[&str]| {
        let out = std::process::Command::new(args[0])
            .args(&args[1..])
            .output()?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

fn check_hcs(exec: &Executor) -> bool {
    exec(&[
        "powershell",
        "-Command",
        "Get-WindowsOptionalFeature -Online -FeatureName Containers | Select-Object -ExpandProperty State",
    ])
    .map(|o| o.trim() == "Enabled")
    .unwrap_or(false)
}

fn check_wsl2(exec: &Executor) -> bool {
    exec(&["wsl", "--status"])
        .map(|o| !o.is_empty())
        .unwrap_or(false)
}

/// Probe for available container backends.
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
