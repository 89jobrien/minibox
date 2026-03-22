use std::sync::Arc;

use crate::domain::{
    CacheHint, CredentialError, CredentialProvider, CredentialRef, CredentialScheme,
    FetchedCredential, FetchedCredentialInner, build_credential,
};

/// Reads credentials from environment variables.
///
/// Ref format: `env:<VAR_NAME>:<kind>`
pub struct EnvProvider;

#[async_trait::async_trait]
impl CredentialProvider for EnvProvider {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError> {
        if r.scheme != CredentialScheme::Env {
            return Err(CredentialError::NotFound(format!(
                "EnvProvider cannot handle scheme {:?}",
                r.scheme
            )));
        }

        let raw = std::env::var(&r.path)
            .map_err(|_| CredentialError::NotFound(format!("env var `{}` not set", r.path)))?;

        let credential = build_credential(r.kind, raw);

        Ok(Arc::new(FetchedCredentialInner {
            credential,
            cache: CacheHint::Never,
        }))
    }
}
