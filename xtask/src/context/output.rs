use super::super::manifest::ContextManifest;
use super::super::model::{
    AdapterMaturity, CapabilitySupport, EvidenceId, ExecutableTest, ProfileStatus,
    TestProfileResult,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const ADAPTER_SUITES_BEGIN: &str = "<!-- BEGIN GENERATED: adapter-suites -->";
const ADAPTER_SUITES_END: &str = "<!-- END GENERATED: adapter-suites -->";
const ADAPTER_CAPABILITIES_BEGIN: &str = "<!-- BEGIN GENERATED: adapter-capabilities -->";
const ADAPTER_CAPABILITIES_END: &str = "<!-- END GENERATED: adapter-capabilities -->";

pub(in crate::context) fn sync_adapter_matrix(
    document: &str,
    manifest: &ContextManifest,
) -> Result<String> {
    let suites = render_adapter_suites(manifest);
    let capabilities = render_adapter_capabilities(manifest)?;
    let document =
        replace_generated_block(document, ADAPTER_SUITES_BEGIN, ADAPTER_SUITES_END, &suites)?;
    replace_generated_block(
        &document,
        ADAPTER_CAPABILITIES_BEGIN,
        ADAPTER_CAPABILITIES_END,
        &capabilities,
    )
}

fn replace_generated_block(
    document: &str,
    begin: &str,
    end: &str,
    generated: &str,
) -> Result<String> {
    let begins = document
        .match_indices(begin)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let ends = document
        .match_indices(end)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if begins.len() != 1 || ends.len() != 1 {
        bail!("generated block must contain exactly one begin and end marker: {begin:?}, {end:?}");
    }
    let begin_index = begins[0];
    let end_index = ends[0];
    if begin_index >= end_index {
        bail!("generated block markers are out of order: {begin:?}, {end:?}");
    }
    let replacement = format!("{begin}\n{generated}\n{end}");
    Ok(format!(
        "{}{}{}",
        &document[..begin_index],
        replacement,
        &document[end_index + end.len()..]
    ))
}

fn render_adapter_suites(manifest: &ContextManifest) -> String {
    let mut adapters = manifest.adapters.iter().collect::<Vec<_>>();
    adapters.sort_by(|left, right| left.id.cmp(&right.id));
    let mut lines = vec![
        "| Adapter | Platforms | Maturity | Default roles |".to_string(),
        "| --- | --- | --- | --- |".to_string(),
    ];
    for adapter in adapters {
        let mut platforms = adapter.platforms.clone();
        platforms.sort();
        platforms.dedup();
        let platforms = platforms
            .iter()
            .map(|platform| format!("`{platform}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut roles = adapter.default_roles.clone();
        roles.sort();
        roles.dedup();
        let roles = if roles.is_empty() {
            "--".to_string()
        } else {
            roles
                .iter()
                .map(|role| format!("`{role}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(format!(
            "| `{}` | {} | {} | {} |",
            adapter.id,
            platforms,
            maturity_label(&adapter.maturity),
            roles
        ));
    }
    lines.join("\n")
}

fn render_adapter_capabilities(manifest: &ContextManifest) -> Result<String> {
    let mut adapters = manifest.adapters.iter().collect::<Vec<_>>();
    adapters.sort_by(|left, right| left.id.cmp(&right.id));
    let capabilities = adapters
        .iter()
        .flat_map(|adapter| adapter.capabilities.keys().cloned())
        .collect::<BTreeSet<_>>();
    let mut lines = vec![format!(
        "| Capability | {} |",
        adapters
            .iter()
            .map(|adapter| format!("`{}`", adapter.id))
            .collect::<Vec<_>>()
            .join(" | ")
    )];
    lines.push(format!(
        "| --- | {} |",
        adapters
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ")
    ));
    for capability in capabilities {
        let supports = adapters
            .iter()
            .map(|adapter| {
                adapter
                    .capabilities
                    .get(&capability)
                    .map(capability_label)
                    .with_context(|| {
                        format!(
                            "adapter {:?} is missing capability {:?}",
                            adapter.id, capability
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        lines.push(format!("| `{capability}` | {} |", supports.join(" | ")));
    }
    Ok(lines.join("\n"))
}

const fn maturity_label(maturity: &AdapterMaturity) -> &'static str {
    match maturity {
        AdapterMaturity::Production => "Production",
        AdapterMaturity::Experimental => "Experimental",
        AdapterMaturity::Stub => "Stub",
        AdapterMaturity::Blocked => "Blocked",
    }
}

const fn capability_label(capability: &CapabilitySupport) -> &'static str {
    match capability {
        CapabilitySupport::Yes => "Yes",
        CapabilitySupport::Limited => "Limited",
        CapabilitySupport::Blocked => "Blocked",
        CapabilitySupport::No => "No",
    }
}

static CONTEXT_VALIDATOR: OnceLock<std::result::Result<jsonschema::Validator, String>> =
    OnceLock::new();

pub(in crate::context) fn validate_snapshot_json(snapshot: &serde_json::Value) -> Result<()> {
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
pub(in crate::context) struct ProfileEvidenceCacheKey {
    pub(in crate::context) commit: String,
    pub(in crate::context) worktree_fingerprint: String,
    pub(in crate::context) manifest_sha256: String,
    pub(in crate::context) target: String,
    pub(in crate::context) features: Vec<String>,
    pub(in crate::context) no_default_features: bool,
    pub(in crate::context) cargo_version: String,
    pub(in crate::context) rustc_version: String,
    pub(in crate::context) nextest_version: String,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::context) struct PendingProfileEvidence {
    pub(in crate::context) expected_key: ProfileEvidenceCacheKey,
    pub(in crate::context) actual_key: ProfileEvidenceCacheKey,
    pub(in crate::context) result: TestProfileResult,
}

struct PreparedProfileEvidence {
    profile_id: String,
    cache_key: String,
    bytes: Vec<u8>,
}

pub(in crate::context) fn persist_snapshot(
    root: &Path,
    snapshot: &serde_json::Value,
    save: bool,
    profile_evidence: &[PendingProfileEvidence],
) -> Result<Option<PathBuf>> {
    if !save {
        return Ok(None);
    }

    validate_snapshot_json(snapshot)?;
    let mut pretty = serde_json::to_vec_pretty(snapshot).context("serialize context snapshot")?;
    pretty.push(b'\n');
    let compact = serde_json::to_vec(snapshot).context("serialize context history record")?;
    let prepared_evidence = profile_evidence
        .iter()
        .filter_map(prepare_profile_evidence)
        .collect::<Result<Vec<_>>>()?;

    let context_dir = root.join("artifacts/context");
    std::fs::create_dir_all(&context_dir)
        .with_context(|| format!("create context output directory {}", context_dir.display()))?;
    let snapshot_path = context_dir.join("snapshot.json");
    atomic_write(&snapshot_path, &pretty)?;

    let history_path = context_dir.join("history.jsonl");
    let mut history = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&history_path)
        .with_context(|| format!("open context history {}", history_path.display()))?;
    history
        .write_all(&compact)
        .and_then(|()| history.write_all(b"\n"))
        .and_then(|()| history.flush())
        .with_context(|| format!("append context history {}", history_path.display()))?;
    history
        .sync_all()
        .with_context(|| format!("sync context history {}", history_path.display()))?;

    for evidence in prepared_evidence {
        let evidence_dir = context_dir.join("evidence").join(&evidence.profile_id);
        std::fs::create_dir_all(&evidence_dir).with_context(|| {
            format!(
                "create profile evidence directory {}",
                evidence_dir.display()
            )
        })?;
        atomic_write(
            &evidence_dir.join(format!("{}.json", evidence.cache_key)),
            &evidence.bytes,
        )?;
    }
    Ok(Some(snapshot_path))
}

fn prepare_profile_evidence(
    pending: &PendingProfileEvidence,
) -> Option<Result<PreparedProfileEvidence>> {
    if pending.actual_key.normalized() != pending.expected_key.normalized()
        || pending.result.status == ProfileStatus::Stale
    {
        return None;
    }
    Some((|| {
        validate_profile_id(&pending.result.profile_id)?;
        let actual_key = pending.actual_key.normalized();
        let mut result_features = pending.result.features.clone();
        result_features.sort();
        result_features.dedup();
        if pending.result.target != actual_key.target || result_features != actual_key.features {
            bail!(
                "profile result {:?} does not match its current evidence key",
                pending.result.profile_id
            );
        }
        if pending.result.status != ProfileStatus::Validated
            && !pending.result.executable_tests.is_empty()
        {
            bail!(
                "non-validated profile evidence contains executable tests: {:?}",
                pending.result.profile_id
            );
        }
        let status = match pending.result.status {
            ProfileStatus::Validated => ArtifactProfileStatus::Validated,
            ProfileStatus::Failed => ArtifactProfileStatus::Failed,
            ProfileStatus::Unavailable => ArtifactProfileStatus::Unavailable,
            ProfileStatus::Stale => unreachable!("stale evidence is filtered before preparation"),
        };
        let mut executable_tests = pending
            .result
            .executable_tests
            .iter()
            .map(|test| ArtifactExecutableTest {
                stable_id: test.stable_id.clone(),
                package_id: test.package_id.clone(),
                binary_id: test.binary_id.clone(),
                test_name: test.test_name.clone(),
                ignored: test.ignored,
            })
            .collect::<Vec<_>>();
        executable_tests.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
        let artifact = ProfileEvidenceArtifact {
            schema_version: 1,
            profile_id: pending.result.profile_id.clone(),
            cache_key: actual_key,
            status,
            executable_tests,
            unavailable_reason: pending.result.unavailable_reason.clone(),
        };
        let mut bytes =
            serde_json::to_vec_pretty(&artifact).context("serialize current profile evidence")?;
        bytes.push(b'\n');
        Ok(PreparedProfileEvidence {
            profile_id: pending.result.profile_id.clone(),
            cache_key: profile_evidence_cache_key(&pending.actual_key),
            bytes,
        })
    })())
}

fn validate_profile_id(profile_id: &str) -> Result<()> {
    if profile_id.is_empty()
        || profile_id == "."
        || profile_id == ".."
        || !profile_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("profile id is not safe for evidence persistence: {profile_id:?}");
    }
    Ok(())
}

pub(in crate::context) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("output path has no parent: {}", path.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create atomic output beside {}", path.display()))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.flush())
        .with_context(|| format!("write atomic output for {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("sync atomic output for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("replace output atomically at {}", path.display()))?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
struct ProfileEvidenceArtifact {
    schema_version: u32,
    profile_id: String,
    cache_key: ProfileEvidenceCacheKey,
    status: ArtifactProfileStatus,
    executable_tests: Vec<ArtifactExecutableTest>,
    unavailable_reason: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactProfileStatus {
    Validated,
    Failed,
    Unavailable,
}

#[derive(Deserialize, Serialize)]
struct ArtifactExecutableTest {
    stable_id: String,
    package_id: String,
    binary_id: String,
    test_name: String,
    ignored: bool,
}

pub(in crate::context) fn read_profile_evidence(
    evidence_dir: &Path,
    expected_keys: &BTreeMap<String, ProfileEvidenceCacheKey>,
    workspace_packages: &BTreeSet<String>,
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
            .map(|test| {
                if !workspace_packages.contains(&test.package_id) {
                    bail!(
                        "profile evidence {} references unknown workspace package {:?}",
                        path.display(),
                        test.package_id
                    );
                }
                let stable_id = format!(
                    "{}::{}::{}",
                    test.package_id, test.binary_id, test.test_name
                );
                if test.stable_id != stable_id {
                    bail!(
                        "profile evidence {} has unstable test identity {:?}; expected {:?}",
                        path.display(),
                        test.stable_id,
                        stable_id
                    );
                }
                Ok(ExecutableTest {
                    stable_id,
                    package_id: test.package_id,
                    binary_id: test.binary_id,
                    test_name: test.test_name,
                    ignored: test.ignored,
                })
            })
            .collect::<Result<Vec<_>>>()?;
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
    fn evidence_key() -> ProfileEvidenceCacheKey {
        ProfileEvidenceCacheKey {
            commit: "abc123".to_string(),
            worktree_fingerprint: "clean".to_string(),
            manifest_sha256: "manifest".to_string(),
            target: "aarch64-apple-darwin".to_string(),
            features: Vec::new(),
            no_default_features: false,
            cargo_version: "cargo 1.85.0".to_string(),
            rustc_version: "rustc 1.85.0".to_string(),
            nextest_version: "cargo-nextest 0.9".to_string(),
        }
    }

    fn evidence_result(status: ProfileStatus) -> TestProfileResult {
        TestProfileResult {
            profile_id: "native-macos".to_string(),
            target: "aarch64-apple-darwin".to_string(),
            features: Vec::new(),
            status,
            executable_tests: vec![ExecutableTest {
                stable_id: "pkg::bin::works".to_string(),
                package_id: "pkg".to_string(),
                binary_id: "bin".to_string(),
                test_name: "works".to_string(),
                ignored: false,
            }],
            unavailable_reason: None,
            evidence_ids: vec![EvidenceId::from("nextest:native-macos")],
        }
    }

    #[test]
    fn persistence_writes_only_valid_current_evidence() {
        let invalid_root = tempfile::tempdir().expect("invalid root should be created");
        let mut invalid = complete_v3_fixture();
        invalid
            .as_object_mut()
            .expect("fixture should be an object")
            .remove("identity");
        assert!(persist_snapshot(invalid_root.path(), &invalid, true, &[]).is_err());
        assert!(!invalid_root.path().join("artifacts").exists());

        let readonly_root = tempfile::tempdir().expect("read-only root should be created");
        let key = evidence_key();
        let current = PendingProfileEvidence {
            expected_key: key.clone(),
            actual_key: key.clone(),
            result: evidence_result(ProfileStatus::Validated),
        };
        assert_eq!(
            persist_snapshot(
                readonly_root.path(),
                &complete_v3_fixture(),
                false,
                std::slice::from_ref(&current),
            )
            .expect("read-only persistence should be a no-op"),
            None
        );
        assert!(!readonly_root.path().join("artifacts").exists());

        let root = tempfile::tempdir().expect("persistence root should be created");
        let context_dir = root.path().join("artifacts/context");
        std::fs::create_dir_all(&context_dir)
            .expect("existing context directory should be created");
        std::fs::write(context_dir.join("snapshot.json"), b"old snapshot\n")
            .expect("old snapshot should be written");
        let mut stale_key = key.clone();
        stale_key.commit = "different".to_string();
        let stale = PendingProfileEvidence {
            expected_key: key.clone(),
            actual_key: stale_key,
            result: evidence_result(ProfileStatus::Validated),
        };
        let stale_status = PendingProfileEvidence {
            expected_key: key.clone(),
            actual_key: key.clone(),
            result: evidence_result(ProfileStatus::Stale),
        };
        let fixture = complete_v3_fixture();
        let saved = persist_snapshot(root.path(), &fixture, true, &[current, stale, stale_status])
            .expect("valid snapshot should persist");
        assert_eq!(saved, Some(context_dir.join("snapshot.json")));

        let snapshot = std::fs::read_to_string(context_dir.join("snapshot.json"))
            .expect("snapshot should be readable");
        assert_eq!(
            snapshot,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&fixture).expect("fixture should serialize")
            )
        );
        let history = std::fs::read_to_string(context_dir.join("history.jsonl"))
            .expect("history should be readable");
        let lines = history.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        let history_value: serde_json::Value =
            serde_json::from_str(lines[0]).expect("history line should be JSON");
        assert_eq!(history_value["snapshot_version"], 3);

        let evidence_path = context_dir
            .join("evidence/native-macos")
            .join(format!("{}.json", profile_evidence_cache_key(&key)));
        assert!(evidence_path.is_file());
        let evidence_files = std::fs::read_dir(context_dir.join("evidence/native-macos"))
            .expect("evidence directory should be readable")
            .collect::<std::io::Result<Vec<_>>>()
            .expect("evidence entries should be readable");
        assert_eq!(evidence_files.len(), 1);
        assert!(
            std::fs::read_dir(&context_dir)
                .expect("context directory should be readable")
                .all(|entry| !entry
                    .expect("context entry should be readable")
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp"))
        );
    }

    #[test]
    fn evidence_rejections_do_not_create_partial_outputs() {
        let root = tempfile::tempdir().expect("persistence root should be created");
        let key = evidence_key();
        let mut invalid = evidence_result(ProfileStatus::Validated);
        invalid.profile_id = "../escape".to_string();
        let pending = PendingProfileEvidence {
            expected_key: key.clone(),
            actual_key: key.clone(),
            result: invalid,
        };
        assert!(persist_snapshot(root.path(), &complete_v3_fixture(), true, &[pending]).is_err());
        assert!(!root.path().join("artifacts").exists());

        for (status, target) in [
            (ProfileStatus::Validated, "different-target"),
            (ProfileStatus::Failed, "aarch64-apple-darwin"),
        ] {
            let root = tempfile::tempdir().expect("persistence root should be created");
            let mut result = evidence_result(status);
            result.target = target.to_string();
            let pending = PendingProfileEvidence {
                expected_key: key.clone(),
                actual_key: key.clone(),
                result,
            };
            assert!(
                persist_snapshot(root.path(), &complete_v3_fixture(), true, &[pending]).is_err()
            );
            assert!(!root.path().join("artifacts").exists());
        }
    }

    #[test]
    fn imported_evidence_rejects_unknown_workspace_package() {
        let directory = tempfile::tempdir().expect("evidence directory should be created");
        let key = evidence_key();
        let artifact = ProfileEvidenceArtifact {
            schema_version: 1,
            profile_id: "native-macos".to_string(),
            cache_key: key.clone(),
            status: ArtifactProfileStatus::Validated,
            executable_tests: vec![ArtifactExecutableTest {
                stable_id: "foreign::bin::works".to_string(),
                package_id: "foreign".to_string(),
                binary_id: "bin".to_string(),
                test_name: "works".to_string(),
                ignored: false,
            }],
            unavailable_reason: None,
        };
        std::fs::write(
            directory.path().join("evidence.json"),
            serde_json::to_vec(&artifact).expect("artifact should serialize"),
        )
        .expect("artifact should be written");
        let expected = BTreeMap::from([("native-macos".to_string(), key)]);
        let error = read_profile_evidence(
            directory.path(),
            &expected,
            &BTreeSet::from(["pkg".to_string()]),
        )
        .expect_err("foreign package identities must be rejected");
        assert!(error.to_string().contains("unknown workspace package"));
    }
    #[test]
    fn adapter_matrix_sync_is_scoped_and_idempotent() {
        use crate::context::manifest::{AdapterDeclaration, ContextManifest};
        use crate::context::model::{AdapterMaturity, CapabilitySupport};

        let manifest = ContextManifest {
            schema_version: 1,
            adapters: vec![
                AdapterDeclaration {
                    id: "zeta".to_string(),
                    maturity: AdapterMaturity::Blocked,
                    platforms: vec!["windows".to_string()],
                    default_roles: Vec::new(),
                    capabilities: BTreeMap::from([
                        ("run".to_string(), CapabilitySupport::Limited),
                        ("build".to_string(), CapabilitySupport::Blocked),
                    ]),
                },
                AdapterDeclaration {
                    id: "alpha".to_string(),
                    maturity: AdapterMaturity::Production,
                    platforms: vec!["macos".to_string(), "linux".to_string()],
                    default_roles: vec!["unix_default".to_string()],
                    capabilities: BTreeMap::from([
                        ("run".to_string(), CapabilitySupport::Yes),
                        ("build".to_string(), CapabilitySupport::No),
                    ]),
                },
            ],
            profiles: Vec::new(),
        };
        let document = "before prose\n\n<!-- BEGIN GENERATED: adapter-suites -->\nold suites\n<!-- END GENERATED: adapter-suites -->\n\nmiddle prose\n\n<!-- BEGIN GENERATED: adapter-capabilities -->\nold capabilities\n<!-- END GENERATED: adapter-capabilities -->\n\nafter prose\n";
        let expected = "before prose\n\n<!-- BEGIN GENERATED: adapter-suites -->\n| Adapter | Platforms | Maturity | Default roles |\n| --- | --- | --- | --- |\n| `alpha` | `linux`, `macos` | Production | `unix_default` |\n| `zeta` | `windows` | Blocked | -- |\n<!-- END GENERATED: adapter-suites -->\n\nmiddle prose\n\n<!-- BEGIN GENERATED: adapter-capabilities -->\n| Capability | `alpha` | `zeta` |\n| --- | --- | --- |\n| `build` | No | Blocked |\n| `run` | Yes | Limited |\n<!-- END GENERATED: adapter-capabilities -->\n\nafter prose\n";

        let synced = sync_adapter_matrix(document, &manifest)
            .expect("marked adapter blocks should synchronize");
        assert_eq!(synced, expected);
        assert_eq!(
            sync_adapter_matrix(&synced, &manifest).expect("second synchronization should succeed"),
            synced
        );
        assert!(synced.starts_with("before prose\n\n"));
        assert!(synced.ends_with("\nafter prose\n"));
        assert!(synced.contains("\n\nmiddle prose\n\n"));

        assert!(sync_adapter_matrix("no generated markers\n", &manifest).is_err());
        let duplicate = format!(
            "{document}<!-- BEGIN GENERATED: adapter-suites -->\nduplicate\n<!-- END GENERATED: adapter-suites -->\n"
        );
        assert!(sync_adapter_matrix(&duplicate, &manifest).is_err());
    }
    #[test]
    fn context_workflow_covers_required_native_profiles() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should have a workspace root");
        let workflow = std::fs::read_to_string(root.join(".github/workflows/context-snapshot.yml"))
            .expect("context snapshot workflow should exist");
        let manifest: ContextManifest = toml::from_str(
            &std::fs::read_to_string(root.join("xtask/context.toml"))
                .expect("context manifest should be readable"),
        )
        .expect("context manifest should parse");
        let required_profiles = manifest
            .profiles
            .iter()
            .filter(|profile| profile.required_in_ci)
            .map(|profile| profile.id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            required_profiles,
            BTreeSet::from([
                "native-linux-gnu",
                "native-linux-musl",
                "native-macos",
                "native-windows",
            ])
        );
        for profile in required_profiles {
            assert!(
                workflow.contains(profile),
                "workflow omits profile {profile:?}"
            );
        }
        for runner in ["ubuntu-latest", "macos-14-xlarge", "windows-latest"] {
            assert!(
                workflow.contains(runner),
                "workflow omits runner {runner:?}"
            );
        }
        assert!(workflow.contains("cargo-nextest@"));
        assert!(workflow.contains("cargo xtask info context --validate-all --save"));
        assert!(workflow.contains("path: artifacts/context/evidence/"));
        assert!(!workflow.contains("snapshot.json"));
        assert!(!workflow.contains("history.jsonl"));
        assert!(workflow.contains("context-evidence-${{ github.sha }}"));
        assert!(workflow.contains("actions/download-artifact@"));
        assert!(workflow.contains("merge-multiple: true"));
        assert!(workflow.contains(
            "cargo xtask info context --validate-all --strict --evidence-dir artifacts/context/evidence"
        ));
        assert!(workflow.contains("needs: collect"));
        assert!(workflow.contains("vz-advisory:"));
        assert!(workflow.contains("continue-on-error: true"));
        assert!(workflow.contains("--features miniboxd/vz"));

        for line in workflow.lines().map(str::trim) {
            let Some(reference) = line.strip_prefix("uses: ") else {
                continue;
            };
            let revision = reference
                .rsplit_once('@')
                .map(|(_, revision)| revision)
                .expect("action reference should contain a revision")
                .split_whitespace()
                .next()
                .expect("action revision should be present");
            assert_eq!(revision.len(), 40, "action is not SHA-pinned: {reference}");
            assert!(
                revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "action is not SHA-pinned: {reference}"
            );
        }
    }
}
