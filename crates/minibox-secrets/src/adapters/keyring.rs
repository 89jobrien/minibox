use std::sync::Arc;

use crate::domain::{
    CacheHint, CredentialError, CredentialProvider, CredentialRef, CredentialScheme,
    FetchedCredential, FetchedCredentialInner, build_credential,
};

/// Reads credentials from the OS keychain via the `keyring` crate.
///
/// Ref format: `keyring:<service>/<username>:<kind>`
///
/// On macOS: Keychain. On Linux: libsecret / KWallet. On Windows: Credential Store.
pub struct KeyringProvider;

#[async_trait::async_trait]
impl CredentialProvider for KeyringProvider {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError> {
        if r.scheme != CredentialScheme::Keyring {
            return Err(CredentialError::NotFound(format!(
                "KeyringProvider cannot handle scheme {:?}",
                r.scheme
            )));
        }

        // path = "<service>/<username>"
        let (service, username) = r.path.split_once('/').ok_or_else(|| {
            CredentialError::InvalidFormat(format!(
                "keyring ref path must be `service/username`, got `{}`",
                r.path
            ))
        })?;

        let service = service.to_string();
        let username = username.to_string();
        let kind = r.kind;

        let raw = tokio::task::spawn_blocking(move || {
            keyring::Entry::new(&service, &username)
                .map_err(|e| CredentialError::ProviderUnavailable(e.to_string()))?
                .get_password()
                .map_err(|e| match e {
                    keyring::Error::NoEntry => {
                        CredentialError::NotFound(format!("{service}/{username}"))
                    }
                    other => CredentialError::ProviderUnavailable(other.to_string()),
                })
        })
        .await
        .map_err(|e| CredentialError::ProviderUnavailable(e.to_string()))??;

        Ok(Arc::new(FetchedCredentialInner {
            credential: build_credential(kind, raw),
            cache: CacheHint::Session,
        }))
    }
}
