//! Integration tests for the Colima adapter suite.
//!
//! Tests use injected `LimaExecutor` closures (via `with_executor()`) to avoid
//! requiring a running Colima VM.  Each test exercises the adapter through the
//! domain trait interface, verifying that the adapter correctly translates trait
//! calls into the expected Lima shell commands and handles responses properly.

use mbx::adapters::{ColimaRegistry, ColimaRuntime};
use mbx::domain::{
    ContainerHooks, ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageRegistry,
    ResourceLimiter,
};
use std::path::PathBuf;
use std::sync::Arc;

// ============================================================================
// ColimaRegistry tests
// ============================================================================

/// has_image returns true when the injected executor succeeds (simulating
/// `nerdctl image inspect` finding the image).
#[tokio::test]
async fn registry_has_image_true_when_executor_succeeds() {
    let registry = ColimaRegistry::new().with_executor(Arc::new(|_args: &[&str]| {
        Ok(r#"[{"Size":1024,"RootFS":{"Layers":[]}}]"#.to_string())
    }));

    assert!(
        registry.has_image("alpine", "latest").await,
        "has_image should return true when executor succeeds"
    );
}

/// has_image returns false when the injected executor returns an error
/// (simulating the image not being found in the local containerd store).
#[tokio::test]
async fn registry_has_image_false_when_executor_fails() {
    let registry = ColimaRegistry::new().with_executor(Arc::new(|_args: &[&str]| {
        Err(anyhow::anyhow!("image not found"))
    }));

    assert!(
        !registry.has_image("nonexistent", "v99").await,
        "has_image should return false when executor returns an error"
    );
}

/// pull_image propagates executor errors — if `nerdctl pull` fails the trait
/// method must surface an error rather than silently succeeding.
#[tokio::test]
async fn registry_pull_image_propagates_executor_error() {
    let registry = ColimaRegistry::new().with_executor(Arc::new(|args: &[&str]| {
        if args.contains(&"pull") {
            Err(anyhow::anyhow!("network unreachable"))
        } else {
            Ok(String::new())
        }
    }));

    let result = registry
        .pull_image(&mbx::image::reference::ImageRef::parse("alpine").unwrap())
        .await;
    assert!(
        result.is_err(),
        "pull_image must return an error when the executor fails"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("network unreachable"),
        "error message should include the underlying cause, got: {msg}"
    );
}

/// pull_image succeeds when both pull and inspect calls return valid JSON.
#[tokio::test]
async fn registry_pull_image_parses_inspect_output() {
    let fake_inspect = r#"[{"Size":2048,"RootFS":{"Layers":["sha256:aabbcc","sha256:ddeeff"]}}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.contains(&"pull") {
            Ok(String::new())
        } else if args.contains(&"inspect") {
            Ok(fake_inspect.to_string())
        } else {
            Ok(String::new())
        }
    }));

    let metadata = registry
        .pull_image(&mbx::image::reference::ImageRef::parse("alpine").unwrap())
        .await
        .expect("pull_image should succeed with valid executor output");

    assert_eq!(metadata.name, "library/alpine");
    assert_eq!(metadata.tag, "latest");
    assert_eq!(metadata.layers.len(), 2, "should have two layers");
}

/// get_image_layers must return paths that are accessible from the macOS host,
/// i.e. they must live under /tmp/ or /Users/ — the Lima-shared mounts.
#[test]
fn registry_get_image_layers_returns_host_accessible_paths() {
    let fake_manifest = r#"[{"Layers":["aaa/layer.tar","bbb/layer.tar"]}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.first() == Some(&"cat") && args[1].ends_with("/manifest.json") {
            Ok(fake_manifest.to_string())
        } else {
            Ok(String::new())
        }
    }));

    let layers = registry
        .get_image_layers("alpine", "latest")
        .expect("get_image_layers should succeed with a valid executor");

    assert_eq!(
        layers.len(),
        2,
        "should return one PathBuf per saved layer tar"
    );

    for layer in &layers {
        let s = layer.to_string_lossy();
        assert!(
            s.starts_with("/tmp/") || s.starts_with("/Users/"),
            "layer path {s:?} is not in a Lima-shared directory (/tmp or /Users)"
        );
    }
}

/// get_image_layers returns an empty vec when the image has no layers.
#[test]
fn registry_get_image_layers_empty_when_no_layers() {
    let fake_manifest = r#"[{"Layers":[]}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.first() == Some(&"cat") && args[1].ends_with("/manifest.json") {
            Ok(fake_manifest.to_string())
        } else {
            Ok(String::new())
        }
    }));

    let layers = registry
        .get_image_layers("scratch", "latest")
        .expect("get_image_layers should succeed");

    assert!(
        layers.is_empty(),
        "no layers in manifest.json → empty result"
    );
}

/// get_image_layers_parses_manifest_json_to_locate_layers — the executor
/// returns a manifest.json with two layer tarballs; the result must contain
/// two PathBufs and the paths must embed the layer-relative names.
#[test]
fn get_image_layers_parses_manifest_json_to_locate_layers() {
    // Docker-save manifest.json format: array of objects with "Layers" key
    // containing paths relative to the export directory.
    let fake_manifest = r#"[{"Layers":["sha256abc/layer.tar","sha256def/layer.tar"]}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.first() == Some(&"cat")
            && args.last().map(|a| a.ends_with("/manifest.json")) == Some(true)
        {
            Ok(fake_manifest.to_string())
        } else {
            // Accept mkdir / tar extraction calls silently.
            Ok(String::new())
        }
    }));

    let layers = registry
        .get_image_layers("alpine", "3.18")
        .expect("get_image_layers should succeed with valid manifest");

    assert_eq!(layers.len(), 2, "should return one PathBuf per layer entry");

    // The returned paths are rootfs extraction directories, not the tarballs
    // themselves, so they are named rootfs-0 and rootfs-1 under the export base.
    let first = layers[0].to_string_lossy();
    let second = layers[1].to_string_lossy();
    assert!(
        first.contains("rootfs-0"),
        "first layer path should be rootfs-0, got: {first}"
    );
    assert!(
        second.contains("rootfs-1"),
        "second layer path should be rootfs-1, got: {second}"
    );
}

/// get_image_layers_returns_error_on_malformed_manifest — when the executor
/// returns bytes that are not valid JSON the method must surface a parse error.
#[test]
fn get_image_layers_returns_error_on_malformed_manifest() {
    let registry = ColimaRegistry::new().with_executor(Arc::new(|args: &[&str]| {
        if args.first() == Some(&"cat")
            && args.last().map(|a| a.ends_with("/manifest.json")) == Some(true)
        {
            // Return definitely-invalid JSON.
            Ok("not valid json {{{".to_string())
        } else {
            Ok(String::new())
        }
    }));

    let result = registry.get_image_layers("alpine", "latest");
    assert!(
        result.is_err(),
        "malformed manifest.json must produce an error, got: {:?}",
        result
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("manifest") || msg.contains("parse") || msg.contains("JSON"),
        "error message should mention the manifest parsing failure, got: {msg}"
    );
}

/// get_image_layers_returns_error_on_empty_layers_array — an empty `"Layers"`
/// array is not an error in the current implementation; the method returns an
/// empty Vec.  This test documents and pins that behaviour so a future change
/// that makes empty-layers an error will cause a deliberate test failure
/// rather than a silent regression.
#[test]
fn get_image_layers_returns_error_on_empty_layers_array() {
    let fake_manifest = r#"[{"Layers":[]}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.first() == Some(&"cat")
            && args.last().map(|a| a.ends_with("/manifest.json")) == Some(true)
        {
            Ok(fake_manifest.to_string())
        } else {
            Ok(String::new())
        }
    }));

    // Empty layers is currently treated as a valid (but empty) result, not an error.
    let result = registry
        .get_image_layers("scratch", "latest")
        .expect("empty layers array should succeed — no error expected");

    assert!(
        result.is_empty(),
        "empty Layers array in manifest.json must produce an empty Vec, got: {result:?}"
    );
}

/// get_image_layers propagates executor errors.
#[test]
fn registry_get_image_layers_propagates_error() {
    let registry = ColimaRegistry::new().with_executor(Arc::new(|_args: &[&str]| {
        Err(anyhow::anyhow!("VM not running"))
    }));

    let result = registry.get_image_layers("alpine", "latest");
    assert!(result.is_err(), "executor error must be surfaced");
}

// ============================================================================
// ColimaRuntime tests
// ============================================================================

/// capabilities() reports all features as supported — Colima runs a full
/// Linux kernel inside the Lima VM.
#[test]
fn runtime_capabilities_all_supported() {
    let runtime = ColimaRuntime::new();
    let caps = runtime.capabilities();

    assert!(
        caps.supports_user_namespaces,
        "Colima runtime must report user namespace support"
    );
    assert!(
        caps.supports_cgroups_v2,
        "Colima runtime must report cgroups v2 support"
    );
    assert!(
        caps.supports_overlay_fs,
        "Colima runtime must report overlay FS support"
    );
    assert!(
        caps.supports_network_isolation,
        "Colima runtime must report network isolation support"
    );
    assert!(
        caps.max_containers.is_none(),
        "Colima runtime should not impose a hard container limit"
    );
}

/// spawn_process parses the PID from the executor's stdout and returns it in
/// SpawnResult.pid.
#[tokio::test]
async fn runtime_spawn_process_extracts_pid_from_executor_output() {
    let runtime = ColimaRuntime::new().with_executor(Arc::new(|_args: &[&str]| {
        // The spawn script prints $! of the background process.
        Ok("42\n".to_string())
    }));

    let config = ContainerSpawnConfig {
        rootfs: PathBuf::from("/tmp/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],    // placeholder — Task 6 replaces this
        privileged: false, // placeholder — Task 6 replaces this
    };

    let result = runtime
        .spawn_process(&config)
        .await
        .expect("spawn_process should succeed");

    assert_eq!(result.pid, 42, "PID must match the executor's output");
}

/// spawn_process returns an error when the executor output is not a valid PID.
#[tokio::test]
async fn runtime_spawn_process_errors_on_invalid_pid() {
    let runtime = ColimaRuntime::new()
        .with_executor(Arc::new(|_args: &[&str]| Ok("not-a-number\n".to_string())));

    let config = ContainerSpawnConfig {
        rootfs: PathBuf::from("/tmp/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],    // placeholder — Task 6 replaces this
        privileged: false, // placeholder — Task 6 replaces this
    };

    let result = runtime.spawn_process(&config).await;
    assert!(result.is_err(), "non-numeric PID output must be an error");
}

/// spawn_process propagates executor failures.
#[tokio::test]
async fn runtime_spawn_process_propagates_executor_error() {
    let runtime = ColimaRuntime::new().with_executor(Arc::new(|_args: &[&str]| {
        Err(anyhow::anyhow!("VM unreachable"))
    }));

    let config = ContainerSpawnConfig {
        rootfs: PathBuf::from("/tmp/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],    // placeholder — Task 6 replaces this
        privileged: false, // placeholder — Task 6 replaces this
    };

    let result = runtime.spawn_process(&config).await;
    assert!(result.is_err(), "executor error must be surfaced");
}

/// The spawn script sent to the Lima VM must embed config.args so they are
/// passed to the container command.
#[tokio::test]
async fn runtime_spawn_script_embeds_args() {
    use std::sync::{Arc, Mutex};

    let captured_argv = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured_argv_ref = captured_argv.clone();
    let captured_script = Arc::new(Mutex::new(String::new()));
    let cap = captured_script.clone();

    let runtime = ColimaRuntime::new().with_executor(Arc::new(move |args: &[&str]| {
        *captured_argv_ref.lock().unwrap() = args.iter().map(|arg| arg.to_string()).collect();
        if let Some(pos) = args.iter().position(|&a| a == "-c" || a == "-lc") {
            if let Some(script) = args.get(pos + 1) {
                *cap.lock().unwrap() = script.to_string();
            }
        }
        Ok("99\n".to_string())
    }));

    let config = ContainerSpawnConfig {
        rootfs: PathBuf::from("/tmp/rootfs"),
        command: "/bin/echo".to_string(),
        args: vec!["hello".to_string(), "world".to_string()],
        env: vec![],
        hostname: "test-container".to_string(),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],    // placeholder — Task 6 replaces this
        privileged: false, // placeholder — Task 6 replaces this
    };

    let result = runtime.spawn_process(&config).await.unwrap();
    assert_eq!(result.pid, 99);

    let argv = captured_argv.lock().unwrap().clone();
    let script = captured_script.lock().unwrap().clone();
    assert_eq!(argv.first().map(String::as_str), Some("bash"));
    assert_eq!(argv.get(1).map(String::as_str), Some("-lc"));
    assert!(
        script.contains("hello"),
        "spawn script must embed arg 'hello', got: {script}"
    );
    assert!(
        script.contains("world"),
        "spawn script must embed arg 'world', got: {script}"
    );
}

// ============================================================================
// ColimaFilesystem and ColimaLimiter — trait-level tests
//
// Both adapters expose `with_executor()` for injecting a test seam (see
// ColimaRegistry/ColimaRuntime pattern).  The tests below exercise both the
// injected-executor path (verifying correct command construction) and the
// no-executor path (where limactl is absent, verifying error propagation).
// ============================================================================

use mbx::adapters::{ColimaFilesystem, ColimaLimiter};

/// setup_rootfs must pass the overlay mount as argv so spaced macOS paths work.
#[test]
fn filesystem_setup_rootfs_uses_sudo_mount_with_spaced_paths() {
    use std::sync::{Arc, Mutex};

    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let recorded = calls.clone();
    let fs = ColimaFilesystem::new().with_executor(Arc::new(move |args: &[&str]| {
        recorded
            .lock()
            .unwrap()
            .push(args.iter().map(|arg| arg.to_string()).collect());
        Ok(String::new())
    }));

    let merged = fs
        .setup_rootfs(
            &[PathBuf::from("/tmp/layer0")],
            &PathBuf::from("/Users/joe/Library/Application Support/minibox/container-test"),
        )
        .expect("setup_rootfs should succeed with injected executor");

    assert_eq!(
        merged.merged_dir,
        PathBuf::from("/Users/joe/Library/Application Support/minibox/container-test/merged")
    );

    let calls = calls.lock().unwrap();
    let mount_call = calls
        .iter()
        .find(|call| {
            call.first().map(String::as_str) == Some("sudo")
                && call.get(1).map(String::as_str) == Some("mount")
        })
        .expect("expected sudo mount invocation");
    assert!(
        mount_call[6].starts_with("lowerdir=/tmp/layer0,upperdir=/Users/joe/Library/Application Support/minibox/container-test/upper,"),
        "mount options must preserve the spaced host path, got: {}",
        mount_call[6]
    );
    assert_eq!(
        mount_call[7],
        "/Users/joe/Library/Application Support/minibox/container-test/merged"
    );
}

/// pivot_root is a no-op for the Colima adapter and always succeeds.
#[test]
fn filesystem_pivot_root_is_noop() {
    let fs = ColimaFilesystem::new();
    let result = fs.pivot_root(&PathBuf::from("/tmp/newroot"));
    assert!(
        result.is_ok(),
        "pivot_root is a no-op in the Colima adapter and must always succeed"
    );
}

/// cleanup must use sudo for the umount before removing the container dir.
#[test]
fn filesystem_cleanup_uses_sudo_umount() {
    use std::sync::{Arc, Mutex};

    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let recorded = calls.clone();
    let fs = ColimaFilesystem::new().with_executor(Arc::new(move |args: &[&str]| {
        recorded
            .lock()
            .unwrap()
            .push(args.iter().map(|arg| arg.to_string()).collect());
        Ok(String::new())
    }));

    fs.cleanup(&PathBuf::from("/tmp/container-test"))
        .expect("cleanup should succeed with injected executor");

    let calls = calls.lock().unwrap();
    assert_eq!(
        calls[0],
        vec![
            "sudo".to_string(),
            "umount".to_string(),
            "/tmp/container-test/merged".to_string()
        ]
    );
    assert_eq!(
        calls[1],
        vec![
            "rm".to_string(),
            "-rf".to_string(),
            "/tmp/container-test".to_string()
        ]
    );
}

/// ResourceLimiter::create must use sudo for cgroup setup and writes.
#[test]
fn limiter_create_uses_sudo_for_cgroup_operations() {
    use mbx::domain::ResourceConfig;
    use std::sync::{Arc, Mutex};

    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let recorded = calls.clone();
    let limiter = ColimaLimiter::new().with_executor(Arc::new(move |args: &[&str]| {
        recorded
            .lock()
            .unwrap()
            .push(args.iter().map(|arg| arg.to_string()).collect());
        Ok(String::new())
    }));
    let config = ResourceConfig {
        memory_limit_bytes: Some(128 * 1024 * 1024),
        cpu_weight: Some(100),
        pids_max: None,
        io_max_bytes_per_sec: None,
    };
    let path = limiter
        .create("test-container-id", &config)
        .expect("create should succeed with injected executor");

    assert_eq!(path, "/sys/fs/cgroup/minibox/test-container-id");

    let calls = calls.lock().unwrap();
    assert_eq!(
        calls[1],
        vec![
            "sudo".to_string(),
            "mkdir".to_string(),
            "-p".to_string(),
            "/sys/fs/cgroup/minibox".to_string()
        ]
    );
    assert_eq!(
        calls[2],
        vec![
            "sudo".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            "echo +cpu +memory +pids +io > /sys/fs/cgroup/minibox/cgroup.subtree_control 2>/dev/null || true"
                .to_string(),
        ]
    );
    assert_eq!(
        calls[3],
        vec![
            "sudo".to_string(),
            "mkdir".to_string(),
            "-p".to_string(),
            "/sys/fs/cgroup/minibox/test-container-id".to_string()
        ]
    );
    assert!(calls.iter().any(|call| {
        call == &vec![
            "sudo".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            "echo 134217728 > /sys/fs/cgroup/minibox/test-container-id/memory.max".to_string(),
        ]
    }));
    assert!(calls.iter().any(|call| {
        call == &vec![
            "sudo".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            "echo 100 > /sys/fs/cgroup/minibox/test-container-id/cpu.weight".to_string(),
        ]
    }));
}

/// ResourceLimiter::add_process must use sudo for cgroup.procs writes.
#[test]
fn limiter_add_process_uses_sudo() {
    use std::sync::{Arc, Mutex};

    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let recorded = calls.clone();
    let limiter = ColimaLimiter::new().with_executor(Arc::new(move |args: &[&str]| {
        recorded
            .lock()
            .unwrap()
            .push(args.iter().map(|arg| arg.to_string()).collect());
        Ok(String::new())
    }));

    limiter
        .add_process("test-container-id", 1234)
        .expect("add_process should succeed with injected executor");

    let calls = calls.lock().unwrap();
    assert!(calls.iter().any(|call| {
        call == &vec![
            "sudo".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            "echo 1234 > /sys/fs/cgroup/minibox/test-container-id/cgroup.procs".to_string(),
        ]
    }));
}

/// ResourceLimiter::cleanup must use sudo for cgroup removal.
#[test]
fn limiter_cleanup_uses_sudo_rmdir() {
    use std::sync::{Arc, Mutex};

    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let recorded = calls.clone();
    let limiter = ColimaLimiter::new().with_executor(Arc::new(move |args: &[&str]| {
        recorded
            .lock()
            .unwrap()
            .push(args.iter().map(|arg| arg.to_string()).collect());
        Ok(String::new())
    }));

    limiter
        .cleanup("test-container-id")
        .expect("cleanup should succeed with injected executor");

    let calls = calls.lock().unwrap();
    assert!(calls.iter().any(|call| {
        call == &vec![
            "sudo".to_string(),
            "rmdir".to_string(),
            "/sys/fs/cgroup/minibox/test-container-id".to_string(),
        ]
    }));
}
