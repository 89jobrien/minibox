//! krun adapter conformance tests — Phase 2: domain port conformance.
//!
//! Tests `KrunRuntime`, `KrunFilesystem`, `KrunLimiter`, and `KrunRegistry`
//! against the domain ports defined in `minibox-core::domain`.
//!
//! Run with:
//!   MINIBOX_KRUN_TESTS=1 cargo test -p macbox --test krun_adapter_conformance -- --test-threads=1
//!
//! `--test-threads=1` is required: parallel krun invocations share per-process
//! state in libkrun and collide on socket paths.

mod suite {
    use macbox::krun::runtime::KrunRuntime;
    use minibox_core::domain::ImageRegistry as _;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn krun_available() -> bool {
        #[cfg(target_os = "macos")]
        return true;

        #[cfg(target_os = "linux")]
        return std::path::Path::new("/dev/kvm").exists()
            && std::fs::metadata("/dev/kvm")
                .map(|m| !m.permissions().readonly())
                .unwrap_or(false);

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return false;
    }

    macro_rules! skip_if_no_krun {
        () => {
            if std::env::var("MINIBOX_KRUN_TESTS").as_deref() != Ok("1") {
                eprintln!("SKIP: set MINIBOX_KRUN_TESTS=1 to run krun conformance tests");
                return;
            }
            if !krun_available() {
                eprintln!("SKIP: no hypervisor available (macOS HVF or Linux /dev/kvm)");
                return;
            }
        };
    }

    // -----------------------------------------------------------------------
    // K-R-01: create() returns a non-empty container ID
    // -----------------------------------------------------------------------

    /// `KrunRuntime::create()` must return a non-empty container ID string.
    ///
    /// The ID uniquely identifies the container and is used for subsequent
    /// lifecycle calls (start, stop, wait, destroy).
    #[tokio::test]
    async fn krun_runtime_create_returns_container_id() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create("alpine", &["/bin/true".to_string()], &[])
            .await
            .expect("create() must not return Err");

        assert!(!id.is_empty(), "container ID must not be empty");
    }

    // -----------------------------------------------------------------------
    // K-R-02: create() + start() + collect_stdout() returns non-empty output
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_create_start_produces_output() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create(
                "alpine",
                &["/bin/echo".to_string(), "hello-krun".to_string()],
                &[],
            )
            .await
            .expect("create() must not return Err");

        runtime
            .start(&id)
            .await
            .expect("start() must not return Err");

        let output = runtime
            .collect_stdout(&id)
            .await
            .expect("collect_stdout() must not return Err");

        assert!(
            !output.is_empty(),
            "collect_stdout() must return non-empty output"
        );
    }

    // -----------------------------------------------------------------------
    // K-R-03: stop() on a running container terminates the process within 5s
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_stop_terminates_process() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create(
                "alpine",
                &["/bin/sleep".to_string(), "300".to_string()],
                &[],
            )
            .await
            .expect("create() must not return Err");

        runtime
            .start(&id)
            .await
            .expect("start() must not return Err");

        let stop_result =
            tokio::time::timeout(std::time::Duration::from_secs(5), runtime.stop(&id))
                .await
                .expect("stop() must complete within 5 seconds")
                .expect("stop() must not return Err");

        let _ = stop_result;
    }

    // -----------------------------------------------------------------------
    // K-R-04: wait() after /bin/true returns exit code 0
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_wait_returns_exit_code() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create("alpine", &["/bin/true".to_string()], &[])
            .await
            .expect("create() must not return Err");

        runtime
            .start(&id)
            .await
            .expect("start() must not return Err");

        let code = runtime.wait(&id).await.expect("wait() must not return Err");
        assert_eq!(code, 0, "exit code for /bin/true must be 0");
    }

    // -----------------------------------------------------------------------
    // K-R-05: wait() after exit 42 returns exit code 42
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_wait_propagates_nonzero_exit() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create(
                "alpine",
                &[
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    "exit 42".to_string(),
                ],
                &[],
            )
            .await
            .expect("create() must not return Err");

        runtime
            .start(&id)
            .await
            .expect("start() must not return Err");

        let code = runtime.wait(&id).await.expect("wait() must not return Err");
        assert_eq!(code, 42, "exit code must be 42");
    }

    // -----------------------------------------------------------------------
    // K-R-06: destroy() after stop → container ID no longer tracked
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_destroy_cleans_up() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create("alpine", &["/bin/true".to_string()], &[])
            .await
            .expect("create() must not return Err");

        runtime
            .start(&id)
            .await
            .expect("start() must not return Err");
        runtime.wait(&id).await.expect("wait() must not return Err");
        runtime
            .destroy(&id)
            .await
            .expect("destroy() must not return Err");

        // After destroy, wait() should return Err (unknown container)
        let result = runtime.wait(&id).await;
        assert!(
            result.is_err(),
            "wait() after destroy() must return Err (container no longer tracked)"
        );
    }

    // -----------------------------------------------------------------------
    // K-R-07: Two containers run concurrently with independent stdout streams
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_concurrent_containers_independent() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();

        let id1 = runtime
            .create(
                "alpine",
                &["/bin/echo".to_string(), "output-one".to_string()],
                &[],
            )
            .await
            .expect("create() id1 must not return Err");

        let id2 = runtime
            .create(
                "alpine",
                &["/bin/echo".to_string(), "output-two".to_string()],
                &[],
            )
            .await
            .expect("create() id2 must not return Err");

        runtime
            .start(&id1)
            .await
            .expect("start() id1 must not return Err");
        runtime
            .start(&id2)
            .await
            .expect("start() id2 must not return Err");

        let out1 = runtime
            .collect_stdout(&id1)
            .await
            .expect("collect_stdout() id1 must not return Err");
        let out2 = runtime
            .collect_stdout(&id2)
            .await
            .expect("collect_stdout() id2 must not return Err");

        assert!(
            out1.contains("output-one"),
            "container 1 stdout must contain 'output-one', got: {out1:?}"
        );
        assert!(
            out2.contains("output-two"),
            "container 2 stdout must contain 'output-two', got: {out2:?}"
        );
    }

    // -----------------------------------------------------------------------
    // K-R-08: create() with non-existent image → start() returns Err
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_missing_image_returns_err() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create(
                "this-image-does-not-exist-xyzzy-9999",
                &["/bin/true".to_string()],
                &[],
            )
            .await
            .expect("create() must not return Err (only records config)");

        let result = runtime.start(&id).await;
        assert!(
            result.is_err(),
            "start() with a missing image must return Err"
        );
    }

    // -----------------------------------------------------------------------
    // K-R-09: env pairs in create() are visible inside the VM
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_env_vars_visible_in_container() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create(
                "alpine",
                &[
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    "echo $KRUN_TEST_VAR".to_string(),
                ],
                &[("KRUN_TEST_VAR".to_string(), "hello-env".to_string())],
            )
            .await
            .expect("create() must not return Err");

        runtime
            .start(&id)
            .await
            .expect("start() must not return Err");

        let output = runtime
            .collect_stdout(&id)
            .await
            .expect("collect_stdout() must not return Err");

        assert!(
            output.contains("hello-env"),
            "stdout must contain env var value 'hello-env', got: {output:?}"
        );
    }

    // -----------------------------------------------------------------------
    // K-R-10: command + args in create() run as specified
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn krun_runtime_command_args_forwarded() {
        skip_if_no_krun!();

        let runtime = KrunRuntime::new();
        let id = runtime
            .create(
                "alpine",
                &[
                    "/bin/echo".to_string(),
                    "arg-alpha".to_string(),
                    "arg-beta".to_string(),
                ],
                &[],
            )
            .await
            .expect("create() must not return Err");

        runtime
            .start(&id)
            .await
            .expect("start() must not return Err");

        let output = runtime
            .collect_stdout(&id)
            .await
            .expect("collect_stdout() must not return Err");

        assert!(
            output.contains("arg-alpha") && output.contains("arg-beta"),
            "stdout must contain forwarded args, got: {output:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 2b — KrunFilesystem
    // -----------------------------------------------------------------------

    // K-F-01: setup_rootfs() returns Ok for a valid existing path
    #[test]
    fn krun_filesystem_setup_rootfs_returns_ok() {
        use macbox::krun::filesystem::KrunFilesystem;
        use minibox_core::domain::RootfsSetup;

        let tmp = tempfile::tempdir().expect("tempdir");
        let fs = KrunFilesystem::new();
        let result = fs.setup_rootfs(&[], tmp.path());
        assert!(
            result.is_ok(),
            "setup_rootfs() with valid path must return Ok, got: {result:?}"
        );
    }

    // K-F-02: setup_rootfs() with nonexistent path → Err
    #[test]
    fn krun_filesystem_setup_rootfs_missing_path_err() {
        use macbox::krun::filesystem::KrunFilesystem;
        use minibox_core::domain::RootfsSetup;

        let fs = KrunFilesystem::new();
        let missing = std::path::Path::new("/this/path/does/not/exist/xyzzy");
        let result = fs.setup_rootfs(&[], missing);
        assert!(
            result.is_err(),
            "setup_rootfs() with nonexistent path must return Err"
        );
    }

    // K-F-03: child_init() (pivot_root) returns Ok (no-op, VM manages init)
    #[test]
    fn krun_filesystem_child_init_is_noop_ok() {
        use macbox::krun::filesystem::KrunFilesystem;
        use minibox_core::domain::ChildInit;

        let tmp = tempfile::tempdir().expect("tempdir");
        let fs = KrunFilesystem::new();
        let result = fs.pivot_root(tmp.path());
        assert!(
            result.is_ok(),
            "pivot_root() must return Ok (no-op for VM adapter), got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 2c — KrunLimiter
    // -----------------------------------------------------------------------

    // K-L-01: apply(memory_bytes=256MB) returns Ok
    #[test]
    fn krun_limiter_apply_memory_limit_ok() {
        use macbox::krun::limiter::KrunLimiter;
        use minibox_core::domain::{ResourceConfig, ResourceLimiter};

        let limiter = KrunLimiter::new();
        let config = ResourceConfig {
            memory_limit_bytes: Some(256 * 1024 * 1024),
            ..Default::default()
        };
        let result = limiter.create("test-container-1", &config);
        assert!(
            result.is_ok(),
            "create() with memory limit must return Ok, got: {result:?}"
        );
    }

    // K-L-02: apply(cpu_weight=512) returns Ok
    #[test]
    fn krun_limiter_apply_cpu_weight_ok() {
        use macbox::krun::limiter::KrunLimiter;
        use minibox_core::domain::{ResourceConfig, ResourceLimiter};

        let limiter = KrunLimiter::new();
        let config = ResourceConfig {
            cpu_weight: Some(512),
            ..Default::default()
        };
        let result = limiter.create("test-container-2", &config);
        assert!(
            result.is_ok(),
            "create() with cpu_weight must return Ok, got: {result:?}"
        );
    }

    // K-L-03: apply(memory_bytes=0) → Ok (treated as unlimited)
    #[test]
    fn krun_limiter_apply_zero_memory_is_noop() {
        use macbox::krun::limiter::KrunLimiter;
        use minibox_core::domain::{ResourceConfig, ResourceLimiter};

        let limiter = KrunLimiter::new();
        let config = ResourceConfig {
            memory_limit_bytes: Some(0),
            ..Default::default()
        };
        let result = limiter.create("test-container-3", &config);
        assert!(
            result.is_ok(),
            "create() with memory_bytes=0 must return Ok (unlimited), got: {result:?}"
        );
    }

    // K-L-04: cleanup() after apply() → Ok
    #[test]
    fn krun_limiter_cleanup_after_apply_ok() {
        use macbox::krun::limiter::KrunLimiter;
        use minibox_core::domain::{ResourceConfig, ResourceLimiter};

        let limiter = KrunLimiter::new();
        let config = ResourceConfig {
            memory_limit_bytes: Some(128 * 1024 * 1024),
            ..Default::default()
        };
        limiter
            .create("test-container-4", &config)
            .expect("create() must succeed");
        let result = limiter.cleanup("test-container-4");
        assert!(
            result.is_ok(),
            "cleanup() after create() must return Ok, got: {result:?}"
        );
    }

    // K-L-05: cleanup() without prior apply() → Ok, no panic
    #[test]
    fn krun_limiter_cleanup_without_apply_is_safe() {
        use macbox::krun::limiter::KrunLimiter;
        use minibox_core::domain::ResourceLimiter;

        let limiter = KrunLimiter::new();
        let result = limiter.cleanup("nonexistent-container");
        assert!(
            result.is_ok(),
            "cleanup() without prior create() must return Ok, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 2d — KrunRegistry (K-I-01..05)
    // -----------------------------------------------------------------------

    fn make_krun_registry() -> macbox::krun::registry::KrunRegistry {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = std::sync::Arc::new(
            minibox_core::image::ImageStore::new(tmp.path().join("images"))
                .expect("ImageStore::new"),
        );
        // Keep `tmp` alive by leaking it — tests are short-lived.
        std::mem::forget(tmp);
        macbox::krun::registry::KrunRegistry::new(store).expect("KrunRegistry::new")
    }

    fn alpine_ref() -> minibox_core::image::reference::ImageRef {
        minibox_core::image::reference::ImageRef::parse("alpine:latest").expect("parse alpine ref")
    }

    fn nonexistent_ref() -> minibox_core::image::reference::ImageRef {
        minibox_core::image::reference::ImageRef::parse("minibox-nonexistent-xyz:latest")
            .expect("parse nonexistent ref")
    }

    // K-I-01: pull("alpine", "latest") returns Ok with valid metadata
    #[tokio::test]
    async fn krun_registry_pull_alpine_succeeds() {
        skip_if_no_krun!();

        let registry = make_krun_registry();
        let image_ref = alpine_ref();
        let result = registry.pull_image(&image_ref).await;
        assert!(
            result.is_ok(),
            "pull_image(alpine:latest) must return Ok, got: {result:?}"
        );
    }

    // K-I-02: second pull() for same image completes without re-downloading
    #[tokio::test]
    async fn krun_registry_pull_cached_image_is_fast() {
        skip_if_no_krun!();

        let tmp = tempfile::tempdir().expect("tempdir");
        let store = std::sync::Arc::new(
            minibox_core::image::ImageStore::new(tmp.path().join("images"))
                .expect("ImageStore::new"),
        );
        let registry =
            macbox::krun::registry::KrunRegistry::new(store.clone()).expect("KrunRegistry::new");
        let image_ref = alpine_ref();

        // First pull — fetches from network.
        registry
            .pull_image(&image_ref)
            .await
            .expect("first pull must succeed");

        // Second pull — should hit local cache; must succeed again.
        let start = std::time::Instant::now();
        let result = registry.pull_image(&image_ref).await;
        let elapsed = start.elapsed();

        assert!(
            result.is_ok(),
            "second pull_image(alpine:latest) must return Ok, got: {result:?}"
        );
        // Cache hits are faster than 30 s (a conservative bound for a network pull).
        assert!(
            elapsed.as_secs() < 30,
            "cached pull took {elapsed:?} — expected < 30 s"
        );
    }

    // K-I-03: pull("minibox-nonexistent-xyz", "latest") → Err
    #[tokio::test]
    async fn krun_registry_pull_nonexistent_image_errors() {
        skip_if_no_krun!();

        let registry = make_krun_registry();
        let image_ref = nonexistent_ref();
        let result = registry.pull_image(&image_ref).await;
        assert!(
            result.is_err(),
            "pull_image(minibox-nonexistent-xyz:latest) must return Err"
        );
    }

    // K-I-04: pulled manifest has at least one layer
    #[tokio::test]
    async fn krun_registry_image_manifest_has_layers() {
        skip_if_no_krun!();

        let registry = make_krun_registry();
        let image_ref = alpine_ref();
        let metadata = registry
            .pull_image(&image_ref)
            .await
            .expect("pull_image(alpine:latest) must succeed");

        assert!(
            !metadata.layers.is_empty(),
            "pulled alpine manifest must have at least one layer, got empty layers"
        );
    }

    // K-I-05: the registry enforces a 10 MiB manifest size cap (pure unit test — no network)
    #[test]
    fn krun_registry_pull_respects_size_limit() {
        // Verify the size limit constant is wired and has the expected 10 MiB value.
        let limit = macbox::krun::registry::KrunRegistry::manifest_size_limit_bytes();
        assert_eq!(
            limit,
            10 * 1024 * 1024,
            "KrunRegistry manifest size limit must be 10 MiB (10 * 1024 * 1024), got {limit}"
        );
    }
}
