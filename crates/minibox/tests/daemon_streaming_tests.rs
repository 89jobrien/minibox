//! Integration tests for the ephemeral streaming run path.
//!
//! These tests require Linux, root, and network access for image pulls.
//! Run them with:
//!
//!   cargo test -p daemonbox --test streaming_tests -- --include-ignored --nocapture

/// Ephemeral `run` with `capture_output=true` must stream stdout via
/// `ContainerOutput` messages and finish with `ContainerStopped`.
///
/// This test is `#[ignore]` because it requires:
/// - Linux kernel with namespaces + cgroups v2
/// - Root privileges
/// - Network access for the alpine image pull
#[tokio::test]
#[ignore = "requires Linux, root, network for image pull"]
#[cfg(target_os = "linux")]
async fn ephemeral_run_streams_output() {
    // Stub — full implementation covered by Task 7 wire-up.
    //
    // When implemented, this test should:
    // 1. Create a LinuxNamespaceRuntime + real deps (overlay, cgroups).
    // 2. Call handle_run with ephemeral=true, command=["echo", "hello"].
    // 3. Collect ContainerOutput messages from the tx channel.
    // 4. Assert at least one chunk contains base64("hello\n").
    // 5. Assert the final message is ContainerStopped { exit_code: 0 }.
    todo!("implement in Task 7 after CLI wire-up")
}
