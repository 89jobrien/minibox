/// Configuration for the tailnet network adapter.
#[derive(Debug, Clone)]
pub struct TailnetConfig {
    /// Default auth key for the daemon gateway device.
    /// If `None`, falls back to `key_secret_name` then `TAILSCALE_AUTH_KEY`.
    pub auth_key: Option<String>,
    /// minibox-secrets key name to look up when `auth_key` is None.
    /// Defaults to `"tailscale-auth-key"`.
    pub key_secret_name: String,
}

impl Default for TailnetConfig {
    fn default() -> Self {
        Self {
            auth_key: None,
            key_secret_name: "tailscale-auth-key".to_string(),
        }
    }
}
