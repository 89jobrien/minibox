mod adapters;
mod client;
mod error;
mod models;
mod server;
mod tracker;

use clap::Parser;

#[derive(Parser)]
#[command(name = "mbxctl")]
#[command(about = "Minibox orchestration controller")]
struct Args {
    /// Address to listen on
    #[arg(long, default_value = "localhost:9999")]
    listen: String,

    /// Path to minibox daemon socket
    #[arg(long, env = "MINIBOX_SOCKET_PATH")]
    socket: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("mbxctl=debug".parse()?),
        )
        .init();

    let args = Args::parse();

    tracing::info!(addr = %args.listen, "mbxctl: starting");
    server::run(&args.listen, args.socket).await
}
