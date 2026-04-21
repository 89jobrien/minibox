//! krun adapter suite — macOS microVM backend via smolvm/libkrun.
//!
//! Phase 1: shells out to `smolvm machine run` for process execution.
//! Phase 2: replace subprocess calls with direct libkrun FFI.

pub mod process;
