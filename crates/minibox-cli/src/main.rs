//! minibox — CLI client for the miniboxd container runtime.
//!
//! Connects to the daemon over `/run/minibox/miniboxd.sock` and issues
//! JSON-over-newline requests, printing human-readable output.
//!
//! Each subcommand serialises a [`minibox_lib::protocol::DaemonRequest`] as a
//! single JSON line, writes it to the Unix socket, then reads one or more
//! [`minibox_lib::protocol::DaemonResponse`] lines back.  The `run` subcommand
//! is special: it uses `ephemeral: true` and loops, streaming
//! `ContainerOutput` chunks to the terminal until a `ContainerStopped` message
//! is received, at which point the CLI exits with the container's exit code.
//!
//! # Usage
//! ```text
//! minibox run alpine --tag latest -- /bin/sh
//! minibox ps
//! minibox stop <id>
//! minibox rm <id>
//! minibox pull nginx
//! ```

mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// Top-level CLI argument parser.  Delegates to [`Commands`] for subcommand
/// dispatch.
#[derive(Parser)]
#[command(
    name = "minibox",
    about = "A container runtime in Rust",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Available minibox subcommands.
///
/// Each variant maps directly to a [`minibox_lib::protocol::DaemonRequest`]
/// variant sent over the Unix socket.
#[derive(Subcommand)]
enum Commands {
    /// Run a container from an image.
    ///
    /// Sends an ephemeral `RunContainer` request to the daemon, then streams
    /// `ContainerOutput` chunks to stdout/stderr until `ContainerStopped` is
    /// received.  The process exits with the container's exit code.
    Run {
        /// Image name (e.g., alpine, ubuntu, library/nginx)
        image: String,

        /// Command to run in the container (everything after --)
        #[arg(last = true)]
        command: Vec<String>,

        /// Memory limit in bytes (passed to cgroups v2 `memory.max`)
        #[arg(long)]
        memory: Option<u64>,

        /// CPU weight in the range 1–10000 (passed to cgroups v2 `cpu.weight`)
        #[arg(long)]
        cpu_weight: Option<u64>,

        /// Image tag (default: latest)
        #[arg(short, long, default_value = "latest")]
        tag: String,

        /// Network mode: none (default), bridge, host, tailnet
        #[arg(long, default_value = "none")]
        network: String,
    },

    /// List all containers
    Ps,

    /// Stop a running container
    Stop {
        /// Container ID
        id: String,
    },

    /// Remove a stopped container
    Rm {
        /// Container ID
        id: String,
    },

    /// Pull an image from Docker Hub
    Pull {
        /// Image name (e.g., alpine, library/nginx)
        image: String,

        /// Image tag (default: latest)
        #[arg(short, long, default_value = "latest")]
        tag: String,
    },
}

/// Entry point.  Parses arguments, dispatches to the appropriate command
/// module, and propagates any errors as a non-zero exit code.
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            image,
            command,
            memory,
            cpu_weight,
            tag,
            network,
        } => commands::run::execute(image, tag, command, memory, cpu_weight, network).await,

        Commands::Ps => commands::ps::execute().await,

        Commands::Stop { id } => commands::stop::execute(id).await,

        Commands::Rm { id } => commands::rm::execute(id).await,

        Commands::Pull { image, tag } => commands::pull::execute(image, tag).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_parses_network_none() {
        let cli = Cli::try_parse_from([
            "minibox",
            "run",
            "--network",
            "none",
            "alpine",
            "--",
            "/bin/sh",
        ]);
        assert!(cli.is_ok());
    }

    #[test]
    fn cli_parses_network_host() {
        let cli = Cli::try_parse_from([
            "minibox",
            "run",
            "--network",
            "host",
            "alpine",
            "--",
            "/bin/sh",
        ]);
        assert!(cli.is_ok());
    }

    #[test]
    fn cli_default_network_is_none() {
        let cli = Cli::try_parse_from(["minibox", "run", "alpine", "--", "/bin/sh"]).unwrap();
        match cli.command {
            Commands::Run { network, .. } => assert_eq!(network, "none"),
            _ => panic!("expected Run"),
        }
    }
}
