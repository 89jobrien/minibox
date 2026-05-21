//! krun adapter suite re-exports.
//!
//! The krun adapters currently live in `macbox::krun`. This module
//! re-exports them so consumers can depend on `smolbox` alone.

pub use macbox::krun::{
    filesystem::KrunFilesystem, limiter::KrunLimiter, process::SmolvmProcess,
    registry::KrunRegistry, runtime::KrunRuntime,
};
