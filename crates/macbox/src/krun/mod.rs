//! krun adapter suite — macOS microVM backend via smolvm/libkrun.
//!
//! Phase 1: shells out to `smolvm machine run` for process execution.
//! Phase 2: replace subprocess calls with direct libkrun FFI.

pub mod filesystem;
pub mod limiter;
pub mod process;
pub mod registry;
pub mod runtime;
