//! Shared fixture builders for benchmark targets.

pub mod layer;
pub mod registry;

pub use layer::{LayerSpec, build_layer_tar_gz, sha256_digest};
pub use registry::BenchRegistry;
