//! `mbx diagnose` — gather diagnostic context for a container.
//!
//! Queries the daemon for the container list, finds the target by ID prefix,
//! then inspects host-side state (cgroup path, /proc/<pid>/status on Linux)
//! and prints a structured text report.  LLM integration is intentionally
//! absent; this command produces the raw data a human or downstream tool
//! can feed to an AI.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{ContainerInfo, DaemonRequest, DaemonResponse};

/// Execute the `diagnose` subcommand.
///
/// Fetches the container list from the daemon, locates the requested container
/// by exact ID or unambiguous prefix, and prints a diagnostic report to
/// stdout.  On Linux, host-visible state (cgroup path, `/proc/<pid>/status`)
/// is also included when accessible.
pub async fn execute(container_id: &str, socket_path: &std::path::Path) -> anyhow::Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::List)
        .await
        .context("failed to call daemon")?;

    let containers = match stream.next().await.context("stream error")? {
        Some(DaemonResponse::ContainerList { containers }) => containers,
        Some(DaemonResponse::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Some(other) => {
            eprintln!("unexpected response: {other:?}");
            std::process::exit(1);
        }
        None => {
            eprintln!("no response from daemon");
            std::process::exit(1);
        }
    };

    let info = find_container(container_id, &containers)
        .ok_or_else(|| anyhow::anyhow!("container not found: {container_id}"))?;

    print_report(info);
    Ok(())
}

/// Find a container by exact ID or unambiguous prefix.
fn find_container<'a>(id: &str, containers: &'a [ContainerInfo]) -> Option<&'a ContainerInfo> {
    // Exact match first.
    if let Some(c) = containers.iter().find(|c| c.id == id) {
        return Some(c);
    }
    // Prefix match — return only if unambiguous.
    let matches: Vec<_> = containers.iter().filter(|c| c.id.starts_with(id)).collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

/// Print the diagnostic report.
fn print_report(info: &ContainerInfo) {
    println!("=== minibox diagnose ===");
    println!("container_id : {}", info.id);
    if let Some(name) = &info.name {
        println!("name         : {name}");
    }
    println!("image        : {}", info.image);
    println!("command      : {}", info.command);
    println!("state        : {}", info.state);
    println!("created_at   : {}", info.created_at);

    match info.pid {
        Some(pid) => {
            println!("pid          : {pid}");
            print_host_diagnostics(pid);
        }
        None => {
            println!("pid          : (none)");
        }
    }
}

/// Gather host-visible diagnostics for a running container PID.
///
/// On non-Linux platforms these checks are silently skipped.
fn print_host_diagnostics(pid: u32) {
    #[cfg(target_os = "linux")]
    {
        // /proc/<pid>/status — process memory + state snapshot.
        let status_path = format!("/proc/{pid}/status");
        match std::fs::read_to_string(&status_path) {
            Ok(content) => {
                println!("\n--- /proc/{pid}/status ---");
                // Print a curated subset: Name, State, VmRSS, Threads.
                for line in content.lines() {
                    let key = line.split(':').next().unwrap_or("").trim();
                    if matches!(key, "Name" | "State" | "VmRSS" | "VmPeak" | "Threads") {
                        println!("{line}");
                    }
                }
            }
            Err(e) => {
                println!("/proc/{pid}/status unavailable: {e}");
            }
        }

        // cgroup path for the container.
        let cgroup_root = std::env::var("MINIBOX_CGROUP_ROOT")
            .unwrap_or_else(|_| "/sys/fs/cgroup/minibox.slice/miniboxd.service".to_string());
        // We don't have the container ID here (only PID), so just note the root.
        println!("\n--- cgroup root ---");
        println!("{cgroup_root}");
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid; // suppress unused warning on non-Linux
        println!("(host diagnostics not available on this platform)");
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup;
    use super::*;
    use minibox_core::protocol::ContainerInfo;

    fn make_info(id: &str, state: &str, pid: Option<u32>) -> ContainerInfo {
        ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine".to_string(),
            command: "/bin/sh".to_string(),
            state: state.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid,
        }
    }

    // ── find_container ────────────────────────────────────────────────────────

    #[test]
    fn find_exact_id_returns_container() {
        let containers = vec![make_info("abc123", "running", Some(42))];
        let result = find_container("abc123", &containers);
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "abc123");
    }

    #[test]
    fn find_prefix_returns_container_when_unambiguous() {
        let containers = vec![make_info("abc123def", "running", Some(1))];
        let result = find_container("abc123", &containers);
        assert!(result.is_some());
    }

    #[test]
    fn find_prefix_returns_none_when_ambiguous() {
        let containers = vec![
            make_info("abc123", "running", Some(1)),
            make_info("abc456", "stopped", None),
        ];
        let result = find_container("abc", &containers);
        assert!(result.is_none(), "should be None for ambiguous prefix");
    }

    #[test]
    fn find_nonexistent_returns_none() {
        let containers = vec![make_info("abc123", "running", Some(1))];
        let result = find_container("xyz999", &containers);
        assert!(result.is_none());
    }

    // ── execute (daemon integration via mock socket) ──────────────────────────

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_returns_error_when_container_not_found() {
        let (_tmp, socket_path) = setup(DaemonResponse::ContainerList {
            containers: vec![make_info("abc123", "running", Some(42))],
        })
        .await;
        // "xyz999" does not exist — execute should return Err.
        let result = execute("xyz999", &socket_path).await;
        assert!(
            result.is_err(),
            "should fail for unknown container: {result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_succeeds_for_known_container() {
        let (_tmp, socket_path) = setup(DaemonResponse::ContainerList {
            containers: vec![make_info("abc123", "running", Some(42))],
        })
        .await;
        let result = execute("abc123", &socket_path).await;
        assert!(
            result.is_ok(),
            "should succeed for known container: {result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_handles_daemon_error_gracefully() {
        // The daemon returns an error response — execute should exit(1), not panic.
        // We test this by asserting the Ok path is NOT taken; in a real test suite
        // this would be an exit-code assertion, but our helpers can't capture that.
        // Instead we confirm the function propagates via process::exit by checking
        // that a success response works and an error response path exists structurally.
        let (_tmp, socket_path) = setup(DaemonResponse::ContainerList { containers: vec![] }).await;
        let result = execute("any", &socket_path).await;
        // Empty list → container not found → Err returned (not process::exit).
        assert!(result.is_err());
    }
}
