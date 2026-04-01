//! CLI command modules.
//!
//! Each module implements a single subcommand using the [`minibox_client`] library
//! to communicate with the daemon. The [`DaemonClient`] abstraction handles socket
//! connection and protocol formatting.

pub mod load;
pub mod ps;
pub mod pull;
pub mod rm;
pub mod run;
pub mod stop;
