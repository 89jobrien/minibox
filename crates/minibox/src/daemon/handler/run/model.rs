//! Data models and builders for container run preparation.

use chrono::Utc;
use minibox_core::domain::{BindMount, ContainerSpawnConfig, NetworkMode};
use minibox_core::protocol::ContainerInfo;
use std::path::PathBuf;

use crate::daemon::network_lifecycle::NetworkLifecycle;
use crate::daemon::state::{ContainerRecord, RunCreationParams};

use super::super::PolicyOverride;

/// User-supplied container configuration shared by the run pipeline.
#[derive(Default)]
pub struct RunParams {
    /// Source image name or reference.
    pub image: String,
    /// Optional image tag.
    pub tag: Option<String>,
    /// Command and arguments to execute.
    pub command: Vec<String>,
    /// Optional memory limit in bytes.
    pub memory_limit_bytes: Option<u64>,
    /// Optional relative CPU scheduling weight.
    pub cpu_weight: Option<u64>,
    /// Whether the container is removed after it exits.
    pub ephemeral: bool,
    /// Requested network mode.
    pub network: Option<NetworkMode>,
    /// Host bind mounts requested for the container.
    pub mounts: Vec<BindMount>,
    /// Whether privileged execution is requested.
    pub privileged: bool,
    /// Environment variables in `KEY=VALUE` form.
    pub env: Vec<String>,
    /// Optional human-readable container name.
    pub name: Option<String>,
    /// Optional image platform override.
    pub platform: Option<String>,
    /// Optional parent cgroup path.
    pub cgroup_parent: Option<String>,
    /// Optional scheduling priority.
    pub priority: Option<slashcrux::Priority>,
    /// Scoped policy overrides for trusted internal callers.
    pub policy_override: Option<PolicyOverride>,
}

/// Bundled parameters for `build_execution_manifest`.
#[cfg(unix)]
pub(super) struct ManifestBuildParams<'a> {
    pub(super) id: &'a str,
    pub(super) ref_str: &'a str,
    pub(super) layer_dirs: &'a [PathBuf],
    pub(super) command: &'a [String],
    pub(super) env: &'a [String],
    pub(super) mounts: &'a [BindMount],
    pub(super) memory_limit_bytes: Option<u64>,
    pub(super) cpu_weight: Option<u64>,
    pub(super) net_mode: NetworkMode,
    pub(super) privileged: bool,
    pub(super) platform: &'a Option<String>,
    pub(super) name: &'a Option<String>,
    pub(super) capture_output: bool,
}

/// Bundled parameters for `build_container_record`.
#[cfg(unix)]
pub(super) struct ContainerRecordBuildParams<'a> {
    pub(super) id: &'a str,
    pub(super) name: &'a Option<String>,
    pub(super) image_label: &'a str,
    pub(super) command: &'a [String],
    pub(super) merged_dir: &'a minibox_core::path::InternalPath,
    pub(super) cgroup_dir: &'a std::path::Path,
    pub(super) rootfs_layout: &'a minibox_core::domain::RootfsLayout,
    pub(super) image: &'a str,
    pub(super) tag: &'a str,
    pub(super) memory_limit_bytes: Option<u64>,
    pub(super) cpu_weight: Option<u64>,
    pub(super) network: Option<NetworkMode>,
    pub(super) env: &'a [String],
    pub(super) mounts: &'a [BindMount],
    pub(super) privileged: bool,
    pub(super) platform: &'a Option<String>,
    pub(super) cgroup_parent: &'a Option<String>,
}

/// All state produced by container preparation, before the process is spawned.
#[cfg(unix)]
pub(super) struct PreparedRun {
    pub(super) id: String,
    pub(super) spawn_config: ContainerSpawnConfig,
    pub(super) image_label: String,
    /// Network lifecycle handle that must stay alive until attach is called.
    pub(super) net: NetworkLifecycle,
    pub(super) manifest_path: PathBuf,
    pub(super) workload_digest: String,
}

/// Construct an `ExecutionManifest` from container run parameters.
#[cfg(unix)]
pub(super) fn build_execution_manifest(
    p: ManifestBuildParams<'_>,
) -> minibox_core::domain::ExecutionManifest {
    let ManifestBuildParams {
        id,
        ref_str,
        layer_dirs,
        command,
        env,
        mounts,
        memory_limit_bytes,
        cpu_weight,
        net_mode,
        privileged,
        platform,
        name,
        capture_output,
    } = p;
    use minibox_core::domain::{
        ExecutionManifest, ExecutionManifestEnvVar, ExecutionManifestImage, ExecutionManifestMount,
        ExecutionManifestRequest, ExecutionManifestResourceLimits, ExecutionManifestRuntime,
        ExecutionManifestSubject,
    };

    let net_mode_str = net_mode.as_str().to_string();
    ExecutionManifest {
        schema_version: 1,
        container_id: id.to_string(),
        created_at: Utc::now().to_rfc3339(),
        manifest_path: None,
        workload_digest: None,
        subject: ExecutionManifestSubject {
            image_ref: ref_str.to_string(),
            image: ExecutionManifestImage {
                manifest_digest: None,
                config_digest: None,
                layer_digests: layer_dirs
                    .iter()
                    .filter_map(|p| p.file_name()?.to_str().map(|s| s.replacen('_', ":", 1)))
                    .collect(),
            },
        },
        runtime: ExecutionManifestRuntime {
            command: command.to_vec(),
            env: env
                .iter()
                .filter_map(|e| {
                    let (k, v) = e.split_once('=')?;
                    Some(ExecutionManifestEnvVar::new(k, v))
                })
                .collect(),
            mounts: mounts
                .iter()
                .map(ExecutionManifestMount::from_bind_mount)
                .collect(),
            resource_limits: Some(ExecutionManifestResourceLimits {
                memory_limit_bytes,
                cpu_weight,
            }),
            network_mode: net_mode_str,
            privileged,
            platform: platform.clone(),
        },
        request: ExecutionManifestRequest {
            name: name.clone(),
            ephemeral: capture_output,
        },
    }
}

/// Build a `ContainerRecord` in `Created` state for a new container.
#[cfg(unix)]
pub(super) fn build_container_record(p: ContainerRecordBuildParams<'_>) -> ContainerRecord {
    let ContainerRecordBuildParams {
        id,
        name,
        image_label,
        command,
        merged_dir,
        cgroup_dir,
        rootfs_layout,
        image,
        tag,
        memory_limit_bytes,
        cpu_weight,
        network,
        env,
        mounts,
        privileged,
        platform,
        cgroup_parent,
    } = p;
    let command_str = command.join(" ");
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: name.clone(),
            image: image_label.to_string(),
            command: command_str,
            state: "Created".to_string(),
            created_at: Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: merged_dir.clone().into_inner(),
        cgroup_path: cgroup_dir.to_path_buf(),
        post_exit_hooks: vec![],
        rootfs_metadata: rootfs_layout.rootfs_metadata.clone(),
        source_image_ref: rootfs_layout
            .source_image_ref
            .clone()
            .or_else(|| Some(image_label.to_string())),
        upper_dir: rootfs_layout
            .rootfs_metadata
            .as_ref()
            .map(|m| m.overlay_upper_dir().clone().into_inner()),
        merged_dir: Some(merged_dir.clone().into_inner()),
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: Some(RunCreationParams {
            image: image.to_string(),
            tag: Some(tag.to_string()),
            command: command.to_vec(),
            memory_limit_bytes,
            cpu_weight,
            network,
            env: env.to_vec(),
            mounts: mounts.to_vec(),
            privileged,
            name: name.clone(),
            tty: false,
            entrypoint: None,
            user: None,
            platform: platform.clone(),
            cgroup_parent: cgroup_parent.clone(),
        }),
        manifest_path: None,
        workload_digest: None,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use minibox_core::domain::{BackendRootfsMetadata, RootfsLayout};
    use minibox_core::path::InternalPath;
    use std::collections::HashMap;

    fn layout_with_upper(upper: &str) -> RootfsLayout {
        RootfsLayout {
            merged_dir: PathBuf::from("/var/lib/minibox/containers/abc/merged").into(),
            rootfs_metadata: Some(BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from(upper).into(),
                metadata: HashMap::new(),
            }),
            source_image_ref: None,
        }
    }

    fn layout_without_metadata() -> RootfsLayout {
        RootfsLayout {
            merged_dir: PathBuf::from("/var/lib/minibox/containers/abc/merged").into(),
            rootfs_metadata: None,
            source_image_ref: None,
        }
    }

    fn record_from(layout: &RootfsLayout) -> ContainerRecord {
        let merged: InternalPath = PathBuf::from("/var/lib/minibox/containers/abc/merged").into();
        let cgroup = std::path::Path::new("/sys/fs/cgroup/minibox/abc");
        let command = vec!["/bin/sh".to_string()];
        let name: Option<String> = None;
        let platform: Option<String> = None;
        let cgroup_parent: Option<String> = None;
        build_container_record(ContainerRecordBuildParams {
            id: "abc123",
            name: &name,
            image_label: "alpine:latest",
            command: &command,
            merged_dir: &merged,
            cgroup_dir: cgroup,
            rootfs_layout: layout,
            image: "alpine",
            tag: "latest",
            memory_limit_bytes: None,
            cpu_weight: None,
            network: None,
            env: &[],
            mounts: &[],
            privileged: false,
            platform: &platform,
            cgroup_parent: &cgroup_parent,
        })
    }

    #[test]
    fn build_container_record_populates_upper_dir_from_overlay_metadata() {
        let layout = layout_with_upper("/var/lib/minibox/containers/abc/upper");
        let record = record_from(&layout);
        assert_eq!(
            record.upper_dir,
            Some(PathBuf::from("/var/lib/minibox/containers/abc/upper")),
            "upper_dir must be projected from RootfsLayout::rootfs_metadata"
        );
    }

    #[test]
    fn build_container_record_leaves_upper_dir_none_without_metadata() {
        let record = record_from(&layout_without_metadata());
        assert_eq!(
            record.upper_dir, None,
            "copy-based backends expose no overlay upper dir"
        );
    }

    #[test]
    fn build_container_record_merged_dir_matches_rootfs_path() {
        let layout = layout_with_upper("/var/lib/minibox/containers/abc/upper");
        let record = record_from(&layout);
        assert_eq!(
            record.merged_dir.as_deref(),
            Some(record.rootfs_path.as_path()),
            "merged_dir and rootfs_path derive from the same InternalPath"
        );
    }

    #[test]
    fn build_container_record_preserves_rootfs_metadata_verbatim() {
        let layout = layout_with_upper("/var/lib/minibox/containers/abc/upper");
        let record = record_from(&layout);
        assert_eq!(record.rootfs_metadata, layout.rootfs_metadata);
    }

    #[test]
    fn build_container_record_prefers_layout_source_image_ref() {
        let mut layout = layout_with_upper("/var/lib/minibox/containers/abc/upper");
        layout.source_image_ref = Some("docker.io/library/alpine@sha256:dead".to_string());
        let record = record_from(&layout);
        assert_eq!(
            record.source_image_ref.as_deref(),
            Some("docker.io/library/alpine@sha256:dead"),
            "layout ref wins over the image_label fallback"
        );
    }

    #[test]
    fn build_container_record_falls_back_to_image_label() {
        let record = record_from(&layout_without_metadata());
        assert_eq!(
            record.source_image_ref.as_deref(),
            Some("alpine:latest"),
            "image_label is the fallback source_image_ref"
        );
    }
}
