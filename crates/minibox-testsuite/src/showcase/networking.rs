//! `bridge_networking` scenario — bridge network mode + port forwarding.
//!
//! Fills the coverage gap identified in research: no existing test in the
//! repo calls `BridgeNetwork::apply_port_mappings`
//! (`crates/minibox/src/adapters/network/bridge.rs`) or verifies that a
//! mapped port is actually DNAT-forwarded end to end. Existing bridge tests
//! either exercise the `IpAllocator` in isolation, or re-implement the
//! expected DNAT destination string inline without invoking the real
//! `apply_port_mappings` code path (`bridge_network_dnat_destination_format`),
//! or are a single `#[ignore]`d smoke test that only checks the `minibox0`
//! bridge interface exists (`test_bridge_setup_creates_interface`).
//!
//! This scenario:
//! 1. Runs `mbx run --network bridge <image> -- <cmd>` (plain bridge mode,
//!    no port mapping) and confirms the container reaches `Running`.
//! 2. Runs a second, port-mapped variant — `--network bridge -p
//!    <host>:<container>` — running a trivial TCP listener inside the
//!    container.
//! 3. Connects a real `TcpStream` to the *host* side of the mapping and
//!    asserts the connection succeeds, proving the DNAT rule
//!    (`BridgeNetwork::apply_port_mappings`) actually forwards traffic
//!    rather than just asserting the CLI/daemon accepted the request.
//!
//! Bridge networking is native-Linux-only and requires root to configure
//! `iptables`/bridge interfaces (per
//! `docs/core/FEATURE_MATRIX.mbx.md` and the `#[ignore = "requires root and
//! Linux kernel with bridge support"]` gate on
//! `bridge.rs::test_bridge_setup_creates_interface`). There is a
//! `BackendCapability::Network` variant covering this, declared via
//! `required_capability()` for harness-level auto-skip, but root + kernel
//! bridge support can't be expressed through that enum, so this scenario
//! additionally self-skips ad hoc — matching the existing ignored
//! `bridge_setup` test's gating.

use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use minibox_core::domain::BackendCapability;

use super::reporter::Reporter;
use super::{Scenario, ScenarioCtx};

/// Container-side port the mapped-variant container listens on.
const CONTAINER_PORT: u16 = 8000;

/// How long to wait for a container to reach `Running` before giving up.
const RUNNING_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to retry connecting to the mapped host port before concluding
/// DNAT forwarding did not happen.
const DNAT_REACHABLE_TIMEOUT: Duration = Duration::from_secs(5);

/// Bridge networking + port forwarding showcase scenario.
pub struct BridgeNetworking;

impl Scenario for BridgeNetworking {
    fn name(&self) -> &'static str {
        "bridge_networking"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        Some(BackendCapability::Network)
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("bridge_networking: --network bridge + DNAT port forwarding");

        if !ctx.is_native_linux() {
            r.skip(
                "bridge networking is native-Linux-only (see \
                 docs/core/FEATURE_MATRIX.mbx.md); current adapter does not \
                 support real bridge/iptables setup",
            );
            return Ok(());
        }

        if !ctx.is_root() {
            r.skip(
                "bridge networking requires root to create the minibox0 \
                 bridge interface and iptables DNAT rules (matches the \
                 #[ignore = \"requires root and Linux kernel with bridge \
                 support\"] gate on bridge.rs::test_bridge_setup_creates_interface)",
            );
            return Ok(());
        }

        r.step("spawning daemon");
        let fixture = ctx.spawn_daemon()?;

        r.step("pulling alpine");
        fixture.pull_required("alpine");

        r.step("running container with --network bridge (no port mapping)");
        let (mut plain_child, plain_id) = fixture.spawn_run_background(&[
            "--network",
            "bridge",
            "alpine",
            "--",
            "/bin/sleep",
            "30",
        ]);

        let plain_running = fixture.wait_for_running(&plain_id, RUNNING_TIMEOUT);
        if !plain_running {
            let _ = plain_child.kill();
            r.failure(&format!(
                "container {plain_id} did not reach Running within {RUNNING_TIMEOUT:?} under --network bridge"
            ));
            return Ok(());
        }
        r.success(&format!(
            "container {plain_id} running under --network bridge"
        ));

        r.step("stopping plain bridge container");
        let _ = fixture.run_cli(&["stop", &plain_id]);
        let _ = plain_child.wait();

        let host_port = free_ephemeral_port().context("allocating a free host port for mapping")?;
        let mapping = format!("{host_port}:{CONTAINER_PORT}");

        r.step(&format!(
            "running port-mapped container: --network bridge -p {mapping}"
        ));
        // Trivial TCP listener: netcat accepts one connection and echoes
        // nothing back — we only need the SYN/ACK handshake to succeed to
        // prove the DNAT rule forwards the host port to the container.
        let listen_cmd = format!("nc -lk -p {CONTAINER_PORT}");
        let (mut mapped_child, mapped_id) = fixture.spawn_run_background(&[
            "--network",
            "bridge",
            "-p",
            &mapping,
            "alpine",
            "--",
            "/bin/sh",
            "-c",
            &listen_cmd,
        ]);

        let mapped_running = fixture.wait_for_running(&mapped_id, RUNNING_TIMEOUT);
        if !mapped_running {
            let _ = mapped_child.kill();
            r.failure(&format!(
                "port-mapped container {mapped_id} did not reach Running within {RUNNING_TIMEOUT:?}"
            ));
            return Ok(());
        }
        r.success(&format!("port-mapped container {mapped_id} running"));

        r.step(&format!(
            "verifying DNAT reachability: connecting to 127.0.0.1:{host_port}"
        ));
        let reachable = wait_for_tcp_reachable(host_port, DNAT_REACHABLE_TIMEOUT);

        r.step("cleaning up port-mapped container");
        let _ = fixture.run_cli(&["stop", &mapped_id]);
        let _ = mapped_child.wait();

        if !reachable {
            r.failure(&format!(
                "127.0.0.1:{host_port} was not reachable within {DNAT_REACHABLE_TIMEOUT:?} \
                 — DNAT port mapping ({mapping}) did not forward traffic \
                 (BridgeNetwork::apply_port_mappings may not have applied the rule)"
            ));
            return Ok(());
        }

        r.success(&format!(
            "127.0.0.1:{host_port} reachable — DNAT mapping {mapping} forwards to \
             the container's listener, confirming apply_port_mappings works end to end"
        ));

        Ok(())
    }
}

/// Bind an ephemeral port on loopback, read back the OS-assigned port
/// number, then drop the listener so the port is free for the container's
/// mapped port to bind to on the host side.
fn free_ephemeral_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("binding ephemeral port")?;
    let port = listener
        .local_addr()
        .context("reading ephemeral port's local address")?
        .port();
    drop(listener);
    Ok(port)
}

/// Poll a loopback TCP connection to `port` until it succeeds or `timeout`
/// elapses, returning whether the port was ever reachable.
fn wait_for_tcp_reachable(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_matches_scenario_id() {
        let scenario = BridgeNetworking;
        assert_eq!(scenario.name(), "bridge_networking");
    }

    #[test]
    fn declares_network_capability_gate() {
        let scenario = BridgeNetworking;
        assert_eq!(
            scenario.required_capability(),
            Some(BackendCapability::Network)
        );
    }

    #[test]
    fn free_ephemeral_port_returns_a_bindable_port() {
        let port = free_ephemeral_port().expect("should allocate a free port");
        // The port should be immediately reusable once the listener is dropped.
        let listener = std::net::TcpListener::bind(("127.0.0.1", port));
        assert!(
            listener.is_ok(),
            "expected port {port} to be free after drop"
        );
    }

    #[test]
    fn wait_for_tcp_reachable_times_out_when_nothing_listens() {
        // Port 1 is a reserved low port unlikely to have a listener in test
        // environments and unlikely to be bindable without root, so a
        // connect attempt should fail fast; use a short timeout to keep the
        // test quick.
        let reachable = wait_for_tcp_reachable(1, Duration::from_millis(300));
        assert!(!reachable);
    }
}
