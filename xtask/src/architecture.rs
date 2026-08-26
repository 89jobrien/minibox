//! Enforce workspace dependency rings and canonical source ownership.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const ALLOWED_DOMAIN_DEPENDENCIES: &[&str] = &[
    "anyhow",
    "async-trait",
    "hex",
    "serde",
    "serde_json",
    "sha2",
    "slashcrux",
    "thiserror",
];

const FORBIDDEN_SHADOW_PATHS: &[&str] = &[
    "crates/minibox/src/preflight.rs",
    "crates/minibox/src/domain/networking.rs",
    "crates/minibox/src/domain/extensions.rs",
    "crates/minibox/src/adapters/registry.rs",
];

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    dependencies: Vec<Dependency>,
}

#[derive(Debug, Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
}

/// Validate the current workspace architecture.
// qual:allow(iosp) reason: "xtask boundary runs cargo metadata and validates filesystem ownership"
pub fn run(root: &Path) -> Result<()> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .context("run cargo metadata for architecture check")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).context("parse cargo metadata")?;
    validate_dependency_rings(&metadata)?;
    validate_canonical_paths(root)?;
    validate_error_ownership(root)?;
    validate_state_ownership(root)?;
    eprintln!("architecture: dependency rings and canonical owners are valid");
    Ok(())
}

fn ring(name: &str) -> Option<u8> {
    match name {
        "minibox-domain" => Some(0),
        "minibox-core" => Some(1),
        "minibox" => Some(2),
        _ => None,
    }
}

fn validate_dependency_rings(metadata: &Metadata) -> Result<()> {
    let workspace: HashMap<&str, &Package> = metadata
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    for package in &metadata.packages {
        let Some(package_ring) = ring(&package.name) else {
            continue;
        };
        for dependency in &package.dependencies {
            if let Some(dependency_ring) = ring(&dependency.name)
                && dependency_ring > package_ring
            {
                bail!(
                    "architecture violation: {} (ring {}) depends outward on {} (ring {})",
                    package.name,
                    package_ring,
                    dependency.name,
                    dependency_ring
                );
            }
        }
    }

    let domain = workspace
        .get("minibox-domain")
        .context("workspace is missing minibox-domain")?;
    let disallowed: Vec<&str> = domain
        .dependencies
        .iter()
        .filter(|dependency| dependency.kind.as_deref() != Some("dev"))
        .map(|dependency| dependency.name.as_str())
        .filter(|name| workspace.contains_key(name) || !ALLOWED_DOMAIN_DEPENDENCIES.contains(name))
        .collect();
    if !disallowed.is_empty() {
        bail!("minibox-domain has non-domain dependencies: {disallowed:?}");
    }

    Ok(())
}

fn validate_canonical_paths(root: &Path) -> Result<()> {
    let existing: Vec<&str> = FORBIDDEN_SHADOW_PATHS
        .iter()
        .copied()
        .filter(|path| root.join(path).exists())
        .collect();
    if !existing.is_empty() {
        bail!("canonical surfaces have shadow implementations: {existing:?}");
    }

    let facade_dir = root.join("crates/minibox-core/src/domain");
    let mut facade_sources = Vec::new();
    collect_rust_sources(&facade_dir, &mut facade_sources)?;
    let extra_facade_sources: Vec<&PathBuf> = facade_sources
        .iter()
        .filter(|path| path.as_path() != facade_dir.join("mod.rs"))
        .collect();
    if !extra_facade_sources.is_empty() {
        bail!("minibox-core domain facade contains implementations: {extra_facade_sources:?}");
    }
    let facade = std::fs::read_to_string(facade_dir.join("mod.rs"))
        .context("read minibox-core domain facade")?;
    let code: Vec<&str> = facade
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect();
    if code != ["pub use minibox_domain::*;"] {
        bail!("minibox-core domain facade must contain only the minibox-domain re-export");
    }
    Ok(())
}

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("read source directory {}", directory.display()))?
    {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_sources(&path, sources)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    Ok(())
}

fn validate_error_ownership(root: &Path) -> Result<()> {
    let runtime_errors = std::fs::read_to_string(root.join("crates/minibox/src/error.rs"))
        .context("read minibox runtime errors")?;
    let duplicate_names: Vec<&str> = ["MiniboxError", "ImageError", "RegistryError"]
        .into_iter()
        .filter(|name| runtime_errors.contains(&format!("pub enum {name}")))
        .collect();
    if !duplicate_names.is_empty() {
        bail!("minibox redeclares core-owned errors: {duplicate_names:?}");
    }
    Ok(())
}

fn validate_state_ownership(root: &Path) -> Result<()> {
    let native_container =
        std::fs::read_to_string(root.join("crates/minibox/src/container/mod.rs"))
            .context("read native container module")?;
    if native_container.contains("pub enum ContainerState") {
        bail!("native container module shadows domain-owned ContainerState");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask manifest must be beneath the workspace root")
            .to_path_buf()
    }

    fn package(name: &str, dependencies: &[&str]) -> Package {
        Package {
            name: name.to_string(),
            dependencies: dependencies
                .iter()
                .map(|name| Dependency {
                    name: (*name).to_string(),
                    kind: None,
                })
                .collect(),
        }
    }

    #[test]
    fn workspace_declares_minibox_domain_ring() {
        let manifest = std::fs::read_to_string(workspace_root().join("Cargo.toml"))
            .expect("workspace manifest must be readable");

        assert!(
            manifest.contains("minibox-domain"),
            "workspace must declare the minibox-domain inner ring"
        );
    }

    #[test]
    fn canonical_surfaces_have_no_minibox_shadows() {
        validate_canonical_paths(&workspace_root())
            .expect("canonical core and domain surfaces must not have shadows");
    }

    #[test]
    fn docker_hub_registry_has_one_canonical_owner() {
        let duplicate = workspace_root().join("crates/minibox/src/adapters/registry.rs");
        assert!(
            !duplicate.exists(),
            "DockerHubRegistry must be owned only by minibox-core"
        );
    }

    #[test]
    fn shared_errors_have_one_canonical_owner() {
        validate_error_ownership(&workspace_root())
            .expect("minibox must re-export core-owned shared errors");
    }

    #[test]
    fn container_state_has_one_canonical_owner() {
        validate_state_ownership(&workspace_root())
            .expect("native container state must have a distinct name");
    }

    #[test]
    fn dependency_rings_reject_inner_to_outer_edge() {
        let metadata = Metadata {
            packages: vec![
                package("minibox-domain", &["minibox-core"]),
                package("minibox-core", &[]),
                package("minibox", &[]),
            ],
        };

        let error = validate_dependency_rings(&metadata)
            .expect_err("domain-to-core dependency must be rejected");
        assert!(error.to_string().contains("depends outward"));
    }

    #[test]
    fn dependency_rings_accept_inward_edges() {
        let metadata = Metadata {
            packages: vec![
                package("minibox-domain", &["serde"]),
                package("minibox-core", &["minibox-domain"]),
                package("minibox", &["minibox-core", "minibox-domain"]),
            ],
        };

        validate_dependency_rings(&metadata).expect("inward dependencies must be accepted");
    }

    #[test]
    fn domain_rejects_runtime_dependency() {
        let metadata = Metadata {
            packages: vec![
                package("minibox-domain", &["tokio"]),
                package("minibox-core", &[]),
                package("minibox", &[]),
            ],
        };

        let error = validate_dependency_rings(&metadata)
            .expect_err("Tokio dependency in domain must be rejected");
        assert!(error.to_string().contains("non-domain dependencies"));
    }
}
