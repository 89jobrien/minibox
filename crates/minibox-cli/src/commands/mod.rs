//! CLI command modules.
//!
//! Each module implements a single subcommand using the [`minibox_client`] library
//! to communicate with the daemon. The [`DaemonClient`] abstraction handles socket
//! connection and protocol formatting.

pub mod events;
pub mod exec;
pub mod load;
pub mod pause;
pub mod prune;
pub mod ps;
pub mod pull;
pub mod resume;
pub mod rm;
pub mod rmi;
pub mod run;
pub mod stop;
