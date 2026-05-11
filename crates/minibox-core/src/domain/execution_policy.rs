//! Execution policy for manifest-based container admission control.
//!
//! An [`ExecutionPolicy`] contains a set of rules evaluated against an
//! [`ExecutionManifest`]. If any rule is violated, the run is denied
//! with a human-readable reason.

use serde::{Deserialize, Serialize};

use super::execution_manifest::ExecutionManifest;

/// A policy decision: allow the run or deny with reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
}

/// A set of rules that constrain which workloads may run.
///
/// All fields use `Option` -- `None` means "no constraint" (allow any).
/// When a field is `Some`, the manifest value must satisfy the constraint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    /// Allowed image reference patterns (glob-style). If set, the manifest's
    /// `subject.image_ref` must match at least one pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_images: Option<Vec<String>>,

    /// Denied image reference patterns. If the manifest's image_ref matches
    /// any pattern here, the run is denied (checked before allowed_images).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denied_images: Option<Vec<String>>,

    /// Allowed network modes. If set, manifest's network_mode must be in this list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_network_modes: Option<Vec<String>>,

    /// Whether privileged containers are allowed. Default: not constrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_privileged: Option<bool>,

    /// Maximum memory limit in bytes. If set and the manifest's memory_limit
    /// exceeds this, the run is denied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,

    /// Allowed mount host path prefixes. If set, every mount's host_path
    /// must start with one of these prefixes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_mount_prefixes: Option<Vec<String>>,

    /// If true, read-only mounts are always allowed regardless of
    /// allowed_mount_prefixes. Default: false.
    #[serde(default)]
    pub allow_readonly_mounts: bool,
}

impl ExecutionPolicy {
    /// Evaluate this policy against a manifest.
    ///
    /// Returns `PolicyDecision::Allow` if all rules pass, or
    /// `PolicyDecision::Deny(reason)` on the first violation.
    pub fn evaluate(&self, manifest: &ExecutionManifest) -> PolicyDecision {
        // Check denied images first
        if let Some(denied) = &self.denied_images {
            for pattern in denied {
                if image_matches(&manifest.subject.image_ref, pattern) {
                    return PolicyDecision::Deny(format!(
                        "image '{}' matches denied pattern '{}'",
                        manifest.subject.image_ref, pattern
                    ));
                }
            }
        }

        // Check allowed images
        if let Some(allowed) = &self.allowed_images {
            let matches = allowed
                .iter()
                .any(|p| image_matches(&manifest.subject.image_ref, p));
            if !matches {
                return PolicyDecision::Deny(format!(
                    "image '{}' not in allowed list",
                    manifest.subject.image_ref
                ));
            }
        }

        // Check network mode
        if let Some(allowed_modes) = &self.allowed_network_modes {
            if !allowed_modes
                .iter()
                .any(|m| m == &manifest.runtime.network_mode)
            {
                return PolicyDecision::Deny(format!(
                    "network mode '{}' not allowed (allowed: {})",
                    manifest.runtime.network_mode,
                    allowed_modes.join(", ")
                ));
            }
        }

        // Check privileged
        if let Some(false) = self.allow_privileged {
            if manifest.runtime.privileged {
                return PolicyDecision::Deny("privileged mode not allowed by policy".to_string());
            }
        }

        // Check memory limit
        if let Some(max_mem) = self.max_memory_bytes {
            if let Some(ref limits) = manifest.runtime.resource_limits {
                if let Some(mem) = limits.memory_limit_bytes {
                    if mem > max_mem {
                        return PolicyDecision::Deny(format!(
                            "memory limit {} exceeds policy maximum {}",
                            mem, max_mem
                        ));
                    }
                }
            }
        }

        // Check mount prefixes
        if let Some(prefixes) = &self.allowed_mount_prefixes {
            for mount in &manifest.runtime.mounts {
                if self.allow_readonly_mounts && mount.read_only {
                    continue;
                }
                let allowed = prefixes.iter().any(|p| mount.host_path.starts_with(p));
                if !allowed {
                    return PolicyDecision::Deny(format!(
                        "mount host_path '{}' not under any allowed prefix",
                        mount.host_path
                    ));
                }
            }
        }

        PolicyDecision::Allow
    }
}

/// Simple glob matching: `*` matches any sequence, everything else is literal.
fn image_matches(image: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return image.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return image.ends_with(suffix);
    }
    image == pattern
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::execution_manifest::{
        ExecutionManifestImage, ExecutionManifestMount, ExecutionManifestRequest,
        ExecutionManifestResourceLimits, ExecutionManifestRuntime, ExecutionManifestSubject,
    };

    fn sample_manifest() -> ExecutionManifest {
        ExecutionManifest {
            schema_version: 1,
            container_id: "test-001".to_string(),
            created_at: "2026-05-11T00:00:00Z".to_string(),
            manifest_path: None,
            workload_digest: None,
            subject: ExecutionManifestSubject {
                image_ref: "alpine:3.18".to_string(),
                image: ExecutionManifestImage {
                    manifest_digest: None,
                    config_digest: None,
                    layer_digests: vec![],
                },
            },
            runtime: ExecutionManifestRuntime {
                command: vec!["echo".to_string(), "hello".to_string()],
                env: vec![],
                mounts: vec![],
                resource_limits: Some(ExecutionManifestResourceLimits {
                    memory_limit_bytes: Some(128 * 1024 * 1024),
                    cpu_weight: None,
                }),
                network_mode: "none".to_string(),
                privileged: false,
                platform: None,
            },
            request: ExecutionManifestRequest {
                name: None,
                ephemeral: true,
            },
        }
    }

    #[test]
    fn default_policy_allows_everything() {
        let policy = ExecutionPolicy::default();
        let manifest = sample_manifest();
        assert_eq!(policy.evaluate(&manifest), PolicyDecision::Allow);
    }

    #[test]
    fn allowed_images_permits_matching() {
        let policy = ExecutionPolicy {
            allowed_images: Some(vec!["alpine*".to_string()]),
            ..Default::default()
        };
        assert_eq!(policy.evaluate(&sample_manifest()), PolicyDecision::Allow);
    }

    #[test]
    fn allowed_images_denies_non_matching() {
        let policy = ExecutionPolicy {
            allowed_images: Some(vec!["ubuntu*".to_string()]),
            ..Default::default()
        };
        let decision = policy.evaluate(&sample_manifest());
        assert!(
            matches!(decision, PolicyDecision::Deny(ref s) if s.contains("not in allowed list"))
        );
    }

    #[test]
    fn denied_images_blocks_matching() {
        let policy = ExecutionPolicy {
            denied_images: Some(vec!["alpine*".to_string()]),
            ..Default::default()
        };
        let decision = policy.evaluate(&sample_manifest());
        assert!(matches!(decision, PolicyDecision::Deny(ref s) if s.contains("denied pattern")));
    }

    #[test]
    fn denied_images_checked_before_allowed() {
        let policy = ExecutionPolicy {
            allowed_images: Some(vec!["alpine*".to_string()]),
            denied_images: Some(vec!["alpine:3.18".to_string()]),
            ..Default::default()
        };
        let decision = policy.evaluate(&sample_manifest());
        assert!(matches!(decision, PolicyDecision::Deny(ref s) if s.contains("denied pattern")));
    }

    #[test]
    fn network_mode_restriction() {
        let policy = ExecutionPolicy {
            allowed_network_modes: Some(vec!["bridge".to_string()]),
            ..Default::default()
        };
        let decision = policy.evaluate(&sample_manifest());
        assert!(matches!(decision, PolicyDecision::Deny(ref s) if s.contains("network mode")));
    }

    #[test]
    fn privileged_denial() {
        let policy = ExecutionPolicy {
            allow_privileged: Some(false),
            ..Default::default()
        };
        let mut manifest = sample_manifest();
        manifest.runtime.privileged = true;
        let decision = policy.evaluate(&manifest);
        assert!(matches!(decision, PolicyDecision::Deny(ref s) if s.contains("privileged")));
    }

    #[test]
    fn memory_limit_enforcement() {
        let policy = ExecutionPolicy {
            max_memory_bytes: Some(64 * 1024 * 1024),
            ..Default::default()
        };
        let decision = policy.evaluate(&sample_manifest());
        assert!(
            matches!(decision, PolicyDecision::Deny(ref s) if s.contains("exceeds policy maximum"))
        );
    }

    #[test]
    fn mount_prefix_restriction() {
        let policy = ExecutionPolicy {
            allowed_mount_prefixes: Some(vec!["/safe/".to_string()]),
            ..Default::default()
        };
        let mut manifest = sample_manifest();
        manifest.runtime.mounts = vec![ExecutionManifestMount {
            host_path: "/unsafe/data".to_string(),
            container_path: "/mnt".to_string(),
            read_only: false,
        }];
        let decision = policy.evaluate(&manifest);
        assert!(
            matches!(decision, PolicyDecision::Deny(ref s) if s.contains("not under any allowed prefix"))
        );
    }

    #[test]
    fn allow_readonly_mounts_bypasses_prefix_check() {
        let policy = ExecutionPolicy {
            allowed_mount_prefixes: Some(vec!["/safe/".to_string()]),
            allow_readonly_mounts: true,
            ..Default::default()
        };
        let mut manifest = sample_manifest();
        manifest.runtime.mounts = vec![ExecutionManifestMount {
            host_path: "/anywhere/data".to_string(),
            container_path: "/mnt".to_string(),
            read_only: true,
        }];
        assert_eq!(policy.evaluate(&manifest), PolicyDecision::Allow);
    }

    #[test]
    fn image_matches_glob_patterns() {
        // Wildcard matches all
        assert!(image_matches("anything", "*"));
        // Prefix glob
        assert!(image_matches("alpine:3.18", "alpine*"));
        assert!(!image_matches("ubuntu:22.04", "alpine*"));
        // Suffix glob
        assert!(image_matches("myregistry/alpine", "*alpine"));
        assert!(!image_matches("myregistry/ubuntu", "*alpine"));
        // Exact match
        assert!(image_matches("alpine:3.18", "alpine:3.18"));
        assert!(!image_matches("alpine:3.19", "alpine:3.18"));
    }

    #[test]
    fn policy_roundtrips_through_json() {
        let policy = ExecutionPolicy {
            allowed_images: Some(vec!["alpine*".to_string()]),
            denied_images: Some(vec!["*:latest".to_string()]),
            allowed_network_modes: Some(vec!["none".to_string()]),
            allow_privileged: Some(false),
            max_memory_bytes: Some(512 * 1024 * 1024),
            allowed_mount_prefixes: Some(vec!["/data/".to_string()]),
            allow_readonly_mounts: true,
        };
        let json = serde_json::to_string_pretty(&policy).expect("serialise policy");
        let restored: ExecutionPolicy = serde_json::from_str(&json).expect("deserialise policy");
        // Compare via re-serialisation since Default doesn't impl PartialEq
        let json2 = serde_json::to_string_pretty(&restored).expect("re-serialise policy");
        assert_eq!(json, json2);
    }
}
