// ---------------------------------------------------------------------------
// Domain Types
// ---------------------------------------------------------------------------

/// Container state machine.
///
/// Represents the lifecycle of a container from creation to removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    /// Container has been created but not yet started.
    Created,
    /// Container process is running.
    Running,
    /// Container is frozen (cgroup.freeze = 1).
    Paused,
    /// Container process has exited.
    Stopped,
    /// Container failed to start or crashed.
    Failed,
    /// Container was running in a previous daemon session but its PID is gone.
    Orphaned,
}

impl ContainerState {
    /// Return the canonical string representation of this state.
    ///
    /// The returned strings (`"Created"`, `"Running"`, `"Paused"`, `"Stopped"`,
    /// `"Failed"`, `"Orphaned"`) are used directly in
    /// [`crate::protocol::ContainerInfo::state`] list responses sent to the CLI.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
            Self::Orphaned => "Orphaned",
        }
    }
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
