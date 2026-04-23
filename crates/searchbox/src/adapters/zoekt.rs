use async_trait::async_trait;
use serde::Deserialize;
use tracing::debug;

use crate::domain::{RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult, SourceType};

pub struct ZoektAdapter {
    /// Base URL of zoekt-webserver, e.g. "http://minibox:6070"
    base_url: String,
    client: reqwest::Client,
}

impl ZoektAdapter {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

// Zoekt search API response shapes
#[derive(Deserialize)]
struct ZoektSearchResponse {
    #[serde(rename = "Result")]
    result: Option<ZoektResult>,
}

#[derive(Deserialize)]
struct ZoektResult {
    #[serde(rename = "Files")]
    files: Option<Vec<ZoektFile>>,
}

#[derive(Deserialize)]
struct ZoektFile {
    #[serde(rename = "Repository")]
    repository: String,
    #[serde(rename = "FileName")]
    file_name: String,
    #[serde(rename = "Branches")]
    branches: Option<Vec<String>>,
    #[serde(rename = "LineMatches")]
    line_matches: Option<Vec<ZoektLineMatch>>,
    #[serde(rename = "Score")]
    score: f64,
}

#[derive(Deserialize)]
struct ZoektLineMatch {
    #[serde(rename = "Line")]
    line: String,
    #[serde(rename = "LineNumber")]
    line_number: u32,
    #[serde(rename = "LineFragments")]
    line_fragments: Option<Vec<ZoektFragment>>,
}

#[derive(Deserialize)]
struct ZoektFragment {
    #[serde(rename = "LineOffset")]
    line_offset: u32,
}

#[derive(Deserialize)]
struct ZoektListResponse {
    #[serde(rename = "Repos")]
    repos: Option<Vec<ZoektRepoEntry>>,
}

#[derive(Deserialize)]
struct ZoektRepoEntry {
    #[serde(rename = "Repository")]
    repository: ZoektRepoInfo,
    #[serde(rename = "Stats")]
    stats: Option<ZoektRepoStats>,
}

#[derive(Deserialize)]
struct ZoektRepoInfo {
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Deserialize)]
struct ZoektRepoStats {
    #[serde(rename = "Documents")]
    documents: Option<u64>,
}

#[async_trait]
impl SearchProvider for ZoektAdapter {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        let url = format!("{}/search", self.base_url);

        // Build Zoekt JSON query
        let mut zoekt_query = query.text.clone();
        if query.case_sensitive {
            zoekt_query = format!("case:yes {zoekt_query}");
        }
        if let Some(lang) = &query.lang {
            zoekt_query = format!("lang:{lang} {zoekt_query}");
        }
        if let Some(repos) = &query.repos {
            let repo_filter = repos
                .iter()
                .map(|r| format!("repo:{r}"))
                .collect::<Vec<_>>()
                .join(" ");
            zoekt_query = format!("{repo_filter} {zoekt_query}");
        }

        let body = serde_json::json!({
            "Q": zoekt_query,
            "Opts": {
                "NumContextLines": query.context_lines,
                "MaxDocDisplayCount": 100,
            }
        });

        debug!(query = %zoekt_query, "ZoektAdapter: searching");

        let resp: ZoektSearchResponse = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Unavailable(e.to_string()))?
            .error_for_status()
            .map_err(|e| SearchError::QueryFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| SearchError::QueryFailed(format!("decode: {e}")))?;

        let files = resp.result.and_then(|r| r.files).unwrap_or_default();
        let mut results = Vec::new();

        for file in files {
            // If result is from a non-main branch, populate commit field
            let commit = file
                .branches
                .as_ref()
                .and_then(|b| {
                    b.iter()
                        .find(|br| *br != "HEAD" && *br != "main" && *br != "master")
                })
                .cloned();

            for lm in file.line_matches.unwrap_or_default() {
                let col = lm
                    .line_fragments
                    .as_ref()
                    .and_then(|f| f.first())
                    .map(|f| f.line_offset)
                    .unwrap_or(0);

                results.push(SearchResult {
                    repo: file.repository.clone(),
                    file: file.file_name.clone(),
                    line: lm.line_number,
                    col,
                    snippet: lm.line.clone(),
                    score: file.score as f32,
                    commit: commit.clone(),
                });
            }
        }

        Ok(results)
    }

    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
        let url = format!("{}/list", self.base_url);
        let body = serde_json::json!({ "Q": { "Repo": ".*" } });

        let resp: ZoektListResponse = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SearchError::Unavailable(e.to_string()))?
            .error_for_status()
            .map_err(|e| SearchError::QueryFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| SearchError::QueryFailed(format!("decode: {e}")))?;

        let repos = resp
            .repos
            .unwrap_or_default()
            .into_iter()
            .map(|e| RepoInfo {
                name: e.repository.name,
                source_type: SourceType::Git,
                last_indexed: None,
                doc_count: e.stats.and_then(|s| s.documents).unwrap_or(0),
            })
            .collect();

        Ok(repos)
    }
}
