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
