//! Integration tests: ZoektAdapter against a live zoekt-webserver.
//! Run with: cargo test -p searchbox --features integration-tests -- --ignored
//!
//! Requires: zoekt-webserver and zoekt-git-index on PATH (or in /opt/zoekt/bin/).
//! Set ZOEKT_BASE_URL to override default http://localhost:6070.

#![cfg(feature = "integration-tests")]

use searchbox::adapters::zoekt::ZoektAdapter;
use searchbox::domain::{SearchProvider, SearchQuery};

fn zoekt_base_url() -> String {
    std::env::var("ZOEKT_BASE_URL").unwrap_or_else(|_| "http://localhost:6070".into())
}

#[tokio::test]
#[ignore = "requires live zoekt-webserver"]
async fn zoekt_adapter_search_returns_results() {
    let adapter = ZoektAdapter::new(zoekt_base_url());
    let results = adapter.search(SearchQuery::new("fn main")).await.unwrap();
    // Just verify we got a response — result count varies by what's indexed
    println!("got {} results", results.len());
}

#[tokio::test]
#[ignore = "requires live zoekt-webserver"]
async fn zoekt_adapter_list_repos_returns_at_least_one() {
    let adapter = ZoektAdapter::new(zoekt_base_url());
    let repos = adapter.list_repos().await.unwrap();
    assert!(!repos.is_empty(), "expected at least one indexed repo");
    println!(
        "repos: {:?}",
        repos.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
}
