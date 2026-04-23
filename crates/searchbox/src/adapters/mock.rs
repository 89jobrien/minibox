use async_trait::async_trait;

use crate::domain::{RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult};

pub struct MockSearchProvider {
    pub results: Vec<SearchResult>,
    pub repos: Vec<RepoInfo>,
    pub fail: bool,
}

impl MockSearchProvider {
    pub fn with_results(results: Vec<SearchResult>) -> Self {
        Self {
            results,
            repos: vec![],
            fail: false,
        }
    }

    pub fn failing() -> Self {
        Self {
            results: vec![],
            repos: vec![],
            fail: true,
        }
    }
}

#[async_trait]
impl SearchProvider for MockSearchProvider {
    async fn search(&self, _query: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        if self.fail {
            return Err(SearchError::Unavailable("mock failure".into()));
        }
        Ok(self.results.clone())
    }

    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
        Ok(self.repos.clone())
    }
}
