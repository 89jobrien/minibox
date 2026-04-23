use crate::domain::SourceType;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct SearchboxConfig {
    pub service: ServiceConfig,
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
    #[serde(default)]
    pub local: LocalConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServiceConfig {
    pub vps_host: String,
    #[serde(default = "default_zoekt_port")]
    pub zoekt_port: u16,
    /// Cron expression for scheduled reindex. Empty = manual only.
    #[serde(default)]
    pub index_schedule: String,
}

fn default_zoekt_port() -> u16 {
    6070
}

#[derive(Debug, Deserialize)]
pub struct RepoConfig {
    pub name: String,
    /// Remote git URL (for source = "git") or local path (for source = "fs"/"local").
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    pub source: SourceType,
}

#[derive(Debug, Default, Deserialize)]
pub struct LocalConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_local_port")]
    pub port: u16,
    #[serde(default)]
    pub repos: Vec<String>,
}

fn default_local_port() -> u16 {
    6071
}

impl SearchboxConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let cfg: Self = toml::from_str(&text).context("parse config TOML")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn load_default() -> Result<Self> {
        let path = config_path();
        Self::load(&path)
    }

    pub fn validate_pub(&self) -> Result<()> {
        self.validate()
    }

    fn validate(&self) -> Result<()> {
        for repo in &self.repos {
            match repo.source {
                SourceType::Git => {
                    if repo.url.is_none() {
                        bail!(
                            "repo `{}`: source = \"git\" requires `url` field",
                            repo.name
                        );
                    }
                }
                SourceType::Filesystem | SourceType::Local => {
                    if repo.path.is_none() {
                        bail!(
                            "repo `{}`: source = \"{}\" requires `path` field",
                            repo.name,
                            match repo.source {
                                SourceType::Filesystem => "fs",
                                _ => "local",
                            }
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("SEARCHBOX_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("searchbox")
        .join("config.toml")
}
