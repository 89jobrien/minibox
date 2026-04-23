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
