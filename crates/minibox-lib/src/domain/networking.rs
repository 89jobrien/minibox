//! Networking domain traits for container network isolation.
//!
//! Defines the contract for network providers that implement container
//! network isolation, bridge setup, and port forwarding.

use super::AsAny;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Abstraction for container networking operations.
///
/// Implementations might include Linux bridge networking, CNI plugins,
/// or cloud provider networking.
///
/// # Example
///
/// ```rust,ignore
/// let network = BridgeNetworking::new();
/// let config = NetworkConfig {
///     bridge_name: "minibox0".to_string(),
///     subnet: "172.18.0.0/16".to_string(),
///     container_ip: Some("172.18.0.2".to_string()),
///     port_mappings: vec![
///         PortMapping { host_port: 8080, container_port: 80, protocol: Protocol::Tcp }
///     ],
/// };
///
/// let net_ns = network.setup("container-abc123", &config)?;
/// // Container now has network connectivity
/// network.cleanup("container-abc123")?;
/// ```
#[async_trait]
pub trait NetworkProvider: AsAny + Send + Sync {
    /// Setup networking for a container.
    ///
    /// Creates network namespace, veth pair, bridge attachment, and
    /// configures IP addressing.
    ///
    /// # Arguments
    ///
    /// * `container_id` - Unique container identifier
    /// * `config` - Network configuration
    ///
    /// # Returns
    ///
    /// Path to network namespace (e.g., `/var/run/netns/container-abc123`)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network namespace creation fails
    /// - Bridge doesn't exist
    /// - IP allocation conflicts
    /// - veth pair creation fails
    async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String>;

    /// Attach a running container to its network namespace.
    ///
    /// Called after container process starts to move it into network namespace.
    ///
    /// # Arguments
    ///
    /// * `container_id` - Container identifier
    /// * `pid` - Container init process PID
    async fn attach(&self, container_id: &str, pid: u32) -> Result<()>;

    /// Cleanup networking for a stopped container.
    ///
    /// Removes veth pair, releases IP, deletes network namespace.
    async fn cleanup(&self, container_id: &str) -> Result<()>;

    /// Get network statistics for a container.
    ///
    /// Returns bytes sent/received, packet counts, etc.
    async fn stats(&self, container_id: &str) -> Result<NetworkStats>;
}

/// Network configuration for a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Bridge name (e.g., "minibox0", "docker0")
    pub bridge_name: String,

    /// Subnet for bridge (CIDR notation, e.g., "172.18.0.0/16")
    pub subnet: String,

    /// Assigned container IP (optional, auto-allocated if None)
    pub container_ip: Option<String>,

    /// Port mappings from host to container
    pub port_mappings: Vec<PortMapping>,

    /// DNS servers to configure in container
    pub dns_servers: Vec<String>,

    /// Enable IPv6 support
    pub ipv6_enabled: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            bridge_name: "minibox0".to_string(),
            subnet: "172.18.0.0/16".to_string(),
            container_ip: None,
            port_mappings: Vec::new(),
            dns_servers: vec!["8.8.8.8".to_string(), "8.8.4.4".to_string()],
            ipv6_enabled: false,
        }
    }
}

/// Port mapping from host to container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    /// Host port number
    pub host_port: u16,

    /// Container port number
    pub container_port: u16,

    /// Protocol (TCP, UDP, SCTP)
    pub protocol: Protocol,

    /// Host interface to bind (None = all interfaces)
    pub host_interface: Option<String>,
}

/// Network protocol for port mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    /// Transmission Control Protocol
    Tcp,
    /// User Datagram Protocol
    Udp,
    /// Stream Control Transmission Protocol
    Sctp,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::Tcp => write!(f, "tcp"),
            Protocol::Udp => write!(f, "udp"),
            Protocol::Sctp => write!(f, "sctp"),
        }
    }
}

/// Network statistics for a container.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkStats {
    /// Bytes received by container
    pub rx_bytes: u64,

    /// Packets received by container
    pub rx_packets: u64,

    /// Receive errors
    pub rx_errors: u64,

    /// Receive drops
    pub rx_dropped: u64,

    /// Bytes transmitted by container
    pub tx_bytes: u64,

    /// Packets transmitted by container
    pub tx_packets: u64,

    /// Transmit errors
    pub tx_errors: u64,

    /// Transmit drops
    pub tx_dropped: u64,
}
