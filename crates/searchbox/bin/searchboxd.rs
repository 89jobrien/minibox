//! searchboxd — composition root for searchbox.
//!
//! Subcommands:
//!   mcp        — run MCP stdio server (for Claude Code)
//!   status     — check zoekt-webserver health
//!   reindex    — trigger reindex (optionally for one repo)
//!   provision  — download + deploy Zoekt binaries to VPS, then start service

use anyhow::Result;
use clap::{Parser, Subcommand};
use searchbox::{
    adapters::{merged::MergedAdapter, zoekt::ZoektAdapter},
    config::SearchboxConfig,
    domain::{ServiceError, ServiceManager, ServiceStatus},
    mcp,
};
use zoektbox::service::{ZoektServiceAdapter, ZoektServiceConfig};

#[derive(Parser)]
#[command(name = "searchboxd", version, about = "Zoekt-backed code search MCP server")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run as MCP stdio server
    Mcp,
    /// Check zoekt-webserver health
    Status,
    /// Trigger reindex (--repo NAME for single repo, omit for all)
    Reindex {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Provision Zoekt on the VPS (first-time setup)
    Provision,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cfg = SearchboxConfig::load_default()?;

    let svc_cfg = ZoektServiceConfig {
        ssh_host: cfg.service.vps_host.clone(),
        port: cfg.service.zoekt_port,
        remote_base: "/opt/zoekt".into(),
    };
    let service = ZoektServiceAdapter::new(svc_cfg);

    match cli.cmd {
        Cmd::Provision => {
            service.provision().await?;
            service.start().await?;
            println!(
                "Provisioning complete. zoekt-webserver running on {}:{}",
                cfg.service.vps_host, cfg.service.zoekt_port
            );
        }

        Cmd::Status => {
            let running = service.status().await?;
            println!("{}", if running { "running" } else { "stopped" });
        }

        Cmd::Reindex { repo } => {
            service.reindex(repo.as_deref()).await?;
            println!("Reindex triggered");
        }

        Cmd::Mcp => {
            let base_url = format!(
                "http://{}:{}",
                cfg.service.vps_host, cfg.service.zoekt_port
            );
            let zoekt = ZoektAdapter::new(&base_url);

            let mut providers: Vec<Box<dyn searchbox::domain::SearchProvider>> =
                vec![Box::new(zoekt)];

            if cfg.local.enabled {
                providers.push(Box::new(
                    searchbox::adapters::local::LocalZoektSource::new(
                        "local",
                        "",
                        cfg.local.port,
                    ),
                ));
            }

            let merged = MergedAdapter::new(providers);

            let svc_bridge = ServiceBridge { inner: service };
            mcp::run_stdio_loop(&merged, &svc_bridge).await?;
        }
    }

    Ok(())
}

/// Bridge ZoektServiceAdapter's direct async methods into the ServiceManager trait.
struct ServiceBridge {
    inner: ZoektServiceAdapter,
}

#[async_trait::async_trait]
impl ServiceManager for ServiceBridge {
    async fn start(&self) -> Result<(), ServiceError> {
        self.inner
            .start()
            .await
            .map_err(|e| ServiceError::Process(e.to_string()))
    }

    async fn stop(&self) -> Result<(), ServiceError> {
        self.inner
            .stop()
            .await
            .map_err(|e| ServiceError::Process(e.to_string()))
    }

    async fn status(&self) -> Result<ServiceStatus, ServiceError> {
        let running = self
            .inner
            .status()
            .await
            .map_err(|e| ServiceError::Ssh(e.to_string()))?;
        Ok(if running {
            ServiceStatus::Running
        } else {
            ServiceStatus::Stopped
        })
    }

    async fn reindex(&self, repo: Option<&str>) -> Result<(), ServiceError> {
        self.inner
            .reindex(repo)
            .await
            .map_err(|e| ServiceError::Process(e.to_string()))
    }
}
