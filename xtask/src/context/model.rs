use anyhow::{Result, bail};
use serde::Serialize;
use std::path::{Component, Path};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub(super) struct RepositoryPath(String);

impl RepositoryPath {
    pub(super) fn from_root(root: &Path, path: &Path) -> Result<Self> {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| anyhow::anyhow!("path is outside workspace root: {}", path.display()))?;
        let mut parts = Vec::new();

        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_str().ok_or_else(|| {
                    anyhow::anyhow!("path is not valid UTF-8: {}", path.display())
                })?),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    bail!("path is not workspace-relative: {}", path.display());
                }
            }
        }

        if parts.is_empty() {
            bail!(
                "path does not identify a workspace file: {}",
                path.display()
            );
        }

        Ok(Self(parts.join("/")))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileMetrics {
    pub(super) physical_lines: usize,
    pub(super) non_empty_lines: usize,
    pub(super) content_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub(super) struct EvidenceId(String);

impl From<&str> for EvidenceId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EvidenceKind {
    CargoMetadata,
    Git,
    Manifest,
    Nextest,
    RustSource,
    ToolVersion,
    TrackedFile,
    CiArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EvidenceStatus {
    Collected,
    Failed,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Evidence {
    pub(super) kind: EvidenceKind,
    pub(super) locator: String,
    pub(super) collected_at: String,
    pub(super) content_sha256: Option<String>,
    pub(super) profile_id: Option<String>,
    pub(super) status: EvidenceStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ValidationState {
    Match,
    Mismatch,
    DeclaredOnly,
    ObservedOnly,
    Unavailable,
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Validation {
    pub(super) state: ValidationState,
    pub(super) reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Fact<T> {
    pub(super) declared: Option<T>,
    pub(super) observed: Option<T>,
    pub(super) validation: Validation,
    pub(super) evidence_ids: Vec<EvidenceId>,
}

impl<T> Fact<T> {
    pub(super) fn new(
        declared: Option<T>,
        observed: Option<T>,
        validation: Validation,
        mut evidence_ids: Vec<EvidenceId>,
    ) -> Self {
        evidence_ids.sort();
        evidence_ids.dedup();
        Self {
            declared,
            observed,
            validation,
            evidence_ids,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ContextDiagnostic {
    pub(super) code: String,
    pub(super) severity: DiagnosticSeverity,
    pub(super) message: String,
    pub(super) profile_id: Option<String>,
    pub(super) evidence_ids: Vec<EvidenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct RepositoryIdentity {
    pub(super) commit: String,
    pub(super) branch: String,
    pub(super) dirty: bool,
    pub(super) changed_paths: Vec<RepositoryPath>,
    pub(super) worktree_fingerprint: String,
    pub(super) evidence_ids: Vec<EvidenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct EnvironmentSnapshot {
    pub(super) host: String,
    pub(super) target: String,
    pub(super) enabled_features: Vec<String>,
    pub(super) rustc_version: Fact<String>,
    pub(super) cargo_version: Fact<String>,
    pub(super) nextest_version: Fact<String>,
    pub(super) generated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct WorkspaceSnapshot {
    pub(super) version: Fact<String>,
    pub(super) edition: Fact<String>,
    pub(super) msrv: Fact<String>,
    pub(super) packages: Vec<PackageSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct PackageSnapshot {
    pub(super) package_id: String,
    pub(super) name: String,
    pub(super) manifest_path: RepositoryPath,
    pub(super) targets: Vec<TargetSnapshot>,
    pub(super) features: Vec<String>,
    pub(super) dependencies: Vec<DependencySnapshot>,
    pub(super) metrics: SourceMetrics,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct DependencySnapshot {
    pub(super) package_name: String,
    pub(super) rename: Option<String>,
    pub(super) kind: DependencyKind,
    pub(super) target_predicate: Option<String>,
    pub(super) optional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DependencyKind {
    Normal,
    Build,
    Dev,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct TargetSnapshot {
    pub(super) name: String,
    pub(super) kinds: Vec<String>,
    pub(super) source_path: RepositoryPath,
    pub(super) required_features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct SourceMetrics {
    pub(super) rust_files: usize,
    pub(super) physical_lines: usize,
    pub(super) non_empty_lines: usize,
    pub(super) included_roots: Vec<RepositoryPath>,
    pub(super) includes_generated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn evidence_and_paths_serialize_stably() {
        let path = RepositoryPath::from_root(
            Path::new("/workspace"),
            Path::new("/workspace/xtask/src/context.rs"),
        )
        .expect("workspace path should be valid");
        assert_eq!(path.as_str(), "xtask/src/context.rs");
        assert!(
            RepositoryPath::from_root(Path::new("/workspace"), Path::new("/outside/context.rs"))
                .is_err()
        );

        let fact = Fact::new(
            Some("1.85".to_string()),
            None,
            Validation {
                state: ValidationState::DeclaredOnly,
                reason: None,
            },
            vec![EvidenceId::from("source-b"), EvidenceId::from("source-a")],
        );
        let diagnostic = ContextDiagnostic {
            code: "context.unavailable".to_string(),
            severity: DiagnosticSeverity::Warning,
            message: "nextest unavailable".to_string(),
            profile_id: None,
            evidence_ids: vec![EvidenceId::from("source-a")],
        };
        let evidence = Evidence {
            kind: EvidenceKind::CargoMetadata,
            locator: "cargo metadata --no-deps".to_string(),
            collected_at: "2026-09-08T00:00:00Z".to_string(),
            content_sha256: None,
            profile_id: None,
            status: EvidenceStatus::Collected,
        };

        let value = serde_json::json!({
            "path": path,
            "fact": fact,
            "diagnostic": diagnostic,
            "evidence": evidence,
        });

        assert_eq!(value["evidence"]["kind"], "cargo_metadata");
        assert_eq!(value["evidence"]["status"], "collected");
        assert_eq!(value["fact"]["validation"]["state"], "declared_only");
        assert_eq!(value["fact"]["evidence_ids"][0], "source-a");
        assert_eq!(value["fact"]["evidence_ids"][1], "source-b");
        assert!(value["fact"].get("observed").is_some());
        assert!(value["diagnostic"].get("profile_id").is_some());
        assert!(value["evidence"].get("content_sha256").is_some());
    }

    #[test]
    fn workspace_snapshot_separates_msrv_and_toolchain() {
        let declared = |value: &str| {
            Fact::new(
                Some(value.to_string()),
                None,
                Validation {
                    state: ValidationState::DeclaredOnly,
                    reason: None,
                },
                vec![EvidenceId::from("cargo-manifest")],
            )
        };
        let observed = |value: &str| {
            Fact::new(
                None,
                Some(value.to_string()),
                Validation {
                    state: ValidationState::ObservedOnly,
                    reason: None,
                },
                vec![EvidenceId::from("tool-version")],
            )
        };
        let workspace = WorkspaceSnapshot {
            version: declared("0.1.0"),
            edition: declared("2024"),
            msrv: declared("1.85"),
            packages: Vec::new(),
        };
        let environment = EnvironmentSnapshot {
            host: "aarch64-apple-darwin".to_string(),
            target: "aarch64-apple-darwin".to_string(),
            enabled_features: Vec::new(),
            rustc_version: observed("1.98.0"),
            cargo_version: observed("1.98.0"),
            nextest_version: observed("0.9.100"),
            generated_at: "2026-09-08T00:00:00Z".to_string(),
        };
        let value = serde_json::json!({
            "workspace": workspace,
            "environment": environment,
        });

        assert_eq!(
            value.pointer("/workspace/msrv/declared"),
            Some(&serde_json::json!("1.85"))
        );
        assert_eq!(
            value.pointer("/environment/rustc_version/observed"),
            Some(&serde_json::json!("1.98.0"))
        );
        assert!(value.pointer("/workspace/rust_version").is_none());
        assert!(value.pointer("/environment/msrv").is_none());
    }
}
