use super::super::model::{EvidenceId, ExecutableTest, ProfileStatus, TestProfileResult};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
