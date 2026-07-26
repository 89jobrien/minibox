//! Result types returned by a CNI plugin chain's ADD command, and the
//! CNI-spec structured error payload a plugin may return on failure.

use serde::{Deserialize, Serialize};

/// Merged result of a CNI ADD chain (the final `prevResult`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CniResult {
    /// CNI spec version the result conforms to.
    #[serde(rename = "cniVersion")]
    pub cni_version: String,
    /// Network interfaces created by the chain.
    #[serde(default)]
    pub interfaces: Vec<CniInterface>,
    /// IP configurations allocated by the chain.
    #[serde(default)]
    pub ips: Vec<CniIpConfig>,
    /// DNS configuration reported by the chain.
    #[serde(default)]
    pub dns: CniDns,
}

/// A network interface reported by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CniInterface {
    /// Interface name inside the container (e.g. `"eth0"`).
    pub name: String,
    /// MAC address, if reported.
    #[serde(default)]
    pub mac: Option<String>,
    /// Network namespace path the interface lives in, if reported.
    #[serde(default)]
    pub sandbox: Option<String>,
}

/// An allocated IP configuration reported by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CniIpConfig {
    /// CIDR address (e.g. `"10.88.0.5/24"`).
    pub address: String,
    /// Gateway address, if reported.
    #[serde(default)]
    pub gateway: Option<String>,
    /// Index into `CniResult::interfaces` this IP belongs to.
    #[serde(default)]
    pub interface: Option<usize>,
}

/// DNS configuration reported by a plugin (e.g. the `dnsname` plugin).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CniDns {
    /// Nameserver IPs.
    #[serde(default)]
    pub nameservers: Vec<String>,
    /// Search domain.
    #[serde(default)]
    pub domain: Option<String>,
    /// Search list.
    #[serde(default)]
    pub search: Vec<String>,
    /// Resolver options.
    #[serde(default)]
    pub options: Vec<String>,
}

/// A CNI-spec structured error object, as returned by a well-behaved
/// plugin on stdout when it fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CniErrorPayload {
    /// CNI spec error code.
    pub code: u32,
    /// Human-readable error message.
    pub msg: String,
    /// Optional additional detail.
    #[serde(default)]
    pub details: Option<String>,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn cni_result_deserializes_add_output() {
        let json = r#"{
            "cniVersion": "1.0.0",
            "interfaces": [{"name": "eth0", "mac": "aa:bb:cc:dd:ee:ff"}],
            "ips": [{"address": "10.88.0.5/24", "gateway": "10.88.0.1", "interface": 0}],
            "dns": {"nameservers": ["10.88.0.1"]}
        }"#;
        let result: CniResult = serde_json::from_str(json).expect("deserialize");
        assert_eq!(result.cni_version, "1.0.0");
        assert_eq!(result.interfaces[0].name, "eth0");
        assert_eq!(result.ips[0].address, "10.88.0.5/24");
        assert_eq!(result.dns.nameservers, vec!["10.88.0.1".to_string()]);
    }

    #[test]
    fn cni_result_defaults_missing_optional_fields() {
        let json = r#"{"cniVersion": "1.0.0"}"#;
        let result: CniResult = serde_json::from_str(json).expect("deserialize");
        assert!(result.interfaces.is_empty());
        assert!(result.ips.is_empty());
        assert!(result.dns.nameservers.is_empty());
    }

    #[test]
    fn cni_error_payload_deserializes_spec_error() {
        let json = r#"{"code": 7, "msg": "no IPs", "details": "pool exhausted"}"#;
        let payload: CniErrorPayload = serde_json::from_str(json).expect("deserialize");
        assert_eq!(payload.code, 7);
        assert_eq!(payload.msg, "no IPs");
        assert_eq!(payload.details.as_deref(), Some("pool exhausted"));
    }
}
