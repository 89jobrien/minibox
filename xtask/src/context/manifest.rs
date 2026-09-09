use super::model::{AdapterMaturity, CapabilitySupport};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct ContextManifest {
    pub(super) schema_version: u32,
    pub(super) adapters: Vec<AdapterDeclaration>,
    pub(super) profiles: Vec<ValidationProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct AdapterDeclaration {
    pub(super) id: String,
    pub(super) maturity: AdapterMaturity,
    pub(super) platforms: Vec<String>,
    pub(super) default_roles: Vec<String>,
    pub(super) capabilities: BTreeMap<String, CapabilitySupport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct ValidationProfile {
    pub(super) id: String,
    pub(super) target: String,
    pub(super) features: Vec<String>,
    pub(super) no_default_features: bool,
    pub(super) all_targets: bool,
    pub(super) required_in_ci: bool,
}

pub(super) fn load_manifest(root: &Path) -> Result<ContextManifest> {
    let path = root.join("xtask/context.toml");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading xtask/context.toml at {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("parsing xtask/context.toml at {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MANIFEST: &str = r#"
schema_version = 1

[[adapters]]
id = "native"
maturity = "production"
platforms = ["linux"]
default_roles = []

[adapters.capabilities]
run = "yes"

[[profiles]]
id = "native-linux"
target = "x86_64-unknown-linux-gnu"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
"#;

    #[test]
    fn manifest_loader_reports_path_and_parse_errors() {
        let valid_root = tempfile::tempdir().expect("valid fixture root should be created");
        std::fs::create_dir(valid_root.path().join("xtask"))
            .expect("fixture xtask directory should be created");
        std::fs::write(valid_root.path().join("xtask/context.toml"), VALID_MANIFEST)
            .expect("valid manifest fixture should be written");

        let manifest = load_manifest(valid_root.path()).expect("valid manifest should load");
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.adapters.len(), 1);
        assert_eq!(manifest.adapters[0].id, "native");
        assert_eq!(manifest.adapters[0].maturity, AdapterMaturity::Production);
        assert_eq!(
            manifest.adapters[0].capabilities["run"],
            CapabilitySupport::Yes
        );
        assert_eq!(manifest.profiles.len(), 1);
        assert_eq!(manifest.profiles[0].id, "native-linux");
        assert!(manifest.profiles[0].all_targets);

        let missing_root = tempfile::tempdir().expect("missing fixture root should be created");
        let missing_error = load_manifest(missing_root.path())
            .expect_err("missing manifest should return an error")
            .to_string();
        assert!(
            missing_error.contains("xtask/context.toml"),
            "{missing_error}"
        );

        let invalid_root = tempfile::tempdir().expect("invalid fixture root should be created");
        std::fs::create_dir(invalid_root.path().join("xtask"))
            .expect("fixture xtask directory should be created");
        std::fs::write(
            invalid_root.path().join("xtask/context.toml"),
            r#"schema_version = "invalid""#,
        )
        .expect("invalid manifest fixture should be written");
        let invalid_error = load_manifest(invalid_root.path())
            .expect_err("invalid manifest should return an error")
            .to_string();
        assert!(
            invalid_error.contains("xtask/context.toml"),
            "{invalid_error}"
        );
    }
}
