use super::super::model::{EvidenceId, ExecutableTest, ProfileStatus, TestProfileResult};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static CONTEXT_VALIDATOR: OnceLock<std::result::Result<jsonschema::Validator, String>> =
    OnceLock::new();

pub(super) fn validate_snapshot_json(snapshot: &serde_json::Value) -> Result<()> {
    let validator = CONTEXT_VALIDATOR
        .get_or_init(build_context_validator)
        .as_ref()
        .map_err(|message| anyhow::anyhow!("{message}"))?;
    if validator.is_valid(snapshot) {
        return Ok(());
    }
    let mut errors = validator
        .iter_errors(snapshot)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    errors.sort();
    bail!(
        "context snapshot does not match v3 JSON schema: {}",
        errors.join("; ")
    )
}

fn build_context_validator() -> std::result::Result<jsonschema::Validator, String> {
    let cli_schema: serde_json::Value =
        serde_json::from_str(include_str!("../../schema/cli.schema.json"))
            .map_err(|error| format!("parse embedded CLI schema: {error}"))?;
    let defs_key = "\u{24}defs";
    let schema_key = "\u{24}schema";
    let ref_key = "\u{24}ref";
    let mut context_schema = serde_json::Map::new();
    context_schema.insert(
        schema_key.to_string(),
        cli_schema
            .get(schema_key)
            .cloned()
            .ok_or_else(|| "embedded CLI schema is missing its draft".to_string())?,
    );
    context_schema.insert(
        ref_key.to_string(),
        serde_json::Value::String(format!("#/{defs_key}/contextSnapshot")),
    );
    context_schema.insert(
        defs_key.to_string(),
        cli_schema
            .get(defs_key)
            .cloned()
            .ok_or_else(|| "embedded CLI schema is missing definitions".to_string())?,
    );
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&serde_json::Value::Object(context_schema))
        .map_err(|error| format!("compile embedded context schema: {error}"))
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct ProfileEvidenceCacheKey {
    pub(super) commit: String,
    pub(super) worktree_fingerprint: String,
    pub(super) manifest_sha256: String,
    pub(super) target: String,
    pub(super) features: Vec<String>,
    pub(super) no_default_features: bool,
    pub(super) cargo_version: String,
    pub(super) rustc_version: String,
    pub(super) nextest_version: String,
}

impl ProfileEvidenceCacheKey {
    fn normalized(&self) -> Self {
        let mut normalized = self.clone();
        normalized.features.sort();
        normalized.features.dedup();
        normalized
    }
}

pub(super) fn profile_evidence_cache_key(key: &ProfileEvidenceCacheKey) -> String {
    let canonical = serde_json::to_vec(&key.normalized())
        .expect("serializing a profile evidence cache key cannot fail");
    hex::encode(Sha256::digest(canonical))
}

#[derive(Deserialize)]
struct ProfileEvidenceArtifact {
    schema_version: u32,
    profile_id: String,
    cache_key: ProfileEvidenceCacheKey,
    status: ArtifactProfileStatus,
    executable_tests: Vec<ArtifactExecutableTest>,
    unavailable_reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactProfileStatus {
    Validated,
    Failed,
    Unavailable,
}

#[derive(Deserialize)]
struct ArtifactExecutableTest {
    stable_id: String,
    package_id: String,
    binary_id: String,
    test_name: String,
    ignored: bool,
}

pub(super) fn read_profile_evidence(
    evidence_dir: &Path,
    expected_keys: &BTreeMap<String, ProfileEvidenceCacheKey>,
) -> Result<Vec<TestProfileResult>> {
    let mut paths = std::fs::read_dir(evidence_dir)
        .with_context(|| format!("read profile evidence directory {}", evidence_dir.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .with_context(|| format!("read entry in {}", evidence_dir.display()))
        })
        .collect::<Result<Vec<PathBuf>>>()?;
    paths.retain(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"));
    paths.sort();

    let mut imported = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read profile evidence {}", path.display()))?;
        let artifact: ProfileEvidenceArtifact = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse profile evidence {}", path.display()))?;
        if artifact.schema_version != 1 {
            bail!(
                "unsupported profile evidence schema version {} in {}",
                artifact.schema_version,
                path.display()
            );
        }
        let expected = expected_keys.get(&artifact.profile_id).with_context(|| {
            format!(
                "profile evidence {} names unconfigured profile {:?}",
                path.display(),
                artifact.profile_id
            )
        })?;
        let artifact_key = artifact.cache_key.normalized();
        let expected_key = expected.normalized();
        let current = artifact_key == expected_key;
        let mut executable_tests = artifact
            .executable_tests
            .into_iter()
            .map(|test| ExecutableTest {
                stable_id: test.stable_id,
                package_id: test.package_id,
                binary_id: test.binary_id,
                test_name: test.test_name,
                ignored: test.ignored,
            })
            .collect::<Vec<_>>();
        executable_tests.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));

        let status = if current {
            match artifact.status {
                ArtifactProfileStatus::Validated => ProfileStatus::Validated,
                ArtifactProfileStatus::Failed => ProfileStatus::Failed,
                ArtifactProfileStatus::Unavailable => ProfileStatus::Unavailable,
            }
        } else {
            executable_tests.clear();
            ProfileStatus::Stale
        };
        if status != ProfileStatus::Validated && !executable_tests.is_empty() {
            bail!(
                "non-validated profile evidence contains executable tests: {}",
                path.display()
            );
        }
        let unavailable_reason = if current {
            artifact.unavailable_reason
        } else {
            Some(format!(
                "cache key mismatch for {}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("profile evidence")
            ))
        };
        imported.push(TestProfileResult {
            profile_id: artifact.profile_id,
            target: artifact_key.target,
            features: artifact_key.features,
            status,
            executable_tests,
            unavailable_reason,
            evidence_ids: vec![EvidenceId::from("profile-evidence:import")],
        });
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(declared: serde_json::Value, observed: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "declared": declared,
            "observed": observed,
            "validation": { "state": "match", "reason": null },
            "evidence_ids": ["fixture"]
        })
    }

    fn complete_v3_fixture() -> serde_json::Value {
        serde_json::json!({
            "snapshot_version": 3,
            "identity": {
                "commit": "abc123",
                "branch": "develop",
                "dirty": false,
                "changed_paths": [],
                "worktree_fingerprint": "fingerprint",
                "evidence_ids": ["git:head"]
            },
            "environment": {
                "host": "aarch64-apple-darwin",
                "target": "aarch64-apple-darwin",
                "enabled_features": [],
                "rustc_version": fact(serde_json::Value::Null, serde_json::json!("rustc 1.85.0")),
                "cargo_version": fact(serde_json::Value::Null, serde_json::json!("cargo 1.85.0")),
                "nextest_version": fact(serde_json::Value::Null, serde_json::json!("cargo-nextest 0.9")),
                "generated_at": "2026-09-12T00:00:00Z"
            },
            "workspace": {
                "version": fact(serde_json::json!("0.1.0"), serde_json::json!("0.1.0")),
                "edition": fact(serde_json::json!("2024"), serde_json::json!("2024")),
                "msrv": fact(serde_json::json!("1.85"), serde_json::json!("1.85")),
                "packages": [{
                    "package_id": "path+file:///workspace/pkg#0.1.0",
                    "name": "pkg",
                    "manifest_path": "crates/pkg/Cargo.toml",
                    "targets": [{
                        "name": "pkg",
                        "kinds": ["lib"],
                        "source_path": "crates/pkg/src/lib.rs",
                        "required_features": []
                    }],
                    "features": ["default"],
                    "dependencies": [{
                        "package_name": "serde",
                        "rename": null,
                        "kind": "normal",
                        "target_predicate": null,
                        "optional": false
                    }],
                    "metrics": {
                        "rust_files": 1,
                        "physical_lines": 10,
                        "non_empty_lines": 8,
                        "included_roots": ["crates/pkg/src"],
                        "includes_generated": false
                    }
                }]
            },
            "adapters": [{
                "id": "native",
                "maturity": fact(serde_json::json!("production"), serde_json::Value::Null),
                "platforms": fact(serde_json::json!(["linux"]), serde_json::json!(["linux"])),
                "default_roles": fact(serde_json::json!(["linux_fallback"]), serde_json::json!(["linux_fallback"])),
                "registry_presence": fact(serde_json::json!(true), serde_json::json!(true)),
                "capabilities": {
                    "run": fact(serde_json::json!("yes"), serde_json::json!("yes"))
                }
            }],
            "tests": {
                "source_declarations": [{
                    "stable_id": "source-test",
                    "package_id": "path+file:///workspace/pkg#0.1.0",
                    "path": "crates/pkg/src/lib.rs",
                    "module_path": ["tests"],
                    "name": "works",
                    "cfg_predicates": ["cfg(test)"],
                    "kind": "function"
                }],
                "profiles": [{
                    "profile_id": "native-macos",
                    "target": "aarch64-apple-darwin",
                    "features": [],
                    "status": "validated",
                    "executable_tests": [{
                        "stable_id": "exec-test",
                        "package_id": "path+file:///workspace/pkg#0.1.0",
                        "binary_id": "pkg::lib",
                        "test_name": "works",
                        "ignored": false
                    }],
                    "unavailable_reason": null,
                    "evidence_ids": ["nextest:native"]
                }],
                "validated_unique_tests": [{
                    "stable_id": "exec-test",
                    "package_id": "path+file:///workspace/pkg#0.1.0",
                    "binary_id": "pkg::lib",
                    "test_name": "works",
                    "ignored": false
                }]
            },
            "context_map": {
                "crates": [{
                    "package_id": "path+file:///workspace/pkg#0.1.0",
                    "manifest_path": "crates/pkg/Cargo.toml",
                    "source_roots": ["crates/pkg/src"],
                    "evidence_ids": ["cargo:metadata"]
                }],
                "files": [{
                    "path": "crates/pkg/src/lib.rs",
                    "owner_package_id": "path+file:///workspace/pkg#0.1.0",
                    "role": "cargo_target",
                    "role_origin": "cargo_metadata",
                    "evidence_ids": ["cargo:metadata"]
                }],
                "changed_files": [],
                "collector_tasks": [{
                    "id": "metadata",
                    "depends_on": [],
                    "status": "collected",
                    "evidence_ids": ["cargo:metadata"]
                }]
            },
            "evidence": {
                "fixture": {
                    "kind": "cargo_metadata",
                    "locator": "cargo metadata --format-version 1",
                    "collected_at": "2026-09-12T00:00:00Z",
                    "content_sha256": null,
                    "profile_id": null,
                    "status": "collected"
                }
            },
            "diagnostics": [{
                "code": "fixture.info",
                "severity": "info",
                "message": "fixture",
                "profile_id": null,
                "evidence_ids": ["fixture"]
            }]
        })
    }

    #[test]
    fn complete_v3_fixture_matches_json_schema() {
        let fixture = complete_v3_fixture();
        validate_snapshot_json(&fixture).expect("complete v3 fixture should validate");

        for section in [
            "snapshot_version",
            "identity",
            "environment",
            "workspace",
            "adapters",
            "tests",
            "context_map",
            "evidence",
            "diagnostics",
        ] {
            let mut missing = fixture.clone();
            missing
                .as_object_mut()
                .expect("fixture should be an object")
                .remove(section);
            assert!(
                validate_snapshot_json(&missing).is_err(),
                "missing section {section:?} should be rejected"
            );
        }

        for legacy in [
            "commit",
            "branch",
            "timestamp",
            "crates",
            "ci_workflows",
            "recent_commits",
        ] {
            let mut value = fixture.clone();
            value
                .as_object_mut()
                .expect("fixture should be an object")
                .insert(legacy.to_string(), serde_json::Value::Null);
            assert!(
                validate_snapshot_json(&value).is_err(),
                "legacy field {legacy:?} should be rejected"
            );
        }

        for legacy in ["rust_version", "deps", "test_count", "total"] {
            let mut value = fixture.clone();
            let object = match legacy {
                "rust_version" => value["workspace"].as_object_mut(),
                "deps" | "test_count" => value["workspace"]["packages"][0].as_object_mut(),
                "total" => value["tests"].as_object_mut(),
                _ => None,
            }
            .expect("legacy target should be an object");
            object.insert(legacy.to_string(), serde_json::Value::Null);
            assert!(
                validate_snapshot_json(&value).is_err(),
                "nested legacy field {legacy:?} should be rejected"
            );
        }
        for legacy in ["crate_assignments", "file_assignments", "task_slices"] {
            let mut value = fixture.clone();
            value["context_map"]
                .as_object_mut()
                .expect("context map should be an object")
                .insert(legacy.to_string(), serde_json::json!([]));
            assert!(validate_snapshot_json(&value).is_err());
        }

        let mut unknown_enum = fixture.clone();
        unknown_enum["adapters"][0]["maturity"]["declared"] = serde_json::json!("future");
        assert!(validate_snapshot_json(&unknown_enum).is_err());
        let mut unknown_role = fixture.clone();
        unknown_role["context_map"]["files"][0]["role"] = serde_json::json!("future");
        assert!(validate_snapshot_json(&unknown_role).is_err());

        let mut unknown_property = fixture;
        unknown_property["identity"]
            .as_object_mut()
            .expect("identity should be an object")
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(validate_snapshot_json(&unknown_property).is_err());
    }
}
