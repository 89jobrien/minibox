//! Typed configuration loaded from `xtask/xconfig.toml`.
//!
//! All fields must be present for deserialization. Some fields are not yet
//! wired into consuming modules — they exist so the config schema is stable
//! and ready when those modules adopt xconfig.

#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct XConfig {
    pub models: Models,
    pub vm: Vm,
    pub cross: Cross,
    pub dotenv: Dotenv,
    pub binaries: Binaries,
    pub orchestration: Orchestration,
}

#[derive(Debug, Deserialize)]
pub struct Models {
    pub default: String,
    pub prod: String,
}

#[derive(Debug, Deserialize)]
pub struct Vm {
    pub name: String,
    pub setup_smolfile: String,
    pub ci_gate_smolfile: String,
}

#[derive(Debug, Deserialize)]
pub struct Cross {
    pub target: String,
}

#[derive(Debug, Deserialize)]
pub struct Dotenv {
    pub env_file: String,
}

#[derive(Debug, Deserialize)]
pub struct Binaries {
    pub agentbox_dir: String,
    pub agentbox_pkg: String,
}

#[derive(Debug, Deserialize)]
pub struct Orchestration {
    pub allowed_tools: String,
}

impl XConfig {
    /// Load from `<workspace_root>/xtask/xconfig.toml`.
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let path = workspace_root.join("xtask/xconfig.toml");
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&content).context("parsing xtask/xconfig.toml")
    }
}

impl Dotenv {
    /// Resolve `$HOME` in the `env_file` path.
    pub fn resolved_env_file(&self) -> String {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        self.env_file.replace("$HOME", &home)
    }
}
