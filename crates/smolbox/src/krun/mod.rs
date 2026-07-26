//! krun adapter suite re-exports.
//!
//! The krun adapters currently live in `macbox::krun`. This module
//! re-exports them so consumers can depend on `smolbox` alone.
//!
//! Both flat re-exports (`smolbox::krun::KrunRuntime`) and module-path
//! re-exports (`smolbox::krun::runtime::KrunRuntime`) are available for
//! compatibility with existing import styles.

pub use macbox::krun::{
    filesystem::KrunFilesystem, limiter::KrunLimiter, process::SmolvmProcess,
    registry::KrunRegistry, runtime::KrunRuntime,
};

/// Re-export `macbox::krun::filesystem` for module-path imports.
pub mod filesystem {
    pub use macbox::krun::filesystem::*;
}

/// Re-export `macbox::krun::limiter` for module-path imports.
pub mod limiter {
    pub use macbox::krun::limiter::*;
}

/// Re-export `macbox::krun::registry` for module-path imports.
pub mod registry {
    pub use macbox::krun::registry::*;
}

/// Re-export `macbox::krun::runtime` for module-path imports.
pub mod runtime {
    pub use macbox::krun::runtime::*;
}

/// Re-export `macbox::krun::process` for module-path imports.
pub mod process {
    pub use macbox::krun::process::*;
}
