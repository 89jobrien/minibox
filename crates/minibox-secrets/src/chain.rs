use std::collections::HashMap;

use chrono::Utc;
use tokio::sync::Mutex;
use tracing::warn;

use crate::domain::{
    CacheHint, Credential, CredentialError, CredentialProvider, CredentialRef, FetchedCredential,
    Validate, ValidationError,
};

fn validate_credential(c: &Credential) -> Result<(), ValidationError> {
    match c {
        Credential::ApiKey(k) => k.validate(),
        Credential::Token(t) => t.validate(),
        Credential::DatabaseUrl(u) => u.validate(),
        Credential::SshKey(k) => k.validate(),
    }
}

/// Tries a ranked list of providers in order, returning the first successful result.
///
/// Cache behaviour per `CacheHint`:
/// - `Never` — always bypasses cache and re-fetches.
/// - `Session` — cached for the lifetime of the process.
/// - `Until(t)` — cached until `t`; evicted and re-fetched after expiry.
pub struct CredentialProviderChain {
    providers: Vec<Box<dyn CredentialProvider>>,
    cache: Mutex<HashMap<String, FetchedCredential>>,
}

impl CredentialProviderChain {
    pub fn new(providers: Vec<Box<dyn CredentialProvider>>) -> Self {
        Self {
            providers,
            cache: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl CredentialProvider for CredentialProviderChain {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError> {
        let key = r.as_str().to_string();

        // Cache check — skip entirely for Never
        {
            let mut guard = self.cache.lock().await;
            if let Some(cached) = guard.get(&key) {
                match &cached.cache {
                    CacheHint::Never => {}
                    CacheHint::Session => return Ok(cached.clone()),
                    CacheHint::Until(t) => {
                        if Utc::now() < *t {
                            return Ok(cached.clone());
                        }
                        guard.remove(&key);
                    }
                }
            }
        }

        // Provider loop
        let mut last_err: Option<CredentialError> = None;
        let mut all_not_found = true;

        for provider in &self.providers {
            match provider.get(r).await {
                Ok(fetched) => {
                    // Validate before accepting
                    if let Err(ve) = validate_credential(&fetched.credential) {
                        warn!(
                            ref_key = key,
                            error = %ve,
                            "chain: credential failed validation, trying next provider"
                        );
                        last_err = Some(CredentialError::Validation(ve));
                        continue;
                    }

                    // Cache if not Never
                    if !matches!(fetched.cache, CacheHint::Never) {
                        self.cache.lock().await.insert(key, fetched.clone());
                    }

                    return Ok(fetched);
                }
                Err(CredentialError::NotFound(_)) => {
                    // Try next provider silently
                }
                Err(e @ CredentialError::ProviderUnavailable(_)) => {
                    warn!(ref_key = key, error = %e, "chain: provider unavailable, trying next");
                    all_not_found = false;
                    last_err = Some(e);
                }
                Err(e) => {
                    warn!(ref_key = key, error = %e, "chain: provider error, trying next");
                    all_not_found = false;
                    last_err = Some(e);
                }
            }
        }

        Err(if all_not_found {
            CredentialError::NotFound(key)
        } else {
            last_err.unwrap_or_else(|| CredentialError::NotFound(key))
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::adapters::in_memory::InMemoryProvider;
    use crate::domain::{ApiKey, CacheHint, Credential, FetchedCredentialInner};

    use super::*;

    fn make_api_key_fetched(raw: &str) -> FetchedCredential {
        Arc::new(FetchedCredentialInner {
            credential: Credential::ApiKey(ApiKey::new(raw)),
            cache: CacheHint::Session,
        })
    }

    fn ref_for(s: &str) -> CredentialRef {
        CredentialRef::parse(s).unwrap()
    }

    #[tokio::test]
    async fn returns_first_match() {
        let r = ref_for("inmemory:my-key:api_key");

        let mut p1 = InMemoryProvider::new();
        p1.insert(r.as_str(), make_api_key_fetched("sk-first1234"));

        let mut p2 = InMemoryProvider::new();
        p2.insert(r.as_str(), make_api_key_fetched("sk-second123"));

        let chain = CredentialProviderChain::new(vec![Box::new(p1), Box::new(p2)]);
        let fetched = chain.get(&r).await.unwrap();

        if let Credential::ApiKey(k) = &fetched.credential {
            assert_eq!(k.hash, crate::domain::sha256_hex(b"sk-first1234"));
        } else {
            panic!("expected ApiKey");
        }
    }

    #[tokio::test]
    async fn falls_through_to_second_provider() {
        let r = ref_for("inmemory:my-key:api_key");

        let p1 = InMemoryProvider::new(); // empty — NotFound

        let mut p2 = InMemoryProvider::new();
        p2.insert(r.as_str(), make_api_key_fetched("sk-second123"));

        let chain = CredentialProviderChain::new(vec![Box::new(p1), Box::new(p2)]);
        let fetched = chain.get(&r).await.unwrap();

        assert!(matches!(fetched.credential, Credential::ApiKey(_)));
    }

    #[tokio::test]
    async fn all_not_found_returns_not_found() {
        let r = ref_for("inmemory:missing:api_key");
        let chain = CredentialProviderChain::new(vec![
            Box::new(InMemoryProvider::new()),
            Box::new(InMemoryProvider::new()),
        ]);
        assert!(matches!(
            chain.get(&r).await,
            Err(CredentialError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn session_cache_hit() {
        let r = ref_for("inmemory:my-key:api_key");

        let mut p = InMemoryProvider::new();
        p.insert(r.as_str(), make_api_key_fetched("sk-cached1234"));

        let chain = CredentialProviderChain::new(vec![Box::new(p)]);

        let first = chain.get(&r).await.unwrap();
        let second = chain.get(&r).await.unwrap();

        // Both should return the same Arc
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn never_hint_bypasses_cache() {
        let r = ref_for("env:SOME_VAR:api_key");

        // Use InMemoryProvider with CacheHint::Never
        let fetched = Arc::new(FetchedCredentialInner {
            credential: Credential::ApiKey(ApiKey::new("sk-envval123")),
            cache: CacheHint::Never,
        });

        let mut p = InMemoryProvider::new();
        p.insert(r.as_str(), fetched);

        let chain = CredentialProviderChain::new(vec![Box::new(p)]);
        let _ = chain.get(&r).await.unwrap();

        // Cache should be empty
        let guard = chain.cache.lock().await;
        assert!(guard.is_empty());
    }
}
