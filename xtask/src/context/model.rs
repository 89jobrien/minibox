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
}
