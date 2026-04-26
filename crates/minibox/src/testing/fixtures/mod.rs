//! On-disk fixtures for conformance and integration tests.

pub mod build_context;
pub mod container;
pub mod image;
pub mod push_target;
pub mod upper_dir;

pub use build_context::BuildContextFixture;
pub use container::{MockAdapterBuilder, MockAdapterSet, TempContainerFixture};
pub use image::MinimalStoredImageFixture;
pub use push_target::LocalPushTargetFixture;
pub use upper_dir::WritableUpperDirFixture;
