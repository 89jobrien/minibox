use async_trait::async_trait;
use futures::future::join_all;
use std::collections::HashSet;
use tracing::warn;

use crate::domain::{RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult};

pub struct MergedAdapter {
    providers: Vec<Box<dyn SearchProvider>>,
}

impl MergedAdapter {
    pub fn new(providers: Vec<Box<dyn SearchProvider>>) -> Self {
        Self { providers }
    }
}

#[async_trait]
impl SearchProvider for MergedAdapter {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        let futs = self.providers.iter().map(|p| p.search(query.clone()));
        let results_per_provider = join_all(futs).await;

        let mut merged: Vec<SearchResult> = Vec::new();
        let mut seen: HashSet<(String, String, u32)> = HashSet::new();

        for result in results_per_provider {
            match result {
                Ok(hits) => {
                    for hit in hits {
                        let key = (hit.repo.clone(), hit.file.clone(), hit.line);
                        if seen.insert(key) {
                            merged.push(hit);
                        }
                    }
                }
                Err(e) => warn!(error = %e, "MergedAdapter: provider error (continuing)"),
            }
        }

        merged.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(merged)
    }

    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
        let futs = self.providers.iter().map(|p| p.list_repos());
        let all = join_all(futs).await;
        let mut repos: Vec<RepoInfo> = all.into_iter().filter_map(|r| r.ok()).flatten().collect();
        repos.dedup_by(|a, b| a.name == b.name);
        Ok(repos)
    }
}
