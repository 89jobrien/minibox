use anyhow::{Context, Result, bail};
use minibox_core::domain::NetworkConfig;
use minibox_secrets::adapters::env::EnvProvider;
use minibox_secrets::{
    CredentialProvider, CredentialProviderChain,
    domain::{Credential, CredentialRef},
};
use secrecy::ExposeSecret;

/// Resolve a Tailscale auth key from the priority chain:
///
/// 1. `config.tailnet_auth_key` — inline key in the `RunContainer` request.
/// 2. `minibox-secrets` lookup via `config.tailnet_secret_name` (if set) or
///    `default_secret_name` (default `"tailscale-auth-key"`).
/// 3. `TAILSCALE_AUTH_KEY` environment variable.
/// 4. `Err(...)` — no key available.
pub async fn resolve_auth_key(config: &NetworkConfig, default_secret_name: &str) -> Result<String> {
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
    match lookup_secret(secret_name).await {
        Ok(key) if !key.is_empty() => return Ok(key),
        Ok(_) => {} // empty string — fall through
        Err(e) => {
            tracing::warn!(
                secret_name = secret_name,
                error = %e,
                "tailnet: secrets lookup failed, falling through to env var"
            );
        }
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
///
/// Converts `name` to an env-var ref by uppercasing and replacing `-` with `_`,
/// then queries an `EnvProvider`-backed `CredentialProviderChain`.
///
/// Example: `"tailscale-auth-key"` → `env:TAILSCALE_AUTH_KEY:api_key`
async fn lookup_secret(name: &str) -> Result<String> {
    let env_var = name.to_uppercase().replace('-', "_");
    let ref_str = format!("env:{env_var}:api_key");
    let cred_ref = CredentialRef::parse(&ref_str)
        .with_context(|| format!("tailnet: failed to build credential ref for '{name}'"))?;

    let chain = CredentialProviderChain::new(vec![Box::new(EnvProvider)]);
    let fetched = chain
        .get(&cred_ref)
        .await
        .with_context(|| format!("tailnet: secret '{name}' not found in provider chain"))?;

    match &fetched.credential {
        Credential::ApiKey(k) => Ok(k.key.expose_secret().to_string()),
        Credential::Token(t) => Ok(t.token.expose_secret().to_string()),
        other => bail!(
            "tailnet: unexpected credential kind {:?} for secret '{name}'",
            std::mem::discriminant(other)
        ),
    }
}
