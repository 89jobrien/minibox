//! Shared and module-specific error types for minibox.
//!
//! Each sub-module has its own fine-grained error type; they all implement
//! `std::error::Error` via thiserror and [`miette::Diagnostic`] for rich
//! CLI rendering (error codes, help text, related errors).

use miette::Diagnostic;
use thiserror::Error;

pub use minibox_core::error::{ImageError, MiniboxError, RegistryError};

// ---------------------------------------------------------------------------
// Filesystem errors
// ---------------------------------------------------------------------------

/// Errors from filesystem / overlay / mount operations.
#[derive(Debug, Error, Diagnostic)]
pub enum FilesystemError {
    #[error("failed to create directory {path}: {source}")]
    #[diagnostic(code(minibox::fs::create_dir))]
    /// A required directory could not be created.
    CreateDir {
        /// Directory path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to mount {fs} at {target}: {source}")]
    #[diagnostic(code(minibox::fs::mount))]
    /// A filesystem could not be mounted.
    Mount {
        /// Filesystem source or type.
        fs: String,
        /// Mount target path.
        target: String,
        /// Kernel mount error.
        #[source]
        source: nix::errno::Errno,
    },

    #[error("overlay mount failed: {0}")]
    #[diagnostic(
        code(minibox::fs::overlay),
        help("ensure overlayfs is supported and the layer paths exist")
    )]
    /// Mounting an overlay filesystem failed.
    OverlayMount(String),

    #[error("pivot_root failed: {0}")]
    #[diagnostic(code(minibox::fs::pivot_root))]
    /// Switching to the container root filesystem failed.
    PivotRoot(String),

    #[error("failed to unmount {target}: {source}")]
    #[diagnostic(code(minibox::fs::umount))]
    /// A filesystem could not be unmounted.
    Umount {
        /// Mount target path.
        target: String,
        /// Kernel unmount error.
        #[source]
        source: nix::errno::Errno,
    },

    #[error("cleanup failed for {path}: {source}")]
    #[diagnostic(code(minibox::fs::cleanup))]
    /// Filesystem cleanup failed.
    Cleanup {
        /// Path being cleaned.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// Cgroup errors
// ---------------------------------------------------------------------------

/// Errors from cgroup v2 operations.
#[derive(Debug, Error, Diagnostic)]
pub enum CgroupError {
    #[error("failed to create cgroup directory {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::create))]
    /// A cgroup directory could not be created.
    CreateFailed {
        /// Cgroup path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to add process {pid} to cgroup {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::add_process))]
    /// A process could not be attached to a cgroup.
    AddProcessFailed {
        /// Host process identifier.
        pid: u32,
        /// Cgroup path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write cgroup file {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::write))]
    /// A cgroup control file could not be written.
    WriteFailed {
        /// Control-file path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to clean up cgroup {path}: {source}")]
    #[diagnostic(code(minibox::cgroup::cleanup))]
    /// A cgroup could not be removed.
    CleanupFailed {
        /// Cgroup path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// Namespace errors
// ---------------------------------------------------------------------------

/// Errors from Linux namespace operations (clone, setns, etc.).
#[derive(Debug, Error, Diagnostic)]
pub enum NamespaceError {
    #[error("clone(2) failed: {0}")]
    #[diagnostic(
        code(minibox::namespace::clone_failed),
        help("ensure you have CAP_SYS_ADMIN or are running as root")
    )]
    /// Creating the namespaced process failed.
    CloneFailed(String),

    #[error("failed to set hostname: {0}")]
    #[diagnostic(code(minibox::namespace::hostname))]
    /// Setting the container hostname failed.
    SetHostnameFailed(String),

    #[error("namespace error: {0}")]
    #[diagnostic(code(minibox::namespace::other))]
    /// A namespace operation failed for another reason.
    Other(String),
}

// ---------------------------------------------------------------------------
// Process errors
// ---------------------------------------------------------------------------

/// Errors from spawning and managing container processes.
#[derive(Debug, Error, Diagnostic)]
pub enum ProcessError {
    #[error("failed to spawn container process: {0}")]
    #[diagnostic(code(minibox::process::spawn))]
    /// Spawning the container process failed.
    SpawnFailed(String),

    #[error("exec failed for command {cmd}: {source}")]
    #[diagnostic(code(minibox::process::exec))]
    /// Replacing the child process image failed.
    ExecFailed {
        /// Command being executed.
        cmd: String,
        /// Kernel exec error.
        #[source]
        source: nix::errno::Errno,
    },

    #[error("waitpid failed for PID {pid}: {source}")]
    #[diagnostic(code(minibox::process::wait))]
    /// Waiting for a container process failed.
    WaitFailed {
        /// Host process identifier.
        pid: u32,
        /// Kernel wait error.
        #[source]
        source: nix::errno::Errno,
    },

    #[error("process error: {0}")]
    #[diagnostic(code(minibox::process::other))]
    /// A process operation failed for another reason.
    Other(String),

    #[error("failed to install mount immutability seccomp filter: {0}")]
    #[diagnostic(code(minibox::process::seccomp_install))]
    /// Installing the mount-protection seccomp filter failed.
    SeccompInstallFailed(String),
}
