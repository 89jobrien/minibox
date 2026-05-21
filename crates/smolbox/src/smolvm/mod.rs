//! SmolVM adapter suite re-exports.
//!
//! The smolvm adapters currently live in `minibox::adapters`.
//! This module re-exports them so consumers can depend on `smolbox` alone.

pub use minibox::adapters::{
    SmolVmExecutor, SmolVmFilesystem, SmolVmLimiter, SmolVmRegistry, SmolVmRuntime,
};
