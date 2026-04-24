//! macOS preflight checks — detects available container backend.
//!
//! The [`preflight`] function probes for Colima by running `colima status` and
//! returns a [`MacboxStatus`] describing what was found. The injectable
//! [`Executor`] type allows tests to substitute fake subprocess results without
//! spawning real processes.

use anyhow::Result;

/// The macOS container backend detected during preflight.
///
/// Currently only Colima is probed by [`preflight`]. The
/// `VirtualizationFramework` variant is reserved for a planned Phase 2
/// integration with Apple's Virtualization.framework and is **never returned**
/// by the current `preflight` implementation.
#[derive(Debug, Clone, PartialEq)]
pub enum MacboxStatus {
    /// Colima is installed and `colima status` reports it is running.
    Colima,
    /// Colima is installed but its VM is not currently running.
    /// Call [`start_colima`] to start it before proceeding.
    ColimaNotRunning,
    /// Neither Colima nor any other supported backend was found.
    /// The daemon cannot start; the user must install a backend.
    NoBackendAvailable,
    /// Reserved for Phase 2: Apple Virtualization.framework integration.
    /// Not returned by the current [`preflight`] implementation.
    VirtualizationFramework,
}

/// Injectable command executor for preflight checks and VM lifecycle calls.
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
        if !out.status.success() {
            anyhow::bail!(
                "command {:?} exited with {}: {}",
                args,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

/// Probe for an available container backend and return its status.
///
/// Runs `colima status` via the provided executor. The output is checked for
/// the word `"running"` to distinguish a live VM from a stopped one. If the
/// executor returns an error (e.g., Colima is not installed), returns
/// [`MacboxStatus::NoBackendAvailable`].
///
/// `VirtualizationFramework` is never returned by this function.
pub fn preflight(exec: &Executor) -> MacboxStatus {
    match exec(&["colima", "status"]) {
        Ok(o) if o.contains("running") => MacboxStatus::Colima,
        Ok(_) => MacboxStatus::ColimaNotRunning,
        Err(_) => MacboxStatus::NoBackendAvailable,
    }
}

/// Start the Colima VM by running `colima start`.
///
/// Blocks until Colima reports that the VM is up. Returns an error if the
/// command fails or if Colima is not installed. Typically called after
/// [`preflight`] returns [`MacboxStatus::ColimaNotRunning`].
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
