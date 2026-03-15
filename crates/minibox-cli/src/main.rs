//! minibox — CLI client for the miniboxd container runtime.
//!
//! Connects to the daemon over `/run/minibox/miniboxd.sock` and issues
//! JSON requests, printing human-readable output.
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

#[derive(Subcommand)]
enum Commands {
    /// Run a container from an image
    Run {
        /// Image name (e.g., alpine, ubuntu, library/nginx)
        image: String,

        /// Command to run in the container (everything after --)
        #[arg(last = true)]
        command: Vec<String>,

        /// Memory limit in bytes
        #[arg(long)]
        memory: Option<u64>,

        /// CPU weight (1–10000)
        #[arg(long)]
        cpu_weight: Option<u64>,

        /// Image tag (default: latest)
        #[arg(short, long, default_value = "latest")]
        tag: String,
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
        } => commands::run::execute(image, tag, command, memory, cpu_weight).await,

        Commands::Ps => commands::ps::execute().await,

        Commands::Stop { id } => commands::stop::execute(id).await,

        Commands::Rm { id } => commands::rm::execute(id).await,

        Commands::Pull { image, tag } => commands::pull::execute(image, tag).await,
    }
}
