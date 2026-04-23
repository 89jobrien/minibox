use searchbox::SearchProvider;
use searchbox::adapters::{merged::MergedAdapter, mock::MockSearchProvider};
use searchbox::config::SearchboxConfig;
use searchbox::domain::{SearchQuery, SourceType};

#[test]
fn search_query_defaults() {
    let q = SearchQuery::new("foo");
    assert_eq!(q.text, "foo");
    assert_eq!(q.context_lines, 2);
    assert!(!q.case_sensitive);
    assert!(q.repos.is_none());
}

#[test]
fn source_type_deserializes_from_toml() {
    #[derive(serde::Deserialize)]
    struct Wrapper {
        kind: SourceType,
    }
    let w: Wrapper = toml::from_str("kind = \"git\"").unwrap();
    assert_eq!(w.kind, SourceType::Git);
    let w: Wrapper = toml::from_str("kind = \"fs\"").unwrap();
    assert_eq!(w.kind, SourceType::Filesystem);
    let w: Wrapper = toml::from_str("kind = \"local\"").unwrap();
    assert_eq!(w.kind, SourceType::Local);
}

#[test]
fn config_parses_valid_toml() {
    let toml = r#"
[service]
vps_host = "minibox"

[[repos]]
name = "myrepo"
url  = "git@github.com:user/myrepo.git"
source = "git"
"#;
    let cfg: SearchboxConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.service.vps_host, "minibox");
    assert_eq!(cfg.service.zoekt_port, 6070); // default
    assert_eq!(cfg.repos[0].name, "myrepo");
}

#[test]
fn config_rejects_git_source_without_url() {
    let toml = r#"
[service]
vps_host = "minibox"

[[repos]]
name   = "bad"
source = "git"
"#;
    let cfg: SearchboxConfig = toml::from_str(toml).unwrap();
    assert!(cfg.validate_pub().is_err());
}

#[test]
fn config_rejects_fs_source_without_path() {
    let toml = r#"
[service]
vps_host = "minibox"

[[repos]]
name   = "bad"
source = "fs"
"#;
    let cfg: SearchboxConfig = toml::from_str(toml).unwrap();
    let err = cfg.validate_pub().unwrap_err();
    assert!(err.to_string().contains("path"), "expected 'path' in error: {err}");
}

#[test]
fn config_rejects_local_source_without_path() {
    let toml = r#"
[service]
vps_host = "minibox"

[[repos]]
name   = "bad"
source = "local"
"#;
    let cfg: SearchboxConfig = toml::from_str(toml).unwrap();
    let err = cfg.validate_pub().unwrap_err();
    assert!(err.to_string().contains("path"), "expected 'path' in error: {err}");
}

#[test]
fn config_local_section_defaults() {
    let toml = r#"
[service]
vps_host = "minibox"
"#;
    let cfg: SearchboxConfig = toml::from_str(toml).unwrap();
    assert!(!cfg.local.enabled);
    assert_eq!(cfg.local.port, 6071);
    assert!(cfg.local.repos.is_empty());
}

fn make_result(repo: &str, file: &str, line: u32, score: f32) -> searchbox::domain::SearchResult {
    searchbox::domain::SearchResult {
        repo: repo.into(),
        file: file.into(),
        line,
        col: 0,
        snippet: "snippet".into(),
        score,
        commit: None,
    }
}

#[tokio::test]
async fn merged_deduplicates_same_repo_file_line() {
    let r1 = make_result("repo", "src/lib.rs", 42, 1.0);
    let r2 = make_result("repo", "src/lib.rs", 42, 0.9); // duplicate
    let r3 = make_result("repo", "src/lib.rs", 99, 0.5); // distinct

    let p1 = MockSearchProvider::with_results(vec![r1]);
    let p2 = MockSearchProvider::with_results(vec![r2, r3]);

    let merged = MergedAdapter::new(vec![Box::new(p1), Box::new(p2)]);
    let results = merged
        .search(searchbox::domain::SearchQuery::new("foo"))
        .await
        .unwrap();

    assert_eq!(
        results.len(),
        2,
        "expected 2 unique results, got {}",
        results.len()
    );
}

#[tokio::test]
async fn merged_sorts_by_score_descending() {
    let results = vec![
        make_result("r", "a", 1, 0.3),
        make_result("r", "b", 2, 0.9),
        make_result("r", "c", 3, 0.6),
    ];
    let p = MockSearchProvider::with_results(results);
    let merged = MergedAdapter::new(vec![Box::new(p)]);
    let out = merged
        .search(searchbox::domain::SearchQuery::new("x"))
        .await
        .unwrap();
    assert_eq!(out[0].score, 0.9);
    assert_eq!(out[1].score, 0.6);
    assert_eq!(out[2].score, 0.3);
}

#[tokio::test]
async fn merged_continues_on_provider_failure() {
    let good = MockSearchProvider::with_results(vec![make_result("r", "f", 1, 1.0)]);
    let bad = MockSearchProvider::failing();
    let merged = MergedAdapter::new(vec![Box::new(bad), Box::new(good)]);
    let out = merged
        .search(searchbox::domain::SearchQuery::new("x"))
        .await
        .unwrap();
    assert_eq!(out.len(), 1);
}
