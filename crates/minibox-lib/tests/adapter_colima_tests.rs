//! Integration tests for the Colima adapter suite.
//!
//! Tests use injected `LimaExecutor` closures (via `with_executor()`) to avoid
//! requiring a running Colima VM.  Each test exercises the adapter through the
//! domain trait interface, verifying that the adapter correctly translates trait
//! calls into the expected Lima shell commands and handles responses properly.

use minibox_lib::adapters::{ColimaRegistry, ColimaRuntime};
use minibox_lib::domain::{
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

    let result = registry.pull_image("alpine", "latest").await;
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
        .pull_image("alpine", "latest")
        .await
        .expect("pull_image should succeed with valid executor output");

    assert_eq!(metadata.name, "alpine");
    assert_eq!(metadata.tag, "latest");
    assert_eq!(metadata.layers.len(), 2, "should have two layers");
}

/// get_image_layers must return paths that are accessible from the macOS host,
/// i.e. they must live under /tmp/ or /Users/ — the Lima-shared mounts.
#[test]
fn registry_get_image_layers_returns_host_accessible_paths() {
    let fake_inspect =
        r#"[{"Size":1024,"RootFS":{"Layers":["sha256:abc123def456","sha256:def456abc789"]}}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.contains(&"inspect") {
            Ok(fake_inspect.to_string())
        } else {
            // accept mkdir, nerdctl save, tar xf, etc.
            Ok(String::new())
        }
    }));

    let layers = registry
        .get_image_layers("alpine", "latest")
        .expect("get_image_layers should succeed with a valid executor");

    assert_eq!(
        layers.len(),
        2,
        "should return one PathBuf per layer digest"
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
    let fake_inspect = r#"[{"Size":0,"RootFS":{"Layers":[]}}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.contains(&"inspect") {
            Ok(fake_inspect.to_string())
        } else {
            Ok(String::new())
        }
    }));

    let layers = registry
        .get_image_layers("scratch", "latest")
        .expect("get_image_layers should succeed");

    assert!(layers.is_empty(), "no layers in inspect → empty result");
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
    };

    let result = runtime.spawn_process(&config).await;
    assert!(result.is_err(), "executor error must be surfaced");
}

/// The spawn script sent to the Lima VM must embed config.args so they are
/// passed to the container command.
#[tokio::test]
async fn runtime_spawn_script_embeds_args() {
    use std::sync::{Arc, Mutex};

    let captured_script = Arc::new(Mutex::new(String::new()));
    let cap = captured_script.clone();

    let runtime = ColimaRuntime::new().with_executor(Arc::new(move |args: &[&str]| {
        if let Some(pos) = args.iter().position(|&a| a == "-c") {
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
    };

    let result = runtime.spawn_process(&config).await.unwrap();
    assert_eq!(result.pid, 99);

    let script = captured_script.lock().unwrap().clone();
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
// Neither ColimaFilesystem nor ColimaLimiter expose `with_executor()` publicly.
// We test them by constructing them without a VM and observing the expected
// failure mode: the `limactl` binary is absent in the test environment, so
// every call that requires the VM must return an error.  This verifies that
// the adapters propagate infrastructure errors rather than silently succeeding.
// ============================================================================

use minibox_lib::adapters::{ColimaFilesystem, ColimaLimiter};

/// setup_rootfs must fail when no Lima VM is available (limactl not found).
#[test]
fn filesystem_setup_rootfs_fails_without_vm() {
    let fs = ColimaFilesystem::new();
    let result = fs.setup_rootfs(
        &[PathBuf::from("/tmp/layer0")],
        &PathBuf::from("/tmp/container-test"),
    );
    assert!(
        result.is_err(),
        "setup_rootfs must fail when limactl is not available"
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

/// cleanup must fail when no Lima VM is available (limactl not found).
#[test]
fn filesystem_cleanup_fails_without_vm() {
    let fs = ColimaFilesystem::new();
    let result = fs.cleanup(&PathBuf::from("/tmp/container-test"));
    assert!(
        result.is_err(),
        "cleanup must fail when limactl is not available"
    );
}

/// ResourceLimiter::create must fail when no Lima VM is available.
#[test]
fn limiter_create_fails_without_vm() {
    use minibox_lib::domain::ResourceConfig;

    let limiter = ColimaLimiter::new();
    let config = ResourceConfig {
        memory_limit_bytes: Some(128 * 1024 * 1024),
        cpu_weight: Some(100),
        pids_max: None,
        io_max_bytes_per_sec: None,
    };
    let result = limiter.create("test-container-id", &config);
    assert!(
        result.is_err(),
        "create must fail when limactl is not available"
    );
}

/// ResourceLimiter::add_process must fail when no Lima VM is available.
#[test]
fn limiter_add_process_fails_without_vm() {
    let limiter = ColimaLimiter::new();
    let result = limiter.add_process("test-container-id", 1234);
    assert!(
        result.is_err(),
        "add_process must fail when limactl is not available"
    );
}

/// ResourceLimiter::cleanup must fail when no Lima VM is available.
#[test]
fn limiter_cleanup_fails_without_vm() {
    let limiter = ColimaLimiter::new();
    let result = limiter.cleanup("test-container-id");
    assert!(
        result.is_err(),
        "cleanup must fail when limactl is not available"
    );
}
