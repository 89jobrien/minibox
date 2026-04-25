//! Bridge network adapter — Linux-only.

use ipnet::IpNet;
use std::collections::BTreeSet;
use std::net::IpAddr;

/// Sequential IP allocator within a subnet.
///
/// Skips the network address (`.0`) and gateway address (`.1`).
/// Released IPs are returned to the pool.
pub struct IpAllocator {
    subnet: IpNet,
    available: BTreeSet<u32>, // IPv4 host parts only
    gateway: u32,
}

impl IpAllocator {
    pub fn new(subnet: IpNet) -> Self {
        let base = match subnet.network() {
            IpAddr::V4(a) => u32::from(a),
            IpAddr::V6(_) => panic!("IPv6 not supported in IpAllocator"),
        };
        let hosts = subnet.hosts().filter_map(|ip| {
            if let IpAddr::V4(a) = ip {
                Some(u32::from(a))
            } else {
                None
            }
        });
        let mut available: BTreeSet<u32> = hosts.collect();
        let gateway = base + 1;
        available.remove(&gateway); // reserve gateway
        Self {
            subnet,
            available,
            gateway,
        }
    }

    pub fn allocate(&mut self) -> Option<IpAddr> {
        self.available.pop_first().map(|n| IpAddr::V4(n.into()))
    }

    pub fn release(&mut self, ip: IpAddr) {
        if let IpAddr::V4(a) = ip {
            let n = u32::from(a);
            if self.subnet.contains(&ip) && n != self.gateway {
                self.available.insert(n);
            }
        }
    }

    pub fn gateway(&self) -> IpAddr {
        IpAddr::V4(self.gateway.into())
    }
}

// ---------------------------------------------------------------------------
// BridgeNetwork adapter
// ---------------------------------------------------------------------------

use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::domain::{NetworkConfig, NetworkProvider, NetworkStats};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

const DEFAULT_BRIDGE: &str = "minibox0";
const DEFAULT_SUBNET: &str = "172.20.0.0/16";

/// Bridge-based network adapter for Linux containers.
///
/// Creates a Linux bridge (`minibox0` by default), allocates a private IP from the
/// configured subnet, creates a veth pair per container, attaches the host end to
/// the bridge, and moves the container end into the container network namespace via
/// `ip link set ... netns {pid}`. NAT/MASQUERADE is configured via iptables so the
/// container can reach the internet.
pub struct BridgeNetwork {
    bridge_name: String,
    subnet: IpNet,
    ip_alloc: Arc<Mutex<IpAllocator>>,
}

impl BridgeNetwork {
    /// Create a new bridge network adapter using the default bridge name and subnet.
    pub fn new() -> Result<Self> {
        let subnet: IpNet = DEFAULT_SUBNET.parse().context("parse default subnet")?;
        Ok(Self {
            bridge_name: DEFAULT_BRIDGE.to_string(),
            subnet,
            ip_alloc: Arc::new(Mutex::new(IpAllocator::new(subnet))),
        })
    }

    /// Ensure the bridge interface exists and is up with the gateway IP assigned.
    fn ensure_bridge(&self) -> Result<()> {
        let exists = Command::new("ip")
            .args(["link", "show", &self.bridge_name])
            .output()
            .context("ip link show")?
            .status
            .success();

        if !exists {
            run_cmd(&["ip", "link", "add", &self.bridge_name, "type", "bridge"])
                .context("create bridge")?;
            let gw = self.ip_alloc.lock().unwrap().gateway().to_string();
            let gw_cidr = format!("{}/{}", gw, self.subnet.prefix_len());
            run_cmd(&["ip", "addr", "add", &gw_cidr, "dev", &self.bridge_name])
                .context("assign gateway IP to bridge")?;
            run_cmd(&["ip", "link", "set", &self.bridge_name, "up"]).context("bring bridge up")?;
        }
        Ok(())
    }

    /// Enable IP forwarding and add a MASQUERADE rule if not already present.
    fn ensure_nat(&self) -> Result<()> {
        std::fs::write("/proc/sys/net/ipv4/ip_forward", "1").context("enable ip_forward")?;
        let subnet = self.subnet.to_string();
        let check = Command::new("iptables")
            .args([
                "-t",
                "nat",
                "-C",
                "POSTROUTING",
                "-s",
                &subnet,
                "-j",
                "MASQUERADE",
            ])
            .status()
            .context("iptables check")?;
        if !check.success() {
            run_cmd(&[
                "iptables",
                "-t",
                "nat",
                "-A",
                "POSTROUTING",
                "-s",
                &subnet,
                "-j",
                "MASQUERADE",
            ])
            .context("add MASQUERADE rule")?;
        }
        Ok(())
    }

    /// Apply port mappings via iptables DNAT rules.
    fn apply_port_mappings(
        &self,
        container_ip: &str,
        mappings: &[minibox_core::domain::PortMapping],
    ) -> Result<()> {
        for pm in mappings {
            let proto = pm.protocol.to_string();
            let dport = pm.host_port.to_string();
            let to_dest = format!("{container_ip}:{}", pm.container_port);

            // Check if rule already exists (idempotent)
            let check = Command::new("iptables")
                .args([
                    "-t",
                    "nat",
                    "-C",
                    "PREROUTING",
                    "-p",
                    &proto,
                    "--dport",
                    &dport,
                    "-j",
                    "DNAT",
                    "--to-destination",
                    &to_dest,
                ])
                .status()
                .context("iptables check for DNAT rule")?;
            if !check.success() {
                run_cmd(&[
                    "iptables",
                    "-t",
                    "nat",
                    "-A",
                    "PREROUTING",
                    "-p",
                    &proto,
                    "--dport",
                    &dport,
                    "-j",
                    "DNAT",
                    "--to-destination",
                    &to_dest,
                ])
                .context("add DNAT rule")?;
            }
            tracing::info!(
                host_port = pm.host_port,
                container_port = pm.container_port,
                proto = %proto,
                "network: port mapping added"
            );
        }
        Ok(())
    }

    /// Derive a short 8-char hex prefix from a container ID for veth naming.
    fn veth_prefix(container_id: &str) -> String {
        // veth interface names must be ≤15 chars; "veth-" + 8 = 13, safe.
        container_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect::<String>()
            .to_lowercase()
    }

    fn net_context_path(container_id: &str) -> std::path::PathBuf {
        std::path::Path::new("/run/minibox/net").join(format!("{container_id}.json"))
    }
}

fn run_cmd(args: &[&str]) -> Result<()> {
    let status = Command::new(args[0])
        .args(&args[1..])
        .status()
        .with_context(|| format!("spawn {}", args[0]))?;
    if !status.success() {
        anyhow::bail!("command failed: {}", args.join(" "));
    }
    Ok(())
}

fn read_stat_file(path: &Path) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[async_trait]
impl NetworkProvider for BridgeNetwork {
    /// Setup bridge networking for a container.
    ///
    /// Returns a JSON blob with `container_ip`, `ceth`, `veth`, `gateway`, and `dns`
    /// fields. The caller stores this as `/run/minibox/net/{container_id}.json` so
    /// that `attach` can read it.
    async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> {
        self.ensure_bridge().context("bridge: ensure bridge")?;
        self.ensure_nat().context("bridge: ensure NAT")?;

        let prefix = Self::veth_prefix(container_id);
        let veth = format!("veth-{prefix}");
        let ceth = format!("ceth-{prefix}");

        // Allocate an IP for the container.
        let container_ip = self
            .ip_alloc
            .lock()
            .unwrap()
            .allocate()
            .ok_or_else(|| anyhow::anyhow!("bridge: IP pool exhausted"))?;

        // Create veth pair.
        run_cmd(&[
            "ip", "link", "add", &veth, "type", "veth", "peer", "name", &ceth,
        ])
        .context("create veth pair")?;

        // Attach host side to bridge.
        run_cmd(&["ip", "link", "set", &veth, "master", &self.bridge_name])
            .context("attach veth to bridge")?;
        run_cmd(&["ip", "link", "set", &veth, "up"]).context("bring host veth up")?;

        let gateway = self.ip_alloc.lock().unwrap().gateway().to_string();
        let prefix_len = self.subnet.prefix_len();

        // Use DNS from config if provided, otherwise fall back to defaults.
        let dns: Vec<String> = if config.dns_servers.is_empty() {
            vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]
        } else {
            config.dns_servers.clone()
        };

        // Apply port mappings before persisting context.
        self.apply_port_mappings(&container_ip.to_string(), &config.port_mappings)?;

        // Persist context for attach().
        let port_mappings_json: Vec<serde_json::Value> = config
            .port_mappings
            .iter()
            .map(|pm| {
                serde_json::json!({
                    "proto": pm.protocol.to_string(),
                    "host_port": pm.host_port,
                    "container_port": pm.container_port,
                    "container_ip": container_ip.to_string(),
                })
            })
            .collect();

        let ctx = serde_json::json!({
            "container_ip": container_ip.to_string(),
            "prefix_len": prefix_len,
            "ceth": ceth,
            "veth": veth,
            "gateway": gateway,
            "dns": dns,
            "port_mappings": port_mappings_json,
        });
        let ctx_path = Self::net_context_path(container_id);
        if let Some(parent) = ctx_path.parent() {
            std::fs::create_dir_all(parent).context("create /run/minibox/net")?;
        }
        std::fs::write(&ctx_path, ctx.to_string()).context("write net context")?;

        tracing::info!(
            container_id = container_id,
            container_ip = %container_ip,
            veth = %veth,
            gateway = %gateway,
            "bridge: network setup complete"
        );

        Ok(ctx.to_string())
    }

    /// Move `ceth` into the container network namespace and configure IP/routes.
    async fn attach(&self, container_id: &str, pid: u32) -> Result<()> {
        let ctx_path = Self::net_context_path(container_id);
        let ctx_raw = std::fs::read_to_string(&ctx_path)
            .with_context(|| format!("read net context for {container_id}"))?;
        let ctx: serde_json::Value = serde_json::from_str(&ctx_raw).context("parse net context")?;

        let container_ip = ctx["container_ip"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing container_ip in net context"))?;
        let prefix_len = ctx["prefix_len"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing prefix_len in net context"))?;
        let ceth = ctx["ceth"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing ceth in net context"))?;
        let gateway = ctx["gateway"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing gateway in net context"))?;
        let dns_servers: Vec<String> = ctx["dns"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let pid_str = pid.to_string();

        // Move container end of veth into container's network namespace.
        run_cmd(&["ip", "link", "set", ceth, "netns", &pid_str])
            .context("move ceth into container netns")?;

        // Configure IP address inside the container.
        let ip_cidr = format!("{container_ip}/{prefix_len}");
        run_cmd(&[
            "nsenter", "-t", &pid_str, "-n", "ip", "addr", "add", &ip_cidr, "dev", ceth,
        ])
        .context("configure container IP")?;

        run_cmd(&[
            "nsenter", "-t", &pid_str, "-n", "ip", "link", "set", ceth, "up",
        ])
        .context("bring ceth up inside container")?;

        run_cmd(&[
            "nsenter", "-t", &pid_str, "-n", "ip", "route", "add", "default", "via", gateway,
        ])
        .context("add default route in container")?;

        // Write resolv.conf inside the container via nsenter.
        if !dns_servers.is_empty() {
            let resolv = dns_servers
                .iter()
                .map(|s| format!("nameserver {s}"))
                .collect::<Vec<_>>()
                .join("\n");
            // Use tee via nsenter to write resolv.conf.
            let output = Command::new("nsenter")
                .args(["-t", &pid_str, "-m", "--"])
                .arg("sh")
                .arg("-c")
                .arg(format!("printf '{resolv}\\n' > /etc/resolv.conf"))
                .output()
                .context("write resolv.conf")?;
            if !output.status.success() {
                tracing::warn!(
                    container_id = container_id,
                    "bridge: could not write resolv.conf inside container"
                );
            }
        }

        tracing::info!(
            container_id = container_id,
            pid = pid,
            container_ip = container_ip,
            "bridge: network attached"
        );

        Ok(())
    }

    /// Delete the veth pair and remove the net context file.
    async fn cleanup(&self, container_id: &str) -> Result<()> {
        let ctx_path = Self::net_context_path(container_id);

        // Best-effort: read context to find veth name, delete it, and remove port mappings.
        #[allow(clippy::collapsible_if)]
        if let Ok(ctx_raw) = std::fs::read_to_string(&ctx_path) {
            if let Ok(ctx) = serde_json::from_str::<serde_json::Value>(&ctx_raw) {
                if let Some(veth) = ctx["veth"].as_str() {
                    if let Err(e) = run_cmd(&["ip", "link", "delete", veth]) {
                        tracing::warn!(
                            container_id = container_id,
                            veth = veth,
                            error = %e,
                            "bridge: veth delete failed (already gone?)"
                        );
                    }
                }
                // Release allocated IP back to the pool.
                if let Some(ip_str) = ctx["container_ip"]
                    .as_str()
                    .and_then(|s| s.parse::<IpAddr>().ok())
                {
                    self.ip_alloc.lock().unwrap().release(ip_str);
                }
                // Remove port mapping rules.
                if let Some(mappings) = ctx["port_mappings"].as_array() {
                    for m in mappings {
                        let proto = m["proto"].as_str().unwrap_or("tcp");
                        let dport = m["host_port"].to_string();
                        let to_dest = format!(
                            "{}:{}",
                            m["container_ip"].as_str().unwrap_or(""),
                            m["container_port"]
                        );
                        let _ = run_cmd(&[
                            "iptables",
                            "-t",
                            "nat",
                            "-D",
                            "PREROUTING",
                            "-p",
                            proto,
                            "--dport",
                            &dport,
                            "-j",
                            "DNAT",
                            "--to-destination",
                            &to_dest,
                        ]);
                    }
                }
            }
        }

        if let Err(e) = std::fs::remove_file(&ctx_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                container_id = container_id,
                error = %e,
                "bridge: could not remove net context file"
            );
        }

        tracing::info!(
            container_id = container_id,
            "bridge: network cleanup complete"
        );
        Ok(())
    }

    /// Read per-interface counters from `/sys/class/net/{veth}/statistics/`.
    async fn stats(&self, container_id: &str) -> Result<NetworkStats> {
        let ctx_path = Self::net_context_path(container_id);
        let veth = if let Ok(ctx_raw) = std::fs::read_to_string(&ctx_path) {
            serde_json::from_str::<serde_json::Value>(&ctx_raw)
                .ok()
                .and_then(|v| v["veth"].as_str().map(|s| s.to_string()))
        } else {
            None
        };

        if let Some(veth) = veth {
            let base = Path::new("/sys/class/net").join(&veth).join("statistics");
            Ok(NetworkStats {
                rx_bytes: read_stat_file(&base.join("rx_bytes")),
                rx_packets: read_stat_file(&base.join("rx_packets")),
                rx_errors: read_stat_file(&base.join("rx_errors")),
                rx_dropped: read_stat_file(&base.join("rx_dropped")),
                tx_bytes: read_stat_file(&base.join("tx_bytes")),
                tx_packets: read_stat_file(&base.join("tx_packets")),
                tx_errors: read_stat_file(&base.join("tx_errors")),
                tx_dropped: read_stat_file(&base.join("tx_dropped")),
            })
        } else {
            Ok(NetworkStats::default())
        }
    }
}

minibox_core::as_any!(BridgeNetwork);

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn test_ip_allocator_skips_network_and_gateway() {
        let subnet: ipnet::IpNet = "172.20.0.0/16".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        let first = alloc.allocate().unwrap();
        // Must not be .0 (network) or .1 (gateway)
        assert_ne!(first, "172.20.0.0".parse::<IpAddr>().unwrap());
        assert_ne!(first, "172.20.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(first, "172.20.0.2".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_ip_allocator_release_and_reuse() {
        let subnet: ipnet::IpNet = "172.20.0.0/16".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        let ip1 = alloc.allocate().unwrap();
        alloc.release(ip1);
        let ip2 = alloc.allocate().unwrap();
        assert_eq!(ip1, ip2); // released IP is reused
    }

    /// Issue #134: gateway IP must never be returned by `allocate()`.
    #[test]
    fn ip_allocator_gateway_never_allocated() {
        let subnet: ipnet::IpNet = "10.0.0.0/24".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);
        let expected_gateway: IpAddr = "10.0.0.1".parse().unwrap();

        let mut allocated = vec![];
        while let Some(ip) = alloc.allocate() {
            allocated.push(ip);
        }

        assert!(
            !allocated.contains(&expected_gateway),
            "gateway 10.0.0.1 must never be allocated; got: {allocated:?}"
        );
        assert_eq!(alloc.gateway(), expected_gateway);
    }

    /// Issue #134: exhausted pool must return `None`, never panic.
    #[test]
    fn ip_allocator_exhaustion_returns_none() {
        // /30 has only 2 usable host addresses (.2 and .3); .1 is gateway.
        let subnet: ipnet::IpNet = "192.168.1.0/30".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        assert!(alloc.allocate().is_some(), "first allocation must succeed");
        assert!(alloc.allocate().is_some(), "second allocation must succeed");
        assert!(
            alloc.allocate().is_none(),
            "pool exhausted — must return None"
        );
    }

    /// Issue #134: releasing an IP outside the subnet must be a safe no-op.
    #[test]
    fn ip_allocator_release_out_of_subnet_is_noop() {
        let subnet: ipnet::IpNet = "172.20.0.0/16".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        let foreign: IpAddr = "10.0.0.5".parse().unwrap();
        alloc.release(foreign); // must not panic

        let ip = alloc.allocate().unwrap();
        assert_eq!(ip, "172.20.0.2".parse::<IpAddr>().unwrap());
    }

    /// Issue #134: allocations must be sequential starting at .2.
    #[test]
    fn ip_allocator_sequential_allocation() {
        let subnet: ipnet::IpNet = "172.20.0.0/16".parse().unwrap();
        let mut alloc = IpAllocator::new(subnet);

        let ip1 = alloc.allocate().unwrap();
        let ip2 = alloc.allocate().unwrap();
        let ip3 = alloc.allocate().unwrap();

        assert_eq!(ip1, "172.20.0.2".parse::<IpAddr>().unwrap());
        assert_eq!(ip2, "172.20.0.3".parse::<IpAddr>().unwrap());
        assert_eq!(ip3, "172.20.0.4".parse::<IpAddr>().unwrap());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod integration_tests {
    use super::*;

    /// Run with: just test-integration (requires root + Linux)
    ///
    /// Verifies BridgeNetwork can create a bridge interface without panicking.
    /// Full attach() test requires a running container — see e2e suite.
    #[tokio::test]
    #[ignore = "requires root and Linux kernel with bridge support"]
    async fn test_bridge_setup_creates_interface() {
        let bridge = BridgeNetwork::new().expect("BridgeNetwork init");
        bridge.ensure_bridge().expect("ensure_bridge");

        // Verify minibox0 exists
        let status = std::process::Command::new("ip")
            .args(["link", "show", "minibox0"])
            .status()
            .unwrap();
        assert!(
            status.success(),
            "minibox0 bridge should exist after ensure_bridge()"
        );
    }
}
