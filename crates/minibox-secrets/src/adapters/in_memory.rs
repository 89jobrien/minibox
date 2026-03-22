use std::{collections::HashMap, sync::Arc};

use crate::domain::{
    CredentialError, CredentialProvider, CredentialRef, FetchedCredential, FetchedCredentialInner,
};

/// In-memory provider for tests. Callers insert entries directly by ref string.
pub struct InMemoryProvider {
    entries: HashMap<String, FetchedCredential>,
}

impl InMemoryProvider {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Insert a credential keyed by the raw ref string.
    pub fn insert(&mut self, key: impl Into<String>, credential: FetchedCredential) {
        self.entries.insert(key.into(), credential);
    }

    /// Convenience: insert a credential with `CacheHint::Never`.
    pub fn insert_inner(&mut self, key: impl Into<String>, inner: FetchedCredentialInner) {
        self.entries.insert(key.into(), Arc::new(inner));
    }
}

impl Default for InMemoryProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CredentialProvider for InMemoryProvider {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError> {
        self.entries
            .get(r.as_str())
            .map(Arc::clone)
            .ok_or_else(|| CredentialError::NotFound(r.as_str().to_string()))
    }
}
