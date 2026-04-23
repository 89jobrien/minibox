//! Mock adapters for minibox domain traits.

pub mod build;
pub mod commit;
pub mod exec;
pub mod filesystem;
pub mod limiter;
pub mod network;
pub mod push;
pub mod registry;
pub mod runtime;

pub use build::MockImageBuilder;
pub use commit::MockContainerCommitter;
pub use exec::MockExecRuntime;
pub use filesystem::{FailableFilesystemMock, MockFilesystem};
pub use limiter::MockLimiter;
pub use network::MockNetwork;
pub use push::MockImagePusher;
pub use registry::MockRegistry;
pub use runtime::MockRuntime;
