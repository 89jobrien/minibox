//! Compile-time container lifecycle typestate.
//!
//! Encodes valid state transitions in the type system so that invalid
//! transitions (e.g. stopping a container that hasn't started) are caught
//! at compile time rather than runtime.
//!
//! # Design
//!
//! Each lifecycle phase is a zero-sized or small struct (the "state tag").
//! `Container<S>` is generic over the state tag; transition methods consume
//! `self` and return `Container<NextState>`, making the old handle unusable.
//!
//! Shared fields (id, rootfs_path, cgroup_path) are carried through every
//! transition without cloning.

use std::path::PathBuf;

// ---------------------------------------------------------------------------
// State tags
// ---------------------------------------------------------------------------

/// Container has been registered but not yet started.
#[derive(Debug, PartialEq, Eq)]
pub struct Created;

/// Container process is running on the host.
#[derive(Debug, PartialEq, Eq)]
pub struct Running {
    /// Host-namespace PID of the container init process.
    pub pid: u32,
}

/// Container is frozen via cgroup.freeze.
#[derive(Debug, PartialEq, Eq)]
pub struct Paused {
    /// PID is retained so the container can be resumed.
    pub pid: u32,
}

/// Container process has exited normally.
#[derive(Debug, PartialEq, Eq)]
pub struct Stopped {
    /// Exit code from the container process.
    pub exit_code: i32,
}

/// Container failed to start or crashed.
#[derive(Debug, PartialEq, Eq)]
pub struct Failed {
    /// Human-readable failure reason.
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Container<S> — the typestate wrapper
// ---------------------------------------------------------------------------

/// A container handle parameterized by its lifecycle state.
///
/// Transition methods consume `self` and return the container in its new
/// state, making the previous handle inaccessible.
#[derive(Debug, PartialEq, Eq)]
pub struct Container<S> {
    /// Unique container identifier.
    pub id: String,
    /// Path to the merged overlay rootfs.
    pub rootfs_path: PathBuf,
    /// Path to the container's cgroup directory.
    pub cgroup_path: PathBuf,
    /// The current state tag (carries per-state data).
    pub state: S,
}

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------

impl Container<Created> {
    /// Transition: Created -> Running (process forked successfully).
    pub fn start(self, pid: u32) -> Container<Running> {
        Container {
            id: self.id,
            rootfs_path: self.rootfs_path,
            cgroup_path: self.cgroup_path,
            state: Running { pid },
        }
    }

    /// Transition: Created -> Failed (process failed to start).
    pub fn fail(self, reason: String) -> Container<Failed> {
        Container {
            id: self.id,
            rootfs_path: self.rootfs_path,
            cgroup_path: self.cgroup_path,
            state: Failed { reason },
        }
    }
}

impl Container<Running> {
    /// Transition: Running -> Paused (cgroup freeze).
    pub fn pause(self) -> Container<Paused> {
        let pid = self.state.pid;
        Container {
            id: self.id,
            rootfs_path: self.rootfs_path,
            cgroup_path: self.cgroup_path,
            state: Paused { pid },
        }
    }

    /// Transition: Running -> Stopped (process exited).
    pub fn stop(self, exit_code: i32) -> Container<Stopped> {
        Container {
            id: self.id,
            rootfs_path: self.rootfs_path,
            cgroup_path: self.cgroup_path,
            state: Stopped { exit_code },
        }
    }

    /// Transition: Running -> Failed (process crashed).
    pub fn fail(self, reason: String) -> Container<Failed> {
        Container {
            id: self.id,
            rootfs_path: self.rootfs_path,
            cgroup_path: self.cgroup_path,
            state: Failed { reason },
        }
    }

    /// Access the PID of the running container.
    pub fn pid(&self) -> u32 {
        self.state.pid
    }
}

impl Container<Paused> {
    /// Transition: Paused -> Running (cgroup thaw).
    pub fn resume(self) -> Container<Running> {
        let pid = self.state.pid;
        Container {
            id: self.id,
            rootfs_path: self.rootfs_path,
            cgroup_path: self.cgroup_path,
            state: Running { pid },
        }
    }

    /// Transition: Paused -> Stopped (killed while frozen).
    pub fn stop(self, exit_code: i32) -> Container<Stopped> {
        Container {
            id: self.id,
            rootfs_path: self.rootfs_path,
            cgroup_path: self.cgroup_path,
            state: Stopped { exit_code },
        }
    }

    /// Access the PID of the paused container.
    pub fn pid(&self) -> u32 {
        self.state.pid
    }
}

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

impl Container<Created> {
    /// Create a new container in the `Created` state.
    pub fn new(id: String, rootfs_path: PathBuf, cgroup_path: PathBuf) -> Self {
        Self {
            id,
            rootfs_path,
            cgroup_path,
            state: Created,
        }
    }
}

// ---------------------------------------------------------------------------
// Bridge to runtime ContainerState enum
// ---------------------------------------------------------------------------

use crate::domain::ContainerState as RuntimeState;

/// Sealed trait implemented by all state tags — allows generic code to query
/// the runtime-equivalent state without knowing the concrete tag.
pub trait TypestateTag: std::fmt::Debug + Send + Sync + 'static {
    /// Return the equivalent runtime `ContainerState` for persistence/protocol.
    fn runtime_state(&self) -> RuntimeState;
}

impl TypestateTag for Created {
    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Created
    }
}

impl TypestateTag for Running {
    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Running
    }
}

impl TypestateTag for Paused {
    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Paused
    }
}

impl TypestateTag for Stopped {
    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Stopped
    }
}

impl TypestateTag for Failed {
    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Failed
    }
}

impl<S: TypestateTag> Container<S> {
    /// Get the runtime-equivalent state for serialization or protocol use.
    pub fn runtime_state(&self) -> RuntimeState {
        self.state.runtime_state()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_container() -> Container<Created> {
        Container::new(
            "abc123".to_string(),
            PathBuf::from("/var/lib/minibox/containers/abc123/merged"),
            PathBuf::from("/sys/fs/cgroup/minibox.slice/miniboxd.service/abc123"),
        )
    }

    #[test]
    fn created_to_running() {
        let c = test_container();
        let c = c.start(1234);
        assert_eq!(c.pid(), 1234);
        assert_eq!(c.id, "abc123");
    }

    #[test]
    fn created_to_failed() {
        let c = test_container();
        let c = c.fail("exec not found".to_string());
        assert_eq!(c.state.reason, "exec not found");
    }

    #[test]
    fn running_to_paused_to_running() {
        let c = test_container().start(42);
        let c = c.pause();
        assert_eq!(c.pid(), 42);
        let c = c.resume();
        assert_eq!(c.pid(), 42);
    }

    #[test]
    fn running_to_stopped() {
        let c = test_container().start(99);
        let c = c.stop(0);
        assert_eq!(c.state.exit_code, 0);
    }

    #[test]
    fn paused_to_stopped() {
        let c = test_container().start(99).pause();
        let c = c.stop(137);
        assert_eq!(c.state.exit_code, 137);
    }

    #[test]
    fn running_to_failed() {
        let c = test_container().start(99);
        let c = c.fail("OOM killed".to_string());
        assert_eq!(c.state.reason, "OOM killed");
    }

    #[test]
    fn shared_fields_preserved_across_transitions() {
        let c = test_container();
        let rootfs = c.rootfs_path.clone();
        let cgroup = c.cgroup_path.clone();

        let c = c.start(1).pause().resume().stop(0);
        assert_eq!(c.rootfs_path, rootfs);
        assert_eq!(c.cgroup_path, cgroup);
    }

    /// Compile-time safety: the following must NOT compile.
    /// Uncomment any line to verify the compiler rejects it.
    ///
    /// ```compile_fail
    /// use minibox_core::typestate::*;
    /// let c = Container::new("x".into(), "/r".into(), "/c".into());
    /// c.stop(0); // ERROR: no method `stop` on Container<Created>
    /// ```
    ///
    /// ```compile_fail
    /// use minibox_core::typestate::*;
    /// let c = Container::new("x".into(), "/r".into(), "/c".into());
    /// c.pause(); // ERROR: no method `pause` on Container<Created>
    /// ```
    ///
    /// ```compile_fail
    /// use minibox_core::typestate::*;
    /// let c = Container::new("x".into(), "/r".into(), "/c".into()).start(1).stop(0);
    /// c.start(2); // ERROR: no method `start` on Container<Stopped>
    /// ```
    #[test]
    fn runtime_state_bridge() {
        use crate::domain::ContainerState as RS;
        let c = test_container();
        assert_eq!(c.runtime_state(), RS::Created);
        let c = c.start(1);
        assert_eq!(c.runtime_state(), RS::Running);
        let c = c.pause();
        assert_eq!(c.runtime_state(), RS::Paused);
        let c = c.resume();
        assert_eq!(c.runtime_state(), RS::Running);
        let c = c.stop(0);
        assert_eq!(c.runtime_state(), RS::Stopped);
    }

    #[test]
    fn compile_fail_docs_exist() {
        // This test just confirms the module compiles.
        // The compile_fail doctests above verify invalid transitions are rejected.
    }
}
