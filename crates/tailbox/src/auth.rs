use anyhow::{bail, Result};
use minibox_core::domain::NetworkConfig;

/// Resolve a Tailscale auth key from the priority chain:
///
/// 1. `config.tailnet_auth_key` — inline key in the `RunContainer` request.
/// 2. `minibox-secrets` lookup via `config.tailnet_secret_name` (if set) or
///    `default_secret_name` (default `"tailscale-auth-key"`).
/// 3. `TAILSCALE_AUTH_KEY` environment variable.
/// 4. `Err(...)` — no key available.
pub async fn resolve_auth_key(
    config: &NetworkConfig,
    default_secret_name: &str,
) -> Result<String> {
    // Step 1: inline key
    if let Some(key) = config.tailnet_auth_key.as_deref()
        && !key.is_empty()
    {
        return Ok(key.to_string());
    }

    // Step 2: minibox-secrets lookup
    let secret_name = config
        .tailnet_secret_name
        .as_deref()
        .unwrap_or(default_secret_name);
    if let Ok(key) = lookup_secret(secret_name).await
        && !key.is_empty()
    {
        return Ok(key);
    }

    // Step 3: environment variable
    if let Ok(key) = std::env::var("TAILSCALE_AUTH_KEY")
        && !key.is_empty()
    {
        return Ok(key);
    }

    bail!(
        "tailnet: no auth key found — set tailnet_auth_key in RunContainer, \
         configure minibox-secrets key '{}', or set TAILSCALE_AUTH_KEY",
        secret_name
    )
}

/// Look up a secret from the minibox-secrets provider chain.
async fn lookup_secret(name: &str) -> Result<String> {
    // Full provider chain wired in Task 3. Stub: env-based lookup only.
    let _ = name;
    Err(anyhow::anyhow!("tailnet: secrets provider not yet configured"))
}
