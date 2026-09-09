use super::model::{AdapterMaturity, CapabilitySupport};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
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

pub(super) fn validate_manifest(manifest: &ContextManifest) -> Result<()> {
    let mut diagnostics = Vec::new();

    collect_duplicate_ids(
        manifest.adapters.iter().map(|adapter| adapter.id.as_str()),
        "adapter",
        &mut diagnostics,
    );
    collect_duplicate_ids(
        manifest.profiles.iter().map(|profile| profile.id.as_str()),
        "profile",
        &mut diagnostics,
    );

    let mut profile_keys = BTreeSet::new();
    for profile in &manifest.profiles {
        let mut features = profile.features.clone();
        features.sort();
        let key = (profile.target.clone(), features);
        if !profile_keys.insert(key) {
            diagnostics.push(format!(
                "duplicate target/feature profile: {}",
                profile.target
            ));
        }
    }

    validate_capability_keys(&manifest.adapters, &mut diagnostics);

    for target in [
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "x86_64-pc-windows-msvc",
    ] {
        let has_native_profile = manifest.profiles.iter().any(|profile| {
            profile.target == target && profile.features.is_empty() && !profile.no_default_features
        });
        if !has_native_profile {
            diagnostics.push(format!("missing native profile: {target}"));
        }
    }

    let adapter_ids: BTreeSet<&str> = manifest
        .adapters
        .iter()
        .map(|adapter| adapter.id.as_str())
        .collect();
    for adapter in &manifest.adapters {
        for role in &adapter.default_roles {
            let expected_adapter = match role.as_str() {
                "unix_default" => Some("smolvm"),
                "linux_fallback" => Some("native"),
                "macos_fallback" => Some("krun"),
                _ => None,
            };
            if let Some(expected_adapter) = expected_adapter {
                if !adapter_ids.contains(expected_adapter) {
                    diagnostics.push(format!(
                        "default role {role} references undeclared adapter: {expected_adapter}"
                    ));
                } else if adapter.id != expected_adapter {
                    diagnostics.push(format!(
                        "default role {role} must reference adapter: {expected_adapter}"
                    ));
                }
            }
        }
    }

    diagnostics.sort();
    diagnostics.dedup();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(diagnostics.join("\n"))
    }
}

fn collect_duplicate_ids<'a>(
    ids: impl Iterator<Item = &'a str>,
    kind: &str,
    diagnostics: &mut Vec<String>,
) {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            diagnostics.push(format!("duplicate {kind} id: {id}"));
        }
    }
}

fn validate_capability_keys(adapters: &[AdapterDeclaration], diagnostics: &mut Vec<String>) {
    let mut frequencies = BTreeMap::<Vec<&str>, usize>::new();
    for adapter in adapters {
        let keys = adapter
            .capabilities
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        *frequencies.entry(keys).or_default() += 1;
    }
    let canonical = frequencies
        .into_iter()
        .max_by(|(left_keys, left_count), (right_keys, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_keys.cmp(left_keys))
        })
        .map(|(keys, _)| keys);

    if let Some(canonical) = canonical {
        for adapter in adapters {
            let keys = adapter
                .capabilities
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            if keys != canonical {
                diagnostics.push(format!(
                    "capability keys differ for adapter: {}",
                    adapter.id
                ));
            }
        }
    }
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
    fn validation_fixture() -> ContextManifest {
        let capabilities = BTreeMap::from([
            ("pull".to_string(), CapabilitySupport::Yes),
            ("run".to_string(), CapabilitySupport::Yes),
        ]);
        let adapter = |id: &str| AdapterDeclaration {
            id: id.to_string(),
            maturity: AdapterMaturity::Production,
            platforms: vec!["any".to_string()],
            default_roles: Vec::new(),
            capabilities: capabilities.clone(),
        };
        let profile = |id: &str, target: &str| ValidationProfile {
            id: id.to_string(),
            target: target.to_string(),
            features: Vec::new(),
            no_default_features: false,
            all_targets: true,
            required_in_ci: true,
        };

        ContextManifest {
            schema_version: 1,
            adapters: vec![adapter("krun"), adapter("native"), adapter("smolvm")],
            profiles: vec![
                profile("native-macos", "aarch64-apple-darwin"),
                profile("native-linux-gnu", "x86_64-unknown-linux-gnu"),
                profile("native-linux-musl", "x86_64-unknown-linux-musl"),
                profile("native-windows", "x86_64-pc-windows-msvc"),
            ],
        }
    }

    fn assert_manifest_invalid(manifest: &ContextManifest, expected: &str) {
        let error = validate_manifest(manifest)
            .expect_err("ambiguous manifest should be rejected")
            .to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?} in {error:?}"
        );
    }

    #[test]
    fn manifest_rejects_ambiguous_declarations() {
        validate_manifest(&validation_fixture()).expect("empty default roles should remain valid");

        let mut duplicate_adapter = validation_fixture();
        duplicate_adapter
            .adapters
            .push(duplicate_adapter.adapters[0].clone());
        assert_manifest_invalid(&duplicate_adapter, "duplicate adapter id: krun");

        let mut duplicate_profile_id = validation_fixture();
        let mut duplicate = duplicate_profile_id.profiles[0].clone();
        duplicate.target = "aarch64-unknown-linux-gnu".to_string();
        duplicate_profile_id.profiles.push(duplicate);
        assert_manifest_invalid(&duplicate_profile_id, "duplicate profile id: native-macos");

        let mut duplicate_profile_key = validation_fixture();
        let mut duplicate = duplicate_profile_key.profiles[0].clone();
        duplicate.id = "same-target-and-features".to_string();
        duplicate_profile_key.profiles.push(duplicate);
        assert_manifest_invalid(
            &duplicate_profile_key,
            "duplicate target/feature profile: aarch64-apple-darwin",
        );

        let mut unequal_capabilities = validation_fixture();
        unequal_capabilities.adapters[0].capabilities.remove("pull");
        assert_manifest_invalid(
            &unequal_capabilities,
            "capability keys differ for adapter: krun",
        );

        let mut missing_native_profile = validation_fixture();
        missing_native_profile
            .profiles
            .retain(|profile| profile.target != "x86_64-pc-windows-msvc");
        assert_manifest_invalid(
            &missing_native_profile,
            "missing native profile: x86_64-pc-windows-msvc",
        );

        let mut unresolved_default_role = validation_fixture();
        unresolved_default_role
            .adapters
            .retain(|adapter| adapter.id != "smolvm");
        unresolved_default_role.adapters[0].default_roles = vec!["unix_default".to_string()];
        assert_manifest_invalid(
            &unresolved_default_role,
            "default role unix_default references undeclared adapter: smolvm",
        );
    }
}
