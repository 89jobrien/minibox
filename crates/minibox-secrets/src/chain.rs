use std::collections::HashMap;

use chrono::Utc;
use tokio::sync::RwLock;
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
///
/// Use `clear()` or `invalidate()` to drop cached credentials on demand (e.g. after
/// credential rotation or session logout).
pub struct CredentialProviderChain {
    providers: Vec<Box<dyn CredentialProvider>>,
    /// RwLock: many concurrent cache hits don't block each other.
    cache: RwLock<HashMap<String, FetchedCredential>>,
}

impl CredentialProviderChain {
    pub fn new(providers: Vec<Box<dyn CredentialProvider>>) -> Self {
        Self {
            providers,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Evict all cached credentials.
    pub async fn clear(&self) {
        self.cache.write().await.clear();
    }

    /// Evict a single cached credential by ref string.
    pub async fn invalidate(&self, ref_str: &str) {
        self.cache.write().await.remove(ref_str);
    }
}

#[async_trait::async_trait]
impl CredentialProvider for CredentialProviderChain {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError> {
        let key = r.as_str().to_string();

        // Cache read — RwLock allows concurrent hits without blocking
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.get(&key) {
                match &cached.cache {
                    CacheHint::Never => {}
                    CacheHint::Session => return Ok(cached.clone()),
                    CacheHint::Until(t) => {
                        if Utc::now() < *t {
                            return Ok(cached.clone());
                        }
                        // Expired — drop read lock and fall through to evict + re-fetch
                        drop(guard);
                        self.cache.write().await.remove(&key);
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
                    if let Err(ve) = validate_credential(&fetched.credential) {
                        warn!(
                            ref_key = key,
                            error = %ve,
                            "chain: credential failed validation, trying next provider"
                        );
                        last_err = Some(CredentialError::Validation(ve));
                        continue;
                    }

                    if !matches!(fetched.cache, CacheHint::Never) {
                        self.cache.write().await.insert(key, fetched.clone());
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

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn never_hint_bypasses_cache() {
        let r = ref_for("env:SOME_VAR:api_key");

        let fetched = Arc::new(FetchedCredentialInner {
            credential: Credential::ApiKey(ApiKey::new("sk-envval123")),
            cache: CacheHint::Never,
        });

        let mut p = InMemoryProvider::new();
        p.insert(r.as_str(), fetched);

        let chain = CredentialProviderChain::new(vec![Box::new(p)]);
        let _ = chain.get(&r).await.unwrap();

        assert!(chain.cache.read().await.is_empty());
    }

    #[tokio::test]
    async fn invalidate_removes_entry() {
        let r = ref_for("inmemory:my-key:api_key");

        let mut p = InMemoryProvider::new();
        p.insert(r.as_str(), make_api_key_fetched("sk-cached1234"));

        let chain = CredentialProviderChain::new(vec![Box::new(p)]);
        let _ = chain.get(&r).await.unwrap();

        assert!(!chain.cache.read().await.is_empty());
        chain.invalidate(r.as_str()).await;
        assert!(chain.cache.read().await.is_empty());
    }

    #[tokio::test]
    async fn clear_removes_all_entries() {
        let r1 = ref_for("inmemory:key-one:api_key");
        let r2 = ref_for("inmemory:key-two:api_key");

        let mut p = InMemoryProvider::new();
        p.insert(r1.as_str(), make_api_key_fetched("sk-first1234"));
        p.insert(r2.as_str(), make_api_key_fetched("sk-second123"));

        let chain = CredentialProviderChain::new(vec![Box::new(p)]);
        let _ = chain.get(&r1).await.unwrap();
        let _ = chain.get(&r2).await.unwrap();

        assert_eq!(chain.cache.read().await.len(), 2);
        chain.clear().await;
        assert!(chain.cache.read().await.is_empty());
    }
}
