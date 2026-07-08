//! Per-adapter conformance test modules.
//!
//! Tests are registered via the `conformance_test!` macro and collected
//! by `inventory` at runtime. See [`TestRunner::collect_inventory`].

pub mod container_committer;
pub mod container_id;
pub mod exec_runtime;
pub mod filesystem;
pub mod image_builder;
pub mod image_loader;
pub mod image_pusher;
pub mod limiter;
pub mod list;
pub mod logs;
pub mod metrics;
pub mod network;
pub mod pause_resume;
pub mod policy;
pub mod pty;
pub mod registry;
pub mod registry_router;
pub mod remove;
pub mod runtime;
pub mod state;
pub mod stop_handler;
pub mod vm_checkpoint;
