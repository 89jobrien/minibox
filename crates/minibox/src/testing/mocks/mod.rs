//! Mock adapters for minibox domain traits.

pub mod build;
pub mod commit;
pub mod exec;
pub mod filesystem;
pub mod image_loader;
pub mod limiter;
pub mod metrics;
pub mod network;
pub mod pty;
pub mod push;
pub mod registry;
pub mod registry_router;
pub mod runtime;
pub mod vm_checkpoint;

pub use build::MockImageBuilder;
pub use commit::MockContainerCommitter;
pub use exec::MockExecRuntime;
pub use filesystem::{FailableFilesystemMock, MockFilesystem};
pub use image_loader::MockImageLoader;
pub use limiter::MockLimiter;
pub use metrics::MockMetricsRecorder;
pub use network::MockNetwork;
pub use pty::MockPtyAllocator;
pub use push::MockImagePusher;
pub use registry::MockRegistry;
pub use registry_router::MockRegistryRouter;
pub use runtime::MockRuntime;
pub use vm_checkpoint::MockVmCheckpoint;
