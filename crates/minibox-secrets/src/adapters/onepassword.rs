use std::{process::Command, sync::Arc, time::Duration};

use crate::domain::{
    CacheHint, CredentialError, CredentialProvider, CredentialRef, CredentialScheme,
    FetchedCredential, FetchedCredentialInner, build_credential,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetches credentials via the `op` CLI (1Password).
///
/// Ref format: `op://vault/item/field:<kind>`
///
/// Requires `op` to be installed and a session to be active (biometric unlock or
/// `op signin`). The crate does not manage the 1Password session.
pub struct OnePasswordProvider {
    timeout: Duration,
}

impl OnePasswordProvider {
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Default for OnePasswordProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CredentialProvider for OnePasswordProvider {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError> {
        if r.scheme != CredentialScheme::OnePassword {
            return Err(CredentialError::NotFound(format!(
                "OnePasswordProvider cannot handle scheme {:?}",
                r.scheme
            )));
        }

        let op_ref = format!("op://{}", r.path);
        let kind = r.kind;
        let timeout = self.timeout;

        let output = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                Command::new("op").args(["read", &op_ref]).output()
            }),
        )
        .await
        .map_err(|_| {
            CredentialError::ProviderUnavailable(format!(
                "op timed out after {}s",
                timeout.as_secs()
            ))
        })?
        .map_err(|e| CredentialError::ProviderUnavailable(e.to_string()))?
        .map_err(|e| {
            CredentialError::ProviderUnavailable(format!("`op` not found or not executable: {e}"))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
            return if stderr.contains("not signed in")
                || stderr.contains("unauthorized")
                || stderr.contains("authentication")
            {
                Err(CredentialError::ProviderUnavailable(
                    "1Password: not signed in".into(),
                ))
            } else {
                Err(CredentialError::NotFound(r.path.clone()))
            };
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            return Err(CredentialError::InvalidFormat(
                "op read returned empty output".into(),
            ));
        }

        Ok(Arc::new(FetchedCredentialInner {
            credential: build_credential(kind, raw),
            cache: CacheHint::Session,
        }))
    }
}
