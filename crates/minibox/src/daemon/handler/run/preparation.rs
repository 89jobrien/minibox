//! Resource preparation for container runs.

use anyhow::{Context as _, Result};
use minibox_core::domain::{
    ContainerHooks, ContainerSpawnConfig, DomainError, NetworkMode, ResourceConfig,
};
use minibox_core::image::reference::ImageRef;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

use crate::daemon::network_lifecycle::NetworkLifecycle;
use crate::daemon::state::DaemonState;

use super::super::HandlerDependencies;
use super::model::{
    ContainerRecordBuildParams, ManifestBuildParams, PreparedRun, build_container_record,
    build_execution_manifest,
};
use super::{RunParams, generate_container_id};

/// Prepare image, filesystem, cgroup, network, state, and spawn configuration.
#[cfg(unix)]
pub(super) async fn prepare_run(
    params: RunParams,
    capture_output: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<PreparedRun> {
    let RunParams {
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        network,
        mounts,
        privileged,
        env,
        name,
        platform,
        cgroup_parent,
        ephemeral: _,
        priority: _,
        policy_override: _,
    } = params;

    let ref_str = match &tag {
        Some(tag) => format!("{image}:{tag}"),
        None => image.clone(),
    };
    let image_ref = ImageRef::parse(&ref_str)
        .with_context(|| format!("invalid image reference {ref_str:?}"))
        .map_err(|error| DomainError::InvalidConfig(error.to_string()))?;
    let tag = image_ref.tag.clone();
    let full_image = image_ref.cache_name();

    let platform_registry =
        super::super::image::resolve_platform_registry(&platform, &image_ref, &deps)?;
    let default_registry = deps.image.registry_router.route(&image_ref);
    let registry: &dyn minibox_core::domain::ImageRegistry = match &platform_registry {
        Some(registry) => registry.as_ref(),
        None => default_registry,
    };

    if !registry.has_image(&full_image, &tag).await {
        info!("image {full_image}:{tag} not cached, pulling...");
        registry
            .pull_image(&image_ref)
            .await
            .map_err(|source| DomainError::ImagePullFailed {
                image: full_image.clone(),
                tag: tag.clone(),
                source,
            })?;
    }

    let layer_dirs = registry.get_image_layers(&full_image, &tag)?;
    if layer_dirs.is_empty() {
        return Err(DomainError::EmptyImage {
            name: full_image.clone(),
            tag: tag.clone(),
        }
        .into());
    }

    let net_mode = network.unwrap_or(NetworkMode::None);
    let id = generate_container_id();
    if state.get_container(&id).await.is_some() {
        return Err(DomainError::InvalidConfig(format!(
            "container ID collision (extremely rare): {id}"
        ))
        .into());
    }

    let mut manifest = build_execution_manifest(ManifestBuildParams {
        id: &id,
        ref_str: &ref_str,
        layer_dirs: &layer_dirs,
        command: &command,
        env: &env,
        mounts: &mounts,
        memory_limit_bytes,
        cpu_weight,
        net_mode,
        privileged,
        platform: &platform,
        name: &name,
        capture_output,
    });
    manifest
        .seal()
        .context("failed to compute execution manifest digest")?;

    if let Some(ref policy) = deps.execution_policy {
        use minibox_core::domain::PolicyDecision;
        match policy.evaluate(&manifest) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny(reason) => {
                return Err(anyhow::anyhow!("execution policy denied: {reason}"));
            }
        }
    }

    let container_dir = deps.lifecycle.containers_base.join(&id);
    let run_dir = deps.lifecycle.run_containers_base.join(&id);
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        const OWNER_RWX_PERMS: u32 = 0o700;
        builder.mode(OWNER_RWX_PERMS);
        builder.recursive(true);
        builder.create(&container_dir)?;
        builder.create(&run_dir)?;
    }

    let rootfs_layout = deps
        .lifecycle
        .filesystem
        .setup_rootfs(&layer_dirs, &container_dir)?;
    let merged_dir = rootfs_layout.merged_dir.clone();

    const DEFAULT_PIDS_MAX: u64 = 1024;
    let resource_config = ResourceConfig {
        memory_limit_bytes,
        cpu_weight,
        pids_max: Some(DEFAULT_PIDS_MAX),
        io_max_bytes_per_sec: None,
    };
    let cgroup_dir_str = {
        #[cfg(target_os = "linux")]
        if let Some(ref parent) = cgroup_parent {
            crate::container::cgroups::validate_cgroup_parent(parent)?;
            let manager = crate::container::cgroups::CgroupManager::with_root(
                &id,
                crate::container::cgroups::CgroupConfig {
                    memory_limit_bytes: resource_config.memory_limit_bytes,
                    cpu_weight: resource_config.cpu_weight,
                    pids_max: resource_config.pids_max,
                    io_max_bytes_per_sec: resource_config.io_max_bytes_per_sec,
                },
                PathBuf::from(parent),
            );
            manager.create()?;
            manager.cgroup_path().display().to_string()
        } else {
            deps.lifecycle
                .resource_limiter
                .create(&id, &resource_config)?
        }

        #[cfg(not(target_os = "linux"))]
        {
            if cgroup_parent.is_some() {
                anyhow::bail!("--cgroup-parent is only supported on Linux");
            }
            deps.lifecycle
                .resource_limiter
                .create(&id, &resource_config)?
        }
    };
    let cgroup_dir = PathBuf::from(cgroup_dir_str);

    #[cfg(target_os = "linux")]
    if privileged {
        let delegation = crate::container::cgroups::DelegationPaths {
            subtree: cgroup_dir.clone(),
            init_leaf: cgroup_dir.join("init"),
        };
        if let Err(error) = crate::container::cgroups::delegate_subtree(&delegation) {
            tracing::debug!(
                container_id = %id,
                %error,
                "cgroup delegation skipped (non-fatal)"
            );
        }
    }

    let network_config = minibox_core::domain::NetworkConfig {
        mode: net_mode,
        ..minibox_core::domain::NetworkConfig::default()
    };
    let net = NetworkLifecycle::new(deps.lifecycle.network_provider.clone());
    let _net_ns = net
        .setup(&id, &network_config)
        .await
        .context("network setup")?;
    let skip_net_ns = net_mode == NetworkMode::Host;

    let display_image = full_image.strip_prefix("library/").unwrap_or(&full_image);
    let image_label = format!("{display_image}:{tag}");
    let record = build_container_record(ContainerRecordBuildParams {
        id: &id,
        name: &name,
        image_label: &image_label,
        command: &command,
        merged_dir: &merged_dir,
        cgroup_dir: &cgroup_dir,
        rootfs_layout: &rootfs_layout,
        image: &image,
        tag: &tag,
        memory_limit_bytes,
        cpu_weight,
        network,
        env: &env,
        mounts: &mounts,
        privileged,
        platform: &platform,
        cgroup_parent: &cgroup_parent,
    });
    let spawn_command = command
        .first()
        .cloned()
        .unwrap_or_else(|| "/bin/sh".to_string());
    let spawn_args = command.iter().skip(1).cloned().collect();
    let mut container_env = vec![
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        "TERM=xterm".to_string(),
    ];
    container_env.extend(env.clone());
    container_env.extend(crate::nesting::NestingContext::from_env().child_env_vars());
    let spawn_config = ContainerSpawnConfig {
        rootfs: merged_dir.clone(),
        command: spawn_command,
        args: spawn_args,
        env: container_env,
        cgroup_path: cgroup_dir.clone().into(),
        hostname: format!("minibox-{}", &id[..8]),
        capture_output,
        hooks: ContainerHooks::default(),
        skip_network_namespace: skip_net_ns,
        mounts: mounts.clone(),
        privileged,
        image_ref: Some(image_label.clone()),
    };

    let manifest_path = container_dir.join("execution-manifest.json");
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("serialise execution manifest")?;
    std::fs::write(&manifest_path, &manifest_json)
        .with_context(|| format!("write execution manifest to {}", manifest_path.display()))?;
    manifest.manifest_path = Some(manifest_path.clone());
    let workload_digest = manifest.workload_digest.clone().unwrap_or_default();

    state.add_container(record).await;

    Ok(PreparedRun {
        id,
        spawn_config,
        image_label,
        net,
        manifest_path,
        workload_digest,
    })
}
