//! Execution manifest types for externally verifiable container execution.
//!
//! An [`ExecutionManifest`] captures every measured input to a container run
//! — image identity, command, environment (hashed, never plaintext), mounts,
//! resource limits, and network mode — along with a deterministic
//! [`workload_digest`](ExecutionManifest::workload_digest) that uniquely
//! identifies the workload configuration.
//!
//! # Digest stability
//!
//! The workload digest is computed from a stable JSON projection that
//! deliberately excludes volatile fields (creation timestamp, manifest path,
//! and the digest field itself). Equal semantic inputs always produce equal
//! digests regardless of serialisation order.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// ─── Core manifest types ─────────────────────────────────────────────────────

/// Complete execution manifest for a single container run.
///
/// Persisted to `{containers_base}/{id}/execution-manifest.json` before
/// the container process is spawned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionManifest {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Unique container identifier.
    pub container_id: String,
    /// Timestamp of manifest creation (ISO 8601). Excluded from digest.
    pub created_at: String,
    /// Filesystem path where this manifest is persisted. Excluded from digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<PathBuf>,
    /// The deterministic workload digest. Excluded from its own computation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_digest: Option<String>,

    /// What is being run.
    pub subject: ExecutionManifestSubject,
    /// Runtime configuration.
    pub runtime: ExecutionManifestRuntime,
    /// Original run request parameters.
    pub request: ExecutionManifestRequest,
}

/// Identifies the container image.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionManifestSubject {
    /// Image reference as provided (e.g. `alpine:3.18`).
    pub image_ref: String,
    /// Resolved image details.
    pub image: ExecutionManifestImage,
}

/// Resolved image identity with content-addressable digests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionManifestImage {
    /// Image manifest digest (sha256), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
    /// Image config digest (sha256), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_digest: Option<String>,
    /// Ordered layer digests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_digests: Vec<String>,
}

/// Runtime configuration that affects execution behaviour.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionManifestRuntime {
    /// Command and arguments.
    pub command: Vec<String>,
    /// Environment variables: name + SHA-256 of value (never plaintext).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<ExecutionManifestEnvVar>,
    /// Bind mounts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<ExecutionManifestMount>,
    /// Resource limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_limits: Option<ExecutionManifestResourceLimits>,
    /// Network mode.
    pub network_mode: String,
    /// Whether the container runs in privileged mode.
    pub privileged: bool,
    /// Requested platform override, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

/// An environment variable with its value hashed for privacy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionManifestEnvVar {
    /// Variable name.
    pub name: String,
    /// SHA-256 hex digest of the variable value.
    pub value_digest: String,
}

/// A bind mount entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionManifestMount {
    /// Host path.
    pub host_path: String,
    /// Container path.
    pub container_path: String,
    /// Read-only flag.
    pub read_only: bool,
}

/// Resource limits recorded in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionManifestResourceLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_weight: Option<u64>,
}

/// A content digest with algorithm prefix (e.g. `sha256:abcdef...`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionManifestDigest {
    pub algorithm: String,
    pub hex: String,
}

impl std::fmt::Display for ExecutionManifestDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.algorithm, self.hex)
    }
}

// ─── Digest computation ──────────────────────────────────────────────────────

impl ExecutionManifest {
    /// Compute the deterministic workload digest.
    ///
    /// The digest covers a stable JSON projection that excludes volatile fields:
    /// `created_at`, `manifest_path`, and `workload_digest` itself.
    pub fn compute_workload_digest(&self) -> ExecutionManifestDigest {
        let projection = self.digest_projection();
        // SAFETY: serialisation cannot fail — the projection contains only
        // primitive types (u32, bool, String, Vec<String>, Option<T>) and
        // structs composed of the same. No maps with non-string keys, no
        // custom Serialize impls that can error.
        let json = serde_json::to_string(&projection)
            .expect("digest projection contains only infallible serialisable types");
        let hash = Sha256::digest(json.as_bytes());
        ExecutionManifestDigest {
            algorithm: "sha256".to_string(),
            hex: hex::encode(hash),
        }
    }

    /// Compute and set `self.workload_digest`.
    pub fn seal(&mut self) {
        let digest = self.compute_workload_digest();
        self.workload_digest = Some(digest.to_string());
    }

    /// Build the stable projection used for digest computation.
    ///
    /// Fields are serialised in a deterministic order via struct field order
    /// (serde serialises struct fields in declaration order).
    fn digest_projection(&self) -> DigestProjection<'_> {
        DigestProjection {
            schema_version: self.schema_version,
            subject: &self.subject,
            runtime: &self.runtime,
            request: &self.request,
        }
    }
}

/// The subset of [`ExecutionManifest`] fields included in the workload digest.
///
/// Volatile / instance-specific fields (`container_id`, `created_at`,
/// `manifest_path`, `workload_digest`) are intentionally excluded so that
/// the digest depends only on semantic inputs.
#[derive(Serialize)]
struct DigestProjection<'a> {
    schema_version: u32,
    subject: &'a ExecutionManifestSubject,
    runtime: &'a ExecutionManifestRuntime,
    request: &'a ExecutionManifestRequest,
}

/// Original run request parameters captured verbatim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionManifestRequest {
    /// Container name if provided by the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether the run was ephemeral (streaming).
    pub ephemeral: bool,
}

// ─── Builder helpers ─────────────────────────────────────────────────────────

impl ExecutionManifestEnvVar {
    /// Create an env var entry by hashing the value.
    pub fn new(name: impl Into<String>, value: &str) -> Self {
        let hash = Sha256::digest(value.as_bytes());
        Self {
            name: name.into(),
            value_digest: hex::encode(hash),
        }
    }
}

impl ExecutionManifestMount {
    /// Create from a domain `BindMount`.
    pub fn from_bind_mount(m: &super::BindMount) -> Self {
        Self {
            host_path: m.host_path.to_string_lossy().to_string(),
            container_path: m.container_path.to_string_lossy().to_string(),
            read_only: m.read_only,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::option;
    use proptest::prelude::*;

    fn sample_manifest() -> ExecutionManifest {
        ExecutionManifest {
            schema_version: 1,
            container_id: "abc123".to_string(),
            created_at: "2026-05-11T00:00:00Z".to_string(),
            manifest_path: Some(PathBuf::from("/tmp/manifest.json")),
            workload_digest: None,
            subject: ExecutionManifestSubject {
                image_ref: "alpine:3.18".to_string(),
                image: ExecutionManifestImage {
                    manifest_digest: Some("sha256:aaa".to_string()),
                    config_digest: Some("sha256:bbb".to_string()),
                    layer_digests: vec!["sha256:layer1".to_string()],
                },
            },
            runtime: ExecutionManifestRuntime {
                command: vec!["echo".to_string(), "hello".to_string()],
                env: vec![ExecutionManifestEnvVar::new("FOO", "bar")],
                mounts: vec![],
                resource_limits: Some(ExecutionManifestResourceLimits {
                    memory_limit_bytes: Some(256 * 1024 * 1024),
                    cpu_weight: None,
                }),
                network_mode: "none".to_string(),
                privileged: false,
                platform: None,
            },
            request: ExecutionManifestRequest {
                name: Some("test-container".to_string()),
                ephemeral: true,
            },
        }
    }

    #[test]
    fn equal_inputs_produce_equal_digest() {
        let m1 = sample_manifest();
        let m2 = sample_manifest();
        assert_eq!(m1.compute_workload_digest(), m2.compute_workload_digest());
    }

    #[test]
    fn volatile_fields_do_not_affect_digest() {
        let m1 = sample_manifest();
        let mut m2 = sample_manifest();
        m2.container_id = "different-id-999".to_string();
        m2.created_at = "2099-12-31T23:59:59Z".to_string();
        m2.manifest_path = Some(PathBuf::from("/other/path"));
        m2.workload_digest = Some("sha256:stale".to_string());
        assert_eq!(m1.compute_workload_digest(), m2.compute_workload_digest());
    }

    #[test]
    fn changed_command_changes_digest() {
        let m1 = sample_manifest();
        let mut m2 = sample_manifest();
        m2.runtime.command = vec!["ls".to_string(), "-la".to_string()];
        assert_ne!(m1.compute_workload_digest(), m2.compute_workload_digest());
    }

    #[test]
    fn changed_env_changes_digest() {
        let m1 = sample_manifest();
        let mut m2 = sample_manifest();
        m2.runtime.env = vec![ExecutionManifestEnvVar::new("FOO", "different")];
        assert_ne!(m1.compute_workload_digest(), m2.compute_workload_digest());
    }

    #[test]
    fn changed_mount_changes_digest() {
        let m1 = sample_manifest();
        let mut m2 = sample_manifest();
        m2.runtime.mounts = vec![ExecutionManifestMount {
            host_path: "/data".to_string(),
            container_path: "/mnt".to_string(),
            read_only: true,
        }];
        assert_ne!(m1.compute_workload_digest(), m2.compute_workload_digest());
    }

    #[test]
    fn changed_network_changes_digest() {
        let m1 = sample_manifest();
        let mut m2 = sample_manifest();
        m2.runtime.network_mode = "host".to_string();
        assert_ne!(m1.compute_workload_digest(), m2.compute_workload_digest());
    }

    #[test]
    fn changed_image_digest_changes_workload_digest() {
        let m1 = sample_manifest();
        let mut m2 = sample_manifest();
        m2.subject.image.manifest_digest = Some("sha256:changed".to_string());
        assert_ne!(m1.compute_workload_digest(), m2.compute_workload_digest());
    }

    #[test]
    fn seal_sets_workload_digest() {
        let mut m = sample_manifest();
        assert!(m.workload_digest.is_none());
        m.seal();
        assert!(m.workload_digest.is_some());
        let digest = m.workload_digest.as_ref().expect("sealed");
        assert!(digest.starts_with("sha256:"));
        assert_eq!(digest.len(), 7 + 64); // "sha256:" + 64 hex chars
    }

    #[test]
    fn env_var_value_is_never_plaintext() {
        let var = ExecutionManifestEnvVar::new("SECRET", "super-secret-value");
        assert!(!var.value_digest.contains("super-secret-value"));
        assert_eq!(var.value_digest.len(), 64); // SHA-256 hex
    }

    #[test]
    fn manifest_roundtrips_through_json() {
        let mut m = sample_manifest();
        m.seal();
        let json = serde_json::to_string_pretty(&m).expect("serialise");
        let m2: ExecutionManifest = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(m, m2);
    }

    #[test]
    fn different_container_id_produces_same_digest() {
        let m1 = sample_manifest();
        let mut m2 = sample_manifest();
        m2.container_id = "completely-different-uuid-456".to_string();
        assert_eq!(
            m1.compute_workload_digest(),
            m2.compute_workload_digest(),
            "container_id is volatile and must not affect workload digest"
        );
    }

    #[test]
    fn digest_display_format() {
        let d = ExecutionManifestDigest {
            algorithm: "sha256".to_string(),
            hex: "abcdef".to_string(),
        };
        assert_eq!(d.to_string(), "sha256:abcdef");
    }

    // ── Proptest strategies ───────────────────────────────────────────────────

    fn arb_env_var() -> impl Strategy<Value = ExecutionManifestEnvVar> {
        (any::<String>(), any::<String>())
            .prop_map(|(name, value_digest)| ExecutionManifestEnvVar { name, value_digest })
    }

    fn arb_mount() -> impl Strategy<Value = ExecutionManifestMount> {
        (any::<String>(), any::<String>(), any::<bool>()).prop_map(
            |(host_path, container_path, read_only)| ExecutionManifestMount {
                host_path,
                container_path,
                read_only,
            },
        )
    }

    fn arb_resource_limits() -> impl Strategy<Value = ExecutionManifestResourceLimits> {
        (option::of(any::<u64>()), option::of(any::<u64>())).prop_map(
            |(memory_limit_bytes, cpu_weight)| ExecutionManifestResourceLimits {
                memory_limit_bytes,
                cpu_weight,
            },
        )
    }

    fn arb_image() -> impl Strategy<Value = ExecutionManifestImage> {
        (
            option::of(any::<String>()),
            option::of(any::<String>()),
            proptest::collection::vec(any::<String>(), 0..4),
        )
            .prop_map(|(manifest_digest, config_digest, layer_digests)| {
                ExecutionManifestImage {
                    manifest_digest,
                    config_digest,
                    layer_digests,
                }
            })
    }

    fn arb_subject() -> impl Strategy<Value = ExecutionManifestSubject> {
        (any::<String>(), arb_image())
            .prop_map(|(image_ref, image)| ExecutionManifestSubject { image_ref, image })
    }

    fn arb_runtime() -> impl Strategy<Value = ExecutionManifestRuntime> {
        (
            proptest::collection::vec(any::<String>(), 0..6),
            proptest::collection::vec(arb_env_var(), 0..4),
            proptest::collection::vec(arb_mount(), 0..4),
            option::of(arb_resource_limits()),
            any::<String>(),
            any::<bool>(),
            option::of(any::<String>()),
        )
            .prop_map(
                |(command, env, mounts, resource_limits, network_mode, privileged, platform)| {
                    ExecutionManifestRuntime {
                        command,
                        env,
                        mounts,
                        resource_limits,
                        network_mode,
                        privileged,
                        platform,
                    }
                },
            )
    }

    fn arb_request() -> impl Strategy<Value = ExecutionManifestRequest> {
        (option::of(any::<String>()), any::<bool>())
            .prop_map(|(name, ephemeral)| ExecutionManifestRequest { name, ephemeral })
    }

    fn arb_manifest() -> impl Strategy<Value = ExecutionManifest> {
        (
            any::<u32>(),
            any::<String>(),
            any::<String>(),
            option::of(any::<String>().prop_map(PathBuf::from)),
            option::of(any::<String>()),
            arb_subject(),
            arb_runtime(),
            arb_request(),
        )
            .prop_map(
                |(
                    schema_version,
                    container_id,
                    created_at,
                    manifest_path,
                    workload_digest,
                    subject,
                    runtime,
                    request,
                )| {
                    ExecutionManifest {
                        schema_version,
                        container_id,
                        created_at,
                        manifest_path,
                        workload_digest,
                        subject,
                        runtime,
                        request,
                    }
                },
            )
    }

    // ── Proptest roundtrip tests ──────────────────────────────────────────────

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            failure_persistence: None,
            ..proptest::prelude::ProptestConfig::default()
        })]

        /// Any `ExecutionManifest` must survive a JSON serialize→deserialize roundtrip
        /// without data loss.
        #[test]
        fn execution_manifest_json_roundtrip(m in arb_manifest()) {
            let json = serde_json::to_string(&m)
                .expect("ExecutionManifest must serialize to JSON");
            let decoded: ExecutionManifest = serde_json::from_str(&json)
                .expect("ExecutionManifest must deserialize from JSON");
            proptest::prop_assert_eq!(m, decoded);
        }

        /// Any `ExecutionManifestRuntime` must survive JSON roundtrip.
        #[test]
        fn execution_manifest_runtime_json_roundtrip(r in arb_runtime()) {
            let json = serde_json::to_string(&r)
                .expect("ExecutionManifestRuntime must serialize");
            let decoded: ExecutionManifestRuntime = serde_json::from_str(&json)
                .expect("ExecutionManifestRuntime must deserialize");
            proptest::prop_assert_eq!(r, decoded);
        }

        /// Any `ExecutionManifestSubject` must survive JSON roundtrip.
        #[test]
        fn execution_manifest_subject_json_roundtrip(s in arb_subject()) {
            let json = serde_json::to_string(&s)
                .expect("ExecutionManifestSubject must serialize");
            let decoded: ExecutionManifestSubject = serde_json::from_str(&json)
                .expect("ExecutionManifestSubject must deserialize");
            proptest::prop_assert_eq!(s, decoded);
        }
    }
}
