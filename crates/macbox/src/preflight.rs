//! macOS preflight checks — detects available container backend.

use anyhow::Result;

/// Detected macOS container backend.
#[derive(Debug, Clone, PartialEq)]
pub enum MacboxStatus {
    /// Colima is installed and running.
    Colima,
    /// Colima is installed but not running.
    ColimaNotRunning,
    /// No supported backend was found.
    NoBackendAvailable,
    /// Apple Virtualization Framework (Phase 2).
    VirtualizationFramework,
}

/// Injectable command executor used by preflight and start_colima.
///
/// Takes a slice of arguments (`args[0]` is the program) and returns stdout.
/// The default implementation calls a real subprocess; tests inject fakes.
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

/// Probe for available container backends.
pub fn preflight(exec: &Executor) -> MacboxStatus {
    match exec(&["colima", "status"]) {
        Ok(o) if o.contains("running") => MacboxStatus::Colima,
        Ok(_) => MacboxStatus::ColimaNotRunning,
        Err(_) => MacboxStatus::NoBackendAvailable,
    }
}

/// Start the Colima VM.
pub fn start_colima(exec: &Executor) -> Result<()> {
    exec(&["colima", "start"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &'static str) -> Executor {
        Box::new(move |_| Ok(s.to_string()))
    }

    fn fail() -> Executor {
        Box::new(|_| Err(anyhow::anyhow!("not found")))
    }

    #[test]
    fn colima_running() {
        assert_eq!(preflight(&ok("colima is running")), MacboxStatus::Colima);
    }

    #[test]
    fn colima_stopped() {
        assert_eq!(
            preflight(&ok("colima is stopped")),
            MacboxStatus::ColimaNotRunning
        );
    }

    #[test]
    fn no_backend() {
        assert_eq!(preflight(&fail()), MacboxStatus::NoBackendAvailable);
    }

    #[test]
    fn start_ok() {
        assert!(start_colima(&ok("")).is_ok());
    }
}
