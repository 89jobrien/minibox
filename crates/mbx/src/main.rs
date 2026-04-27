//! mbx — CLI client for the miniboxd container runtime.
//!
//! Connects to the daemon over `/run/minibox/miniboxd.sock` and issues
//! JSON-over-newline requests, printing human-readable output.
//!
//! Each subcommand serialises a [`minibox_core::protocol::DaemonRequest`] as a
//! single JSON line, writes it to the Unix socket, then reads one or more
//! [`minibox_core::protocol::DaemonResponse`] lines back.  The `run` subcommand
//! is special: it uses `ephemeral: true` and loops, streaming
//! `ContainerOutput` chunks to the terminal until a `ContainerStopped` message
//! is received, at which point the CLI exits with the container's exit code.
//!
//! # Usage
//! ```text
//! mbx run alpine --tag latest -- /bin/sh
//! mbx ps
//! mbx stop <id>
//! mbx rm <id>
//! mbx pull nginx
//! ```

mod commands;
pub(crate) mod terminal;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use std::path::Path;

/// Top-level CLI argument parser.  Delegates to [`Commands`] for subcommand
/// dispatch.
#[derive(Parser)]
#[command(
    name = "mbx",
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
/// Each variant maps directly to a [`minibox::protocol::DaemonRequest`]
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

        /// Network mode: none (default), bridge, host, tailnet.
        /// 'none' runs the container in an isolated namespace with no network connectivity.
        #[arg(long, default_value = "none")]
        network: String,

        /// Grant full Linux capabilities to the container (required for DinD).
        #[arg(long)]
        privileged: bool,

        /// Bind mount in src:dst[:ro] format. Repeatable.
        /// Example: -v /tmp/bin:/minibox  -v /tmp/traces:/traces:ro
        #[arg(short = 'v', long = "volume", value_name = "SRC:DST[:ro]")]
        volumes: Vec<String>,

        /// Long-form mount specification. Repeatable.
        /// Example: --mount type=bind,src=/tmp/bin,dst=/minibox
        #[arg(long = "mount", value_name = "type=bind,src=PATH,dst=PATH[,readonly]")]
        mounts: Vec<String>,

        /// Assign a human-readable name to the container.
        /// Can be used instead of the ID in stop/rm commands.
        #[arg(long)]
        name: Option<String>,

        /// Allocate a pseudo-TTY.
        #[arg(long)]
        tty: bool,

        /// Keep stdin open (interactive mode).
        #[arg(short = 'i', long = "interactive")]
        interactive: bool,

        /// Set environment variables (KEY=VALUE). Repeatable.
        #[arg(short = 'e', long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,

        /// Override the image entrypoint.
        #[arg(long)]
        entrypoint: Option<String>,

        /// Run as a specific user (e.g. "nobody", "1000:1000").
        #[arg(short = 'u', long = "user")]
        user: Option<String>,

        /// Automatically remove the container when it exits.
        #[arg(long)]
        rm: bool,
    },

    /// List all containers
    Ps,

    /// Stop a running container
    Stop {
        /// Container ID
        id: String,
    },

    /// Pause a running container
    Pause {
        /// Container ID
        id: String,
    },

    /// Resume a paused container
    Resume {
        /// Container ID
        id: String,
    },

    /// Remove a stopped container
    Rm {
        /// Container ID (omit when using --all)
        id: Option<String>,

        /// Remove all stopped containers.
        #[arg(long)]
        all: bool,
    },

    /// Pull an image from Docker Hub
    Pull {
        /// Image name (e.g., alpine, library/nginx)
        image: String,

        /// Image tag (default: latest)
        #[arg(long, default_value = "latest")]
        tag: String,
    },

    /// Execute a command inside a running container.
    ///
    /// Sends a `DaemonRequest::Exec` to the daemon, then streams
    /// `ContainerOutput` chunks to stdout/stderr until `ContainerStopped` is
    /// received.  Exits with the exec process exit code.
    Exec {
        /// Container ID or name.
        container_id: String,

        /// Command and arguments to run (everything after --)
        #[arg(last = true, required = true)]
        cmd: Vec<String>,

        /// Allocate a pseudo-TTY.
        #[arg(long)]
        tty: bool,

        /// Keep stdin open (interactive mode).
        #[arg(short = 'i', long = "interactive")]
        interactive: bool,

        /// Run as a specific user (e.g. "nobody", "1000:1000").
        #[arg(short = 'u', long = "user")]
        user: Option<String>,
    },

    /// Fetch or stream log output from a container.
    ///
    /// Sends a `DaemonRequest::ContainerLogs` to the daemon and prints each
    /// log line to stdout (stdout stream) or stderr (stderr stream).
    Logs {
        /// Container ID or name.
        id: String,

        /// Keep the connection open and stream new output as it arrives.
        #[arg(long)]
        follow: bool,
    },

    /// Stream container lifecycle events as JSON-lines to stdout.
    ///
    /// Subscribes to the daemon event stream and prints each event as a
    /// newline-delimited JSON object until the connection is closed.
    Events,

    /// Remove unused images from the image store.
    ///
    /// Images currently in use by running or paused containers are skipped.
    Prune {
        /// Show what would be removed without actually deleting anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove a specific image by reference (e.g. alpine:latest).
    Rmi {
        /// Image reference in name:tag format.
        image_ref: String,
    },

    /// Run a script inside a sandboxed container with resource limits.
    ///
    /// Detects language from the file extension, bind-mounts the script
    /// read-only, and enforces safety defaults (512 MB, 60 s timeout, no
    /// network, no privileged).
    Sandbox {
        /// Path to the script file on the host.
        script: std::path::PathBuf,

        /// Image to use (default: minibox-sandbox:latest).
        #[arg(long, default_value = "minibox-sandbox")]
        image: String,

        /// Image tag.
        #[arg(long, default_value = "latest")]
        tag: String,

        /// Memory limit in MB (default: 512).
        #[arg(long, default_value = "512")]
        memory_mb: u64,

        /// Timeout in seconds (default: 60).
        #[arg(long, default_value = "60")]
        timeout: u64,

        /// Extra bind mounts in src:dst[:ro] format. Repeatable.
        #[arg(short = 'v', long = "volume")]
        volumes: Vec<String>,

        /// Enable bridge networking (default: no network).
        #[arg(long)]
        network: bool,
    },

    /// Manage VM state snapshots (save, restore, list).
    #[command(subcommand)]
    Snapshot(SnapshotCommands),

    /// Load an image from a local OCI tar archive
    Load {
        /// Path to the OCI image tar archive
        path: String,

        /// Image name (default: derived from filename without extension)
        #[arg(long)]
        name: Option<String>,

        /// Image tag (default: latest)
        #[arg(long, default_value = "latest")]
        tag: String,
    },
}

/// Snapshot sub-subcommands.
#[derive(Subcommand)]
enum SnapshotCommands {
    /// Save a VM state snapshot.
    Save {
        /// Container ID.
        id: String,
        /// Snapshot name (auto-generated if omitted).
        #[arg(long)]
        name: Option<String>,
    },
    /// Restore a VM state snapshot.
    Restore {
        /// Container ID.
        id: String,
        /// Snapshot name to restore.
        name: String,
    },
    /// List available snapshots.
    List {
        /// Container ID.
        id: String,
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

    let socket_path = minibox_core::client::default_socket_path();
    let socket_path: &Path = &socket_path;

    match cli.command {
        Commands::Run {
            image,
            command,
            memory,
            cpu_weight,
            tag,
            network,
            privileged,
            volumes,
            mounts,
            name,
            tty,
            interactive,
            env,
            entrypoint,
            user,
            rm,
        } => {
            commands::run::execute(
                image,
                tag,
                command,
                memory,
                cpu_weight,
                network,
                privileged,
                volumes,
                mounts,
                name,
                tty || interactive,
                env,
                entrypoint,
                user,
                rm,
                socket_path,
            )
            .await
        }

        Commands::Ps => commands::ps::execute(socket_path).await,

        Commands::Exec {
            container_id,
            cmd,
            tty,
            interactive,
            user,
        } => {
            commands::exec::execute(container_id, cmd, tty || interactive, user, socket_path).await
        }

        Commands::Stop { id } => commands::stop::execute(id, socket_path).await,

        Commands::Pause { id } => commands::pause::execute(id, socket_path).await,

        Commands::Resume { id } => commands::resume::execute(id, socket_path).await,

        Commands::Rm { id, all } => {
            if all {
                commands::rm::execute_all(socket_path).await
            } else if let Some(id) = id {
                commands::rm::execute(id, socket_path).await
            } else {
                eprintln!("error: provide a container ID or use --all");
                std::process::exit(1);
            }
        }

        Commands::Pull { image, tag } => commands::pull::execute(image, tag, socket_path).await,

        Commands::Load { path, name, tag } => {
            let name = name.unwrap_or_else(|| commands::load::name_from_path(&path));
            commands::load::execute(path, name, tag, socket_path).await
        }

        Commands::Logs { id, follow } => commands::logs::execute(id, follow, socket_path).await,

        Commands::Events => commands::events::execute(socket_path).await,

        Commands::Prune { dry_run } => commands::prune::execute(dry_run, socket_path).await,

        Commands::Rmi { image_ref } => commands::rmi::execute(image_ref, socket_path).await,

        Commands::Snapshot(sub) => match sub {
            SnapshotCommands::Save { id, name } => {
                commands::snapshot::execute_save(id, name, socket_path).await
            }
            SnapshotCommands::Restore { id, name } => {
                commands::snapshot::execute_restore(id, name, socket_path).await
            }
            SnapshotCommands::List { id } => {
                commands::snapshot::execute_list(id, socket_path).await
            }
        },

        Commands::Sandbox {
            script,
            image,
            tag,
            memory_mb,
            timeout,
            volumes,
            network,
        } => {
            let mut extra_mounts = Vec::new();
            for v in &volumes {
                let mut mount = commands::run::parse_volume(v)
                    .with_context(|| format!("invalid -v flag {:?}", v))?;
                mount.host_path = mount.host_path.canonicalize().with_context(|| {
                    format!(
                        "sandbox mount host path not found: {}",
                        mount.host_path.display()
                    )
                })?;
                extra_mounts.push(mount);
            }
            commands::sandbox::execute(
                script,
                image,
                tag,
                memory_mb,
                timeout,
                extra_mounts,
                network,
                socket_path,
            )
            .await
        }
    }
}

/// Test-only helpers that expose parser internals for unit tests in submodules.
#[cfg(test)]
pub mod main_tests_shim {
    use super::{Cli, Commands};
    use clap::Parser;

    /// Parse a `logs` subcommand invocation and return `(id, follow)`.
    pub fn parse_logs(args: &[&str]) -> (String, bool) {
        let cli = Cli::try_parse_from(args).expect("parse failed");
        match cli.command {
            Commands::Logs { id, follow } => (id, follow),
            _ => panic!("expected Logs command"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_parses_network_none() {
        let cli =
            Cli::try_parse_from(["mbx", "run", "--network", "none", "alpine", "--", "/bin/sh"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn cli_parses_network_host() {
        let cli =
            Cli::try_parse_from(["mbx", "run", "--network", "host", "alpine", "--", "/bin/sh"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn cli_default_network_is_none() {
        let cli = Cli::try_parse_from(["mbx", "run", "alpine", "--", "/bin/sh"]).unwrap();
        match cli.command {
            Commands::Run { network, .. } => assert_eq!(network, "none"),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn cli_parses_privileged_flag() {
        let cli = Cli::try_parse_from(["mbx", "run", "--privileged", "ubuntu", "--", "/bin/sh"]);
        assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Run { privileged, .. } => assert!(privileged),
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn cli_parses_volume_flag() {
        let cli = Cli::try_parse_from([
            "mbx",
            "run",
            "-v",
            "/tmp/host:/guest",
            "ubuntu",
            "--",
            "/bin/sh",
        ]);
        assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Run { volumes, .. } => {
                assert_eq!(volumes.len(), 1);
                assert_eq!(volumes[0], "/tmp/host:/guest");
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn cli_parses_multiple_volume_flags() {
        let cli = Cli::try_parse_from([
            "mbx",
            "run",
            "-v",
            "/tmp/a:/a",
            "-v",
            "/tmp/b:/b:ro",
            "ubuntu",
            "--",
            "/bin/sh",
        ]);
        assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Run { volumes, .. } => assert_eq!(volumes.len(), 2),
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn cli_parses_exec_subcommand() {
        let cli = Cli::try_parse_from(["mbx", "exec", "abc123", "--", "/bin/sh", "-c", "echo hi"]);
        assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Exec {
                container_id, cmd, ..
            } => {
                assert_eq!(container_id, "abc123");
                assert_eq!(cmd, vec!["/bin/sh", "-c", "echo hi"]);
            }
            _ => panic!("expected Exec"),
        }
    }

    #[test]
    fn cli_parses_name_flag() {
        let cli = Cli::try_parse_from([
            "mbx",
            "run",
            "--name",
            "my-container",
            "alpine",
            "--",
            "/bin/sh",
        ]);
        assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Run { name, .. } => {
                assert_eq!(name, Some("my-container".to_string()));
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn cli_run_without_name_is_none() {
        let cli = Cli::try_parse_from(["mbx", "run", "alpine", "--", "/bin/sh"]).unwrap();
        match cli.command {
            Commands::Run { name, .. } => assert_eq!(name, None),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn cli_parses_mount_flag() {
        let cli = Cli::try_parse_from([
            "mbx",
            "run",
            "--mount",
            "type=bind,src=/tmp/host,dst=/guest",
            "ubuntu",
            "--",
            "/bin/sh",
        ]);
        assert!(cli.is_ok(), "parse failed: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Run { mounts, .. } => assert_eq!(mounts.len(), 1),
            _ => panic!("wrong command"),
        }
    }
}
