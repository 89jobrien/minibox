# Security Testing Guide

This document outlines security testing procedures, test cases, and tools for validating minibox's security controls.

## Overview

Security testing ensures that:
- Isolation mechanisms prevent container escapes
- Input validation blocks malicious payloads
- Authentication prevents unauthorized access
- Resource limits prevent denial of service
- Path validation prevents directory traversal

## Test Categories

### 1. Authentication Tests

#### Unix Socket Authentication

**Test: Root-only access enforcement**

```bash
# As non-root user (should fail)
sudo -u nobody nc -U /run/minibox/miniboxd.sock
# Expected: Connection rejected

# As root (should succeed)
sudo nc -U /run/minibox/miniboxd.sock
# Expected: Connection accepted
```

**Test: Socket permission enforcement**

```bash
# Check socket permissions
ls -la /run/minibox/miniboxd.sock
# Expected: srwx------ (0600, root:root)

# Attempt chmod as non-root (should fail)
sudo -u nobody chmod 666 /run/minibox/miniboxd.sock
# Expected: Permission denied
```

**Automated Test:**

```rust
#[test]
#[ignore]
fn test_authentication_root_only() {
    require_root();
    // Start daemon
    let daemon = start_daemon();

    // Try connecting as non-root (fork process with different UID)
    let result = attempt_connect_as_uid(1000);
    assert!(result.is_err(), "Non-root connection should be rejected");

    // Connect as root
    let result = attempt_connect_as_uid(0);
    assert!(result.is_ok(), "Root connection should succeed");
}
```

### 2. Input Validation Tests

#### Path Traversal Prevention

**Test Cases:**

```rust
#[test]
fn test_path_traversal_rejection() {
    let test_cases = vec![
        "../etc/passwd",                    // Parent directory
        "/etc/../../../etc/passwd",         // Absolute with traversal
        "layer/../../root/.ssh",            // Multiple levels
        "layer/../../../",                  // Escape to root
        "layer/./../../etc",                // Dot-dot with dot
    ];

    for malicious_path in test_cases {
        let result = validate_layer_path(
            Path::new("/var/lib/minibox/images"),
            Path::new(malicious_path)
        );
        assert!(
            result.is_err(),
            "Path traversal should be rejected: {}",
            malicious_path
        );
    }
}
```

**Manual Test:**

```bash
# Create malicious image with path traversal
mkdir -p /tmp/evil-layer
cd /tmp/evil-layer
ln -s /etc/passwd evil-symlink
tar -czf /tmp/evil.tar.gz evil-symlink

# Try to extract (should fail)
# Implementation should reject symlink to absolute path
```

#### Tar Extraction Safety (Zip Slip)

**Test Cases:**

```rust
#[test]
fn test_zip_slip_rejection() {
    let malicious_entries = vec![
        "../../etc/passwd",                 // Path traversal
        "/etc/passwd",                      // Absolute path
        "layer/../../../../../root/.ssh",  // Deep traversal
    ];

    for entry in malicious_entries {
        let result = validate_tar_entry(entry);
        assert!(
            result.is_err(),
            "Zip slip entry should be rejected: {}",
            entry
        );
    }
}

#[test]
fn test_device_node_rejection() {
    // Test that device nodes are rejected
    let device_types = vec![
        tar::EntryType::Block,
        tar::EntryType::Char,
        tar::EntryType::Fifo,
    ];

    for entry_type in device_types {
        let result = validate_tar_entry_type(entry_type);
        assert!(
            result.is_err(),
            "Device nodes should be rejected: {:?}",
            entry_type
        );
    }
}

#[test]
fn test_setuid_stripping() {
    // Test that setuid/setgid bits are stripped
    let mode = 0o4755; // setuid + rwxr-xr-x
    let safe_mode = strip_dangerous_bits(mode);
    assert_eq!(safe_mode, 0o755, "Setuid bit should be stripped");

    let mode = 0o2755; // setgid + rwxr-xr-x
    let safe_mode = strip_dangerous_bits(mode);
    assert_eq!(safe_mode, 0o755, "Setgid bit should be stripped");
}
```

**Manual Test with Crafted Archive:**

```bash
# Create malicious tar with path traversal
mkdir -p /tmp/zipslip-test
cd /tmp/zipslip-test
mkdir -p evil
touch evil/../../../../tmp/pwned
tar -czf zipslip.tar.gz evil/../../../../tmp/pwned

# Try to extract with minibox (should fail)
# Layer extraction should reject this
```

#### Request Size Validation

**Test Cases:**

```rust
#[test]
async fn test_request_size_limit() {
    // Create request larger than 1 MB
    let large_payload = "A".repeat(2 * 1024 * 1024); // 2 MB
    let request = DaemonRequest::Run {
        image: large_payload,
        tag: None,
        command: vec![],
        memory_limit_bytes: None,
        cpu_weight: None,
    };

    let result = handle_request(request).await;
    assert!(
        result.is_err(),
        "Request exceeding size limit should be rejected"
    );
}
```

### 3. Isolation Tests

#### Namespace Isolation

**Test: PID namespace isolation**

```bash
# Run container
sudo minibox run alpine -- /bin/sh -c "echo \$\$ && sleep 60" &

# In another terminal, check PID
ps aux | grep sleep
# Expected: Host sees different PID than container reports (PID 1 inside)

# Try to kill from host using container's PID 1 (should not work)
sudo kill 1
# Expected: Cannot kill init process from outside namespace
```

**Test: Mount namespace isolation**

```bash
# Run container with custom mount
sudo minibox run alpine -- /bin/sh -c "mount && sleep 60" &

# Check host mounts
mount | grep minibox
# Expected: Container overlay visible on host, but container sees different mount tree

# Unmount from host should not affect container
```

**Test: Network namespace isolation**

```bash
# Run container
sudo minibox run alpine -- /bin/sh -c "ip addr && sleep 60" &

# Container should see only loopback
# Expected: No eth0 or other host interfaces visible inside container
```

**Automated Test:**

```rust
#[test]
#[ignore]
fn test_pid_namespace_isolation() {
    require_root();

    // Spawn container
    let container_id = spawn_test_container("/bin/sh", vec!["-c", "echo $$"]);

    // Container should see PID 1
    let output = read_container_output(&container_id);
    assert_eq!(output.trim(), "1", "Container should see PID 1");

    // Host should see different PID
    let host_pid = get_container_host_pid(&container_id);
    assert_ne!(host_pid, 1, "Host should see real PID, not namespace PID");
}
```

#### Mount Security Flags

**Test Cases:**

```rust
#[test]
#[ignore]
fn test_nosuid_mount_flag() {
    require_root();

    // Create setuid binary in container
    let container_id = create_test_container();
    create_file_in_container(&container_id, "/tmp/setuid-test", 0o4755);

    // Try to execute setuid binary (should not escalate privileges)
    let result = exec_in_container(&container_id, "/tmp/setuid-test");
    assert_eq!(
        result.effective_uid,
        result.real_uid,
        "Setuid should not escalate privileges (MS_NOSUID)"
    );
}

#[test]
#[ignore]
fn test_nodev_mount_flag() {
    require_root();

    let container_id = create_test_container();

    // Try to create device node (should fail)
    let result = exec_in_container(
        &container_id,
        "mknod /tmp/null c 1 3"
    );
    assert!(
        result.is_err(),
        "Device node creation should fail (MS_NODEV)"
    );
}

#[test]
#[ignore]
fn test_sys_readonly() {
    require_root();

    let container_id = create_test_container();

    // Try to write to /sys (should fail)
    let result = exec_in_container(
        &container_id,
        "echo 1 > /sys/kernel/test"
    );
    assert!(
        result.is_err(),
        "/sys should be read-only"
    );
}
```

### 4. Resource Limit Tests

#### Memory Limits

**Test Cases:**

```rust
#[test]
#[ignore]
fn test_memory_limit_enforcement() {
    require_root();

    // Create container with 100 MB memory limit
    let container_id = run_container_with_limits(
        "alpine",
        "/bin/sh",
        vec!["-c", "dd if=/dev/zero of=/dev/null bs=1M count=200"],
        Some(100 * 1024 * 1024), // 100 MB
        None,
    );

    // Process should be killed by OOM killer
    let status = wait_for_container(&container_id);
    assert!(
        status.is_err() || status.unwrap().signal() == Some(9),
        "Container should be OOM killed when exceeding memory limit"
    );
}
```

#### PID Limits (Fork Bomb Protection)

**Test Cases:**

```rust
#[test]
#[ignore]
fn test_fork_bomb_protection() {
    require_root();

    // Create container with default PID limit (1024)
    let container_id = run_test_container(
        "alpine",
        "/bin/sh",
        vec!["-c", ":(){ :|:& };:"], // Fork bomb
    );

    // Wait for PID limit to be hit
    std::thread::sleep(Duration::from_secs(5));

    // Container should not exhaust host PIDs
    let host_pids = count_host_processes();
    assert!(
        host_pids < 10000,
        "Fork bomb should be contained by PID limit"
    );

    // cgroup pids.current should be at limit
    let cgroup_pids = read_cgroup_value(&container_id, "pids.current");
    assert!(
        cgroup_pids <= 1024,
        "Container PID count should not exceed limit"
    );
}
```

#### CPU Limits

**Test Cases:**

```rust
#[test]
#[ignore]
fn test_cpu_weight_enforcement() {
    require_root();

    // Create two containers with different CPU weights
    let high_prio = run_container_with_limits(
        "alpine",
        "stress",
        vec!["--cpu", "1"],
        None,
        Some(1000), // High CPU weight
    );

    let low_prio = run_container_with_limits(
        "alpine",
        "stress",
        vec!["--cpu", "1"],
        None,
        Some(100), // Low CPU weight
    );

    // Measure CPU usage after 10 seconds
    std::thread::sleep(Duration::from_secs(10));

    let high_cpu = get_container_cpu_usage(&high_prio);
    let low_cpu = get_container_cpu_usage(&low_prio);

    // High priority should get more CPU time
    assert!(
        high_cpu > low_cpu * 5,
        "High CPU weight container should get more CPU time"
    );
}
```

### 5. Denial of Service Tests

#### Concurrent Spawn Limits

**Test Cases:**

```rust
#[test]
#[ignore]
fn test_concurrent_spawn_limit() {
    require_root();

    let semaphore_limit = 100;
    let mut handles = vec![];

    // Try to spawn 200 containers concurrently
    for i in 0..200 {
        let handle = tokio::spawn(async move {
            run_container("alpine", "/bin/sleep", vec!["60"]).await
        });
        handles.push(handle);
    }

    // Wait for all spawns
    let results = futures::future::join_all(handles).await;

    // Count successful spawns
    let successful = results.iter().filter(|r| r.is_ok()).count();

    // Should be capped at semaphore limit
    assert!(
        successful <= semaphore_limit,
        "Concurrent spawns should be limited to {}",
        semaphore_limit
    );
}
```

#### Image Size Limits

**Test Cases:**

```rust
#[test]
#[ignore]
async fn test_large_layer_rejection() {
    require_root();

    // Mock registry returning 11 GB layer
    let mock_registry = MockRegistry::new()
        .with_layer_size(11 * 1024 * 1024 * 1024); // 11 GB

    let result = pull_image_with_registry("huge-image", "latest", mock_registry).await;

    assert!(
        result.is_err(),
        "Layer exceeding 10 GB should be rejected"
    );
}

#[test]
#[ignore]
async fn test_large_manifest_rejection() {
    require_root();

    // Mock registry returning 11 MB manifest
    let mock_registry = MockRegistry::new()
        .with_manifest_size(11 * 1024 * 1024); // 11 MB

    let result = pull_image_with_registry("bad-image", "latest", mock_registry).await;

    assert!(
        result.is_err(),
        "Manifest exceeding 10 MB should be rejected"
    );
}
```

### 6. Fuzzing Tests

#### Input Fuzzing with cargo-fuzz

**Setup:**

```bash
# Install cargo-fuzz
cargo install cargo-fuzz

# Initialize fuzz targets
cargo fuzz init
```

**Fuzz Target: JSON Protocol**

```rust
// fuzz/fuzz_targets/protocol.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use minibox_lib::protocol::DaemonRequest;

fuzz_target!(|data: &[u8]| {
    // Try to parse arbitrary bytes as JSON request
    if let Ok(json_str) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<DaemonRequest>(json_str);
    }
});
```

**Fuzz Target: Path Validation**

```rust
// fuzz/fuzz_targets/path_validation.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use minibox_lib::container::filesystem::validate_layer_path;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    if let Ok(path_str) = std::str::from_utf8(data) {
        let base = Path::new("/var/lib/minibox/images");
        let _ = validate_layer_path(base, Path::new(path_str));
    }
});
```

**Run Fuzzing:**

```bash
# Fuzz protocol parsing (run for hours/days)
cargo fuzz run protocol -- -max_len=10000 -runs=1000000

# Fuzz path validation
cargo fuzz run path_validation -- -max_len=1000 -runs=1000000
```

### 7. Penetration Testing Scenarios

#### Scenario 1: Container Escape Attempt

**Objective:** Try to escape container and access host filesystem

**Steps:**

1. Run container with privileged-looking image
2. Try to break out of namespaces
3. Try to mount host filesystem
4. Try to access host /proc
5. Try to send signals to host processes

**Expected:** All attempts should fail

#### Scenario 2: Privilege Escalation

**Objective:** Escalate from container user to host root

**Steps:**

1. Create setuid binary in container
2. Try to execute and gain elevated privileges
3. Try to exploit kernel vulnerabilities
4. Try to escape via cgroup manipulation

**Expected:** MS_NOSUID prevents setuid, namespace isolation prevents host access

#### Scenario 3: Resource Exhaustion

**Objective:** Exhaust host resources from container

**Steps:**

1. Fork bomb (PID exhaustion)
2. Memory allocation bomb
3. Disk I/O saturation
4. CPU spinning

**Expected:** cgroups limits contain all resource usage

## Continuous Security Testing

### Pre-commit Checks

```bash
# Add to .git/hooks/pre-commit
#!/bin/bash
set -e

# Run security lints
cargo clippy --workspace -- -D warnings

# Run unit tests
cargo test --workspace

# Check for known vulnerabilities
cargo deny check advisories
```

### CI Pipeline Integration

See `.github/workflows/security.yml` for automated security scanning on every commit.

### Daily Security Scan

```bash
# Run comprehensive security scan
cargo deny check
cargo audit
cargo clippy --workspace -- -D warnings -W clippy::suspicious

# Run security-specific tests
cargo test --workspace security
```

## Security Test Metrics

### Coverage Goals

- **Path validation:** 100% coverage
- **Tar extraction:** 100% coverage
- **Authentication:** 100% coverage
- **Resource limits:** 90%+ coverage
- **Namespace isolation:** 80%+ coverage

### Test Execution Frequency

- **Unit tests:** Every commit
- **Integration tests:** Every PR
- **Fuzzing:** Continuous (overnight runs)
- **Penetration testing:** Quarterly
- **Full security audit:** Bi-annually

## Reporting Security Test Failures

### Critical Failures

If any of these tests fail, **DO NOT MERGE:**

- Authentication bypass
- Path traversal allowed
- Tar extraction allows Zip Slip
- Container escape possible
- Resource limits not enforced

### Process

1. Create security issue (private)
2. Assign to security team
3. Develop fix
4. Test fix thoroughly
5. Deploy fix
6. Disclose responsibly after users have upgraded

## Tools and Dependencies

### Required Tools

- **cargo-deny:** Dependency vulnerability scanning
- **cargo-audit:** Advisory database checking
- **cargo-fuzz:** Fuzzing framework
- **semgrep:** Static analysis
- **clippy:** Rust linting with security rules

### Installation

```bash
# Install security tools
cargo install cargo-deny cargo-audit cargo-fuzz

# Install semgrep
pip install semgrep
```

## References

- [SECURITY.md](SECURITY.md) - Security policy and threat model
- [SECURITY_FIXES.md](SECURITY_FIXES.md) - Vulnerability fixes
- OWASP Testing Guide: https://owasp.org/www-project-web-security-testing-guide/
- Fuzzing Book: https://www.fuzzingbook.org/
- Rust Security Guidelines: https://anssi-fr.github.io/rust-guide/

## Changelog

### 2026-03-16
- Created comprehensive security testing guide
- Defined test cases for all security controls
- Established fuzzing strategy
- Documented penetration testing scenarios
