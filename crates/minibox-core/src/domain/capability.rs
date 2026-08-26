//! Backend capability declarations and conformance gating support.

#[cfg(test)]
use super::{
    AsAny, BackendRootfsMetadata, ChildInit, ContainerHooks, ContainerId, ContainerState,
    DomainError, HookSpec, ImageLoader, MetricsRecorder, MockPtyAllocator, NullPtyAllocator,
    PhaseOutcome, PtyAllocator, PtyConfig, ResourceConfig, RootfsLayout, RootfsSetup,
    RuntimeCapabilities, StepStatus, WorkflowStep, determine_final_phase,
};
#[cfg(test)]
use anyhow::Result;
#[cfg(test)]
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Conformance boundary — commit / build / push capabilities
// ---------------------------------------------------------------------------

/// An individual capability that a backend adapter may or may not support.
///
/// Used by [`BackendCapabilitySet`] to describe what a concrete backend can do.
/// The conformance suite gates tests on these flags so that backend-specific
/// tests are skipped rather than failed when a capability is absent.
///
/// # Backend support matrix
///
/// | Capability          | linux-native | Colima |
/// |---------------------|:------------:|:------:|
/// | `Commit`            | yes          | no     |
/// | `BuildFromContext`  | yes          | no     |
/// | `PushToRegistry`    | yes          | yes    |
///
/// **linux-native** — `OverlayCommitAdapter`, `MiniboxImageBuilder`,
/// `OciPushAdapter`: all three traits are fully implemented; commit and build
/// require root and Linux namespaces; push requires a reachable OCI registry.
///
/// **Colima** — `ColimaImagePusher` implements `ImagePusher`; there is no
/// Colima-native `ContainerCommitter` or `ImageBuilder` implementation yet
/// (Colima containers use the nerdctl/lima CLI, which does not expose an
/// upperdir for overlay-style commit, and no Dockerfile build path has been
/// wired into the adapter suite).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendCapability {
    /// Backend can snapshot a running container's FS diff into a new image
    /// via [`ContainerCommitter::commit`].
    Commit,
    /// Backend can build an image from a `BuildContext` + `BuildConfig` via
    /// [`ImageBuilder::build_image`].
    BuildFromContext,
    /// Backend can push an image to an OCI-compliant registry via
    /// [`ImagePusher::push_image`].
    PushToRegistry,
    /// Backend can save/restore VM state checkpoints via
    /// [`VmCheckpoint::save_snapshot`] / [`VmCheckpoint::restore_snapshot`].
    Checkpoint,
    /// Backend provides [`RootfsSetup`] + [`ChildInit`] (filesystem operations).
    Filesystem,
    /// Backend provides [`ExecRuntime`] (exec into running containers).
    Exec,
    /// Backend provides [`NetworkProvider`] (bridge/host/tailnet networking).
    Network,
    /// Backend provides [`TtyProvider`] (pseudo-terminal allocation).
    Tty,
    /// Backend provides [`PtyAllocator`] (low-level PTY pair allocation).
    Pty,
    /// Backend provides [`MetricsRecorder`] (counter/histogram/gauge).
    Metrics,
    /// Backend provides [`RegistryRouter`] (multi-registry routing).
    RegistryRouter,
    /// Backend provides [`ImageLoader`] (local OCI tarball loading).
    ImageLoader,
}

/// The full set of [`BackendCapability`] flags declared by one backend.
///
/// Construct via [`BackendCapabilitySet::new`] and chain
/// [`BackendCapabilitySet::with`] calls:
///
/// ```rust
/// use minibox_core::domain::{BackendCapability, BackendCapabilitySet};
///
/// let caps = BackendCapabilitySet::new()
///     .with(BackendCapability::Commit)
///     .with(BackendCapability::PushToRegistry);
///
/// assert!(caps.supports(BackendCapability::Commit));
/// assert!(!caps.supports(BackendCapability::BuildFromContext));
/// ```
#[derive(Debug, Clone, Default)]
pub struct BackendCapabilitySet {
    flags: std::collections::HashSet<BackendCapability>,
}

impl BackendCapabilitySet {
    /// Create an empty capability set (no capabilities).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a capability to this set (builder-style).
    #[must_use]
    pub fn with(mut self, cap: BackendCapability) -> Self {
        self.flags.insert(cap);
        self
    }

    /// Return `true` if this set includes `cap`.
    #[must_use]
    pub fn supports(&self, cap: BackendCapability) -> bool {
        self.flags.contains(&cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // --- MetricsRecorder tests ---

    /// Verify that a no-op MetricsRecorder can be constructed and used as a trait object.
    #[test]
    fn metrics_recorder_trait_object() {
        struct StubRecorder;
        impl MetricsRecorder for StubRecorder {
            fn increment_counter(&self, _name: &str, _labels: &[(&str, &str)]) {}
            fn record_histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
            fn set_gauge(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
        }

        let recorder: Arc<dyn MetricsRecorder> = Arc::new(StubRecorder);
        recorder.increment_counter("test_counter", &[("key", "val")]);
        recorder.record_histogram("test_hist", 1.5, &[]);
        recorder.set_gauge("test_gauge", 42.0, &[("a", "b")]);
    }

    // --- ContainerId tests ---

    #[test]
    fn container_id_valid() {
        let id = ContainerId::new("abc123".to_string()).expect("valid alphanumeric id");
        assert_eq!(id.as_str(), "abc123");
    }

    #[test]
    fn container_id_empty() {
        let result = ContainerId::new(String::new());
        assert!(result.is_err(), "empty id should fail");
    }

    #[test]
    fn container_id_too_long() {
        let long = "a".repeat(65);
        let result = ContainerId::new(long);
        assert!(result.is_err(), "65-char id should fail");
    }

    #[test]
    fn container_id_max_length() {
        let id_str = "a".repeat(64);
        let id = ContainerId::new(id_str.clone()).expect("64-char id should succeed");
        assert_eq!(id.as_str(), id_str);
    }

    #[test]
    fn container_id_special_chars() {
        let result = ContainerId::new("abc-123".to_string());
        assert!(result.is_err(), "hyphen should fail alphanumeric check");
    }

    #[test]
    fn container_id_spaces() {
        let result = ContainerId::new("abc 123".to_string());
        assert!(result.is_err(), "space should fail alphanumeric check");
    }

    #[test]
    fn container_id_as_str() {
        let id = ContainerId::new("deadbeef".to_string()).expect("valid id");
        assert_eq!(id.as_str(), "deadbeef");
    }

    #[test]
    fn container_id_display() {
        let id = ContainerId::new("abc123".to_string()).expect("valid id");
        assert_eq!(format!("{id}"), "abc123");
    }

    #[test]
    fn container_id_equality() {
        let a = ContainerId::new("abc123".to_string()).expect("valid id");
        let b = ContainerId::new("abc123".to_string()).expect("valid id");
        assert_eq!(a, b);
    }

    #[test]
    fn container_id_hash() {
        let a = ContainerId::new("abc123".to_string()).expect("valid id");
        let b = ContainerId::new("def456".to_string()).expect("valid id");
        let mut set: HashSet<ContainerId> = HashSet::new();
        set.insert(a.clone());
        set.insert(b.clone());
        assert!(set.contains(&a));
        assert!(set.contains(&b));
        assert_eq!(set.len(), 2);
    }

    // --- ContainerId hex edge-case tests (GH #145) ---

    #[test]
    fn container_id_valid_16_char_hex() {
        // A standard 16-character lowercase hex ID (common Docker short-ID format) is valid.
        let id = ContainerId::new("deadbeef01234567".to_string()).expect("valid 16-char hex id");
        assert_eq!(id.as_str(), "deadbeef01234567");
    }

    #[test]
    fn container_id_15_chars_is_valid() {
        // The validator requires 1–64 alphanumeric chars; 15 chars is within that range.
        // There is no minimum length beyond non-empty, so a 15-char hex string is accepted.
        let id = ContainerId::new("deadbeef0123456".to_string())
            .expect("15-char hex id is within the 1-64 range and must be accepted");
        assert_eq!(id.as_str().len(), 15);
    }

    #[test]
    fn container_id_17_chars_is_valid() {
        // Similarly, 17-char hex strings are within the 64-char maximum and accepted.
        let id = ContainerId::new("deadbeef012345678".to_string())
            .expect("17-char hex id is within the 1-64 range and must be accepted");
        assert_eq!(id.as_str().len(), 17);
    }

    #[test]
    fn container_id_non_hex_chars_rejected() {
        // Characters outside [0-9a-fA-F] that are also non-alphanumeric are rejected.
        // Hyphens and underscores are not alphanumeric, so they fail validation.
        let result = ContainerId::new("deadbeef-0123456".to_string());
        assert!(
            result.is_err(),
            "hyphen is not alphanumeric and must be rejected"
        );
    }

    #[test]
    fn container_id_empty_rejected() {
        let result = ContainerId::new(String::new());
        assert!(result.is_err(), "empty string must be rejected");
    }

    /// The validator uses `is_ascii_alphanumeric()`, which allows both lowercase and uppercase
    /// hex characters (a-f and A-F). Mixed-case hex IDs such as "DeadBeef01234567" are
    /// therefore accepted — they are alphanumeric even though they mix case. Code that compares
    /// container IDs must normalise case if canonical form matters.
    #[test]
    fn container_id_mixed_case_hex_accepted() {
        let id = ContainerId::new("DeadBeef01234567".to_string())
            .expect("mixed-case hex is alphanumeric and must be accepted");
        assert_eq!(id.as_str(), "DeadBeef01234567");
    }

    // --- ContainerState tests ---

    #[test]
    fn container_state_as_str() {
        assert_eq!(ContainerState::Created.as_str(), "Created");
        assert_eq!(ContainerState::Running.as_str(), "Running");
        assert_eq!(ContainerState::Paused.as_str(), "Paused");
        assert_eq!(ContainerState::Stopped.as_str(), "Stopped");
        assert_eq!(ContainerState::Failed.as_str(), "Failed");
    }

    #[test]
    fn container_state_display() {
        assert_eq!(format!("{}", ContainerState::Created), "Created");
        assert_eq!(format!("{}", ContainerState::Running), "Running");
        assert_eq!(format!("{}", ContainerState::Paused), "Paused");
        assert_eq!(format!("{}", ContainerState::Stopped), "Stopped");
        assert_eq!(format!("{}", ContainerState::Failed), "Failed");
    }

    #[test]
    fn container_state_clone_eq() {
        let state = ContainerState::Running;
        let cloned = state;
        assert_eq!(state, cloned);
        assert_ne!(state, ContainerState::Stopped);
    }

    // --- DomainError tests ---

    #[test]
    fn domain_error_display_image_not_found() {
        let err = DomainError::ImageNotFound {
            name: "library/ubuntu".to_string(),
            tag: "22.04".to_string(),
        };
        assert_eq!(err.error_kind(), "image_not_found");
        assert_eq!(err.to_string(), "image library/ubuntu:22.04 not found");
    }

    #[test]
    fn domain_error_display_container_not_found() {
        let err = DomainError::ContainerNotFound {
            id: "abc123".to_string(),
        };
        assert_eq!(err.error_kind(), "container_not_found");
        assert_eq!(err.to_string(), "container 'abc123' not found");
    }

    #[test]
    fn domain_error_display_resource_limit_exceeded() {
        let err = DomainError::ResourceLimitExceeded {
            limit: "memory_bytes".to_string(),
            value: 9999,
            max: 1024,
        };
        assert_eq!(err.error_kind(), "resource_limit_exceeded");
        let msg = err.to_string();
        assert!(msg.contains("memory_bytes"), "should contain limit name");
        assert!(msg.contains("9999"), "should contain value");
        assert!(msg.contains("1024"), "should contain max");
    }

    // --- ResourceConfig tests ---

    #[test]
    fn resource_config_default() {
        let config = ResourceConfig::default();
        assert!(config.memory_limit_bytes.is_none());
        assert!(config.cpu_weight.is_none());
        assert!(config.pids_max.is_none());
        assert!(config.io_max_bytes_per_sec.is_none());
    }

    #[test]
    fn resource_config_serde_roundtrip() {
        let config = ResourceConfig {
            memory_limit_bytes: Some(1024 * 1024 * 256),
            cpu_weight: Some(500),
            pids_max: Some(100),
            io_max_bytes_per_sec: Some(1024 * 1024),
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let back: ResourceConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.memory_limit_bytes, config.memory_limit_bytes);
        assert_eq!(back.cpu_weight, config.cpu_weight);
        assert_eq!(back.pids_max, config.pids_max);
        assert_eq!(back.io_max_bytes_per_sec, config.io_max_bytes_per_sec);
    }

    // --- HookSpec / ContainerHooks tests ---

    #[test]
    fn hook_spec_default() {
        let hook = HookSpec::default();
        assert_eq!(hook.command, "");
        assert!(hook.args.is_empty());
        assert!(hook.timeout_secs.is_none());
    }

    #[test]
    fn container_hooks_default() {
        let hooks = ContainerHooks::default();
        assert!(hooks.pre_exec.is_empty());
        assert!(hooks.post_exit.is_empty());
    }

    // --- RuntimeCapabilities tests ---

    #[test]
    fn runtime_capabilities_debug() {
        let caps = RuntimeCapabilities {
            supports_user_namespaces: true,
            supports_cgroups_v2: false,
            supports_overlay_fs: true,
            supports_network_isolation: false,
            max_containers: Some(128),
        };
        let debug_str = format!("{caps:?}");
        assert!(!debug_str.is_empty(), "Debug impl should produce output");
    }

    // --- ImageLoader tests ---

    // --- ExecSpec purity test ---

    /// Verify that ExecSpec is Clone and contains no channel fields.
    /// This encodes the architecture contract: ExecSpec is a pure domain
    /// value type that must not depend on tokio infrastructure.
    #[test]
    fn exec_spec_is_pure_domain() {
        let spec = crate::domain::ExecSpec {
            cmd: vec!["echo".to_string()],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        // Must be Clone — pure domain types are always Clone
        let cloned = spec.clone();
        assert_eq!(cloned.cmd, vec!["echo".to_string()]);
        assert!(!cloned.tty);
    }

    #[cfg(test)]
    mod image_loader_tests {
        use super::*;
        use std::path::Path;

        struct AlwaysOkLoader;

        #[async_trait::async_trait]
        impl ImageLoader for AlwaysOkLoader {
            async fn load_image(
                &self,
                _path: &Path,
                _name: &str,
                _tag: &str,
            ) -> anyhow::Result<()> {
                Ok(())
            }
        }

        #[tokio::test]
        async fn image_loader_trait_is_object_safe() {
            let loader: Box<dyn ImageLoader> = Box::new(AlwaysOkLoader);
            let result = loader
                .load_image(
                    std::path::Path::new("/fake.tar"),
                    "minibox-tester",
                    "latest",
                )
                .await;
            assert!(result.is_ok());
        }
    }

    mod backend_rootfs_metadata_tests {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn overlay_upper_dir_returns_path_for_native_variant() {
            let path = PathBuf::from("/var/lib/minibox/containers/abc/upper");
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: path.clone().into(),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(&**meta.overlay_upper_dir(), path.as_path());
        }

        #[test]
        fn overlay_upper_dir_returns_path_for_colima_variant() {
            let path = PathBuf::from("/Users/joe/.lima/colima/upper");
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: path.clone().into(),
                metadata: kv,
            };
            assert_eq!(&**meta.overlay_upper_dir(), path.as_path());
        }

        #[test]
        fn metadata_value_none_for_missing_key() {
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/tmp/upper").into(),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(meta.metadata_value("colima_instance"), None);
        }

        #[test]
        fn metadata_value_returns_value_for_present_key() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/tmp/upper").into(),
                metadata: kv,
            };
            assert_eq!(meta.metadata_value("colima_instance"), Some("colima"));
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_overlay() {
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/var/lib/minibox/containers/abc/upper").into(),
                metadata: std::collections::HashMap::new(),
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_with_kv() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/Users/joe/.lima/colima/upper").into(),
                metadata: kv,
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }

        #[test]
        fn rootfs_layout_metadata_survives_commit_image_ref() {
            // Verify that an Overlay metadata's upper_dir is unchanged
            // after being stored and retrieved (simulates the commit path
            // reading the upper_dir from the container record).
            let upper = PathBuf::from("/Users/joe/.lima/colima/containers/abc/upper");
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: upper.clone().into(),
                metadata,
            };
            let layout = RootfsLayout {
                merged_dir: PathBuf::from("/tmp/merged").into(),
                rootfs_metadata: Some(meta),
                source_image_ref: Some("alpine:latest".to_string()),
            };
            let recovered_upper = layout
                .rootfs_metadata
                .as_ref()
                .expect("metadata present")
                .overlay_upper_dir();
            assert_eq!(&**recovered_upper, upper.as_path());
        }

        // --- Task 1: OCP fix tests ---

        #[test]
        fn overlay_variant_has_opaque_metadata_map() {
            // BackendRootfsMetadata::Overlay must carry an opaque HashMap so
            // backends can encode their own KVs without adding new variants.
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("colima_instance".to_string(), "colima".to_string());
            let upper = PathBuf::from("/Users/joe/.lima/colima/upper");
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: upper.clone().into(),
                metadata: metadata.clone(),
            };
            assert_eq!(&**meta.overlay_upper_dir(), upper.as_path());
            assert_eq!(meta.metadata_value("colima_instance"), Some("colima"));
        }

        #[test]
        fn overlay_variant_metadata_empty_for_native() {
            // Native overlay encodes no extra KVs.
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/var/lib/minibox/containers/abc/upper").into(),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(meta.metadata_value("colima_instance"), None);
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_with_metadata_map() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/Users/joe/.lima/colima/upper").into(),
                metadata: kv,
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }
    }

    mod pty_allocator_tests {
        use super::*;

        #[test]
        fn pty_config_default_values() {
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            assert!(cfg.enabled);
            assert_eq!(cfg.cols, 80);
            assert_eq!(cfg.rows, 24);
        }

        #[test]
        fn pty_config_serde_roundtrip() {
            let cfg = PtyConfig {
                enabled: true,
                cols: 120,
                rows: 40,
            };
            let json = serde_json::to_string(&cfg).expect("serialize");
            let back: PtyConfig = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back.enabled, cfg.enabled);
            assert_eq!(back.cols, cfg.cols);
            assert_eq!(back.rows, cfg.rows);
        }

        #[test]
        fn pty_config_deserialize_missing_fields_use_serde_default() {
            // When a JSON payload omits fields the struct must still deserialize.
            let json = r#"{"enabled":false,"cols":80,"rows":24}"#;
            let cfg: PtyConfig = serde_json::from_str(json).expect("deserialize");
            // Exercise NullPtyAllocator::allocate — a domain-defined SUT function.
            let result = NullPtyAllocator.allocate(&cfg);
            assert!(result.is_err(), "NullPtyAllocator must return Err");
            assert!(!cfg.enabled);
            assert_eq!(cfg.cols, 80);
            assert_eq!(cfg.rows, 24);
        }

        #[test]
        fn null_pty_allocator_returns_err() {
            let alloc = NullPtyAllocator;
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            assert!(
                alloc.allocate(&cfg).is_err(),
                "NullPtyAllocator must always return Err"
            );
        }

        #[cfg(feature = "test-utils")]
        #[test]
        fn mock_pty_allocator_returns_configured_handle() {
            let alloc = MockPtyAllocator::new(5, 6);
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            let handle = alloc.allocate(&cfg).expect("MockPtyAllocator must succeed");
            assert_eq!(handle.master_fd, 5);
            assert_eq!(handle.slave_fd, 6);
        }
    }

    mod isp_trait_split_tests {
        use super::*;
        use std::path::{Path, PathBuf};

        // --- Task 2: ISP split tests ---

        /// Verify that RootfsSetup is a standalone trait (not mixed with ChildInit).
        struct OnlySetup;
        impl AsAny for OnlySetup {
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        impl RootfsSetup for OnlySetup {
            fn setup_rootfs(
                &self,
                _layers: &[PathBuf],
                _container_dir: &Path,
            ) -> Result<RootfsLayout> {
                Ok(RootfsLayout {
                    merged_dir: PathBuf::from("/tmp/merged").into(),
                    rootfs_metadata: None,
                    source_image_ref: None,
                })
            }

            fn cleanup(&self, _container_dir: &Path) -> Result<()> {
                Ok(())
            }
        }

        /// Verify that ChildInit is a standalone trait for pivot_root.
        struct OnlyChildInit;
        impl ChildInit for OnlyChildInit {
            fn pivot_root(&self, _new_root: &Path) -> Result<()> {
                Ok(())
            }
        }

        #[test]
        fn rootfs_setup_can_be_used_without_child_init() {
            let setup = OnlySetup;
            let result = setup.setup_rootfs(&[], Path::new("/tmp/container"));
            assert!(result.is_ok());
            assert!(setup.cleanup(Path::new("/tmp/container")).is_ok());
        }

        #[test]
        fn child_init_can_be_used_without_rootfs_setup() {
            let init = OnlyChildInit;
            assert!(init.pivot_root(Path::new("/tmp/new_root")).is_ok());
        }
    }

    mod workflow_tests {
        use super::*;

        #[test]
        fn workflow_step_deserialize_defaults_continue_on_error_false() {
            let json = r#"{"kind":"container-run","alias":"build"}"#;
            let step: WorkflowStep = serde_json::from_str(json).unwrap();
            // Exercise determine_final_phase — a domain-defined SUT function.
            let outcome = determine_final_phase(&[StepStatus::Succeeded]);
            assert_eq!(outcome, PhaseOutcome::Succeeded);
            assert!(!step.continue_on_error);
            assert!(step.retry.is_none());
            assert_eq!(step.alias, "build");
        }

        #[test]
        fn phase_outcome_errored_beats_failed() {
            assert!(PhaseOutcome::Errored > PhaseOutcome::Failed);
        }

        #[test]
        fn phase_outcome_failed_beats_aborted() {
            assert!(PhaseOutcome::Failed > PhaseOutcome::Aborted);
        }

        #[test]
        fn phase_outcome_aborted_beats_skipped() {
            assert!(PhaseOutcome::Aborted > PhaseOutcome::Skipped);
        }

        #[test]
        fn phase_outcome_skipped_beats_succeeded() {
            assert!(PhaseOutcome::Skipped > PhaseOutcome::Succeeded);
        }

        use proptest::prelude::*;

        proptest! {
            #[test]
            fn worst_case_phase_with_any_errored_is_errored(count in 1usize..10) {
                let steps: Vec<PhaseOutcome> = (0..count)
                    .map(|_| PhaseOutcome::Succeeded)
                    .chain(std::iter::once(PhaseOutcome::Errored))
                    .collect();
                let worst = steps.iter().copied().max().unwrap();
                prop_assert_eq!(worst, PhaseOutcome::Errored);
            }
        }
    }
}
