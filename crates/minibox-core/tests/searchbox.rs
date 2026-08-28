//! Searchbox ranking and filtering contract tests.
use minibox_core::image::search::{ImageSearchResult, SearchSource, search_image_refs};

fn refs() -> Vec<String> {
    vec![
        "library/alpine:3.19".into(),
        "library/nginx:latest".into(),
        "team/alpine-tools:latest".into(),
        "library/alpine:latest".into(),
        "ghcr.io/acme/alpine-agent:v1".into(),
    ]
}

#[test]
fn exact_repository_match_ranks_before_prefix_and_substring_matches() {
    let results = search_image_refs(refs(), "alpine", 20);

    assert_eq!(results[0].name, "library/alpine");
    assert_eq!(results[1].name, "team/alpine-tools");
    assert_eq!(results[2].name, "ghcr.io/acme/alpine-agent");
}

#[test]
fn groups_and_sorts_tags_deterministically() {
    let results = search_image_refs(refs(), "library/alpine", 20);

    assert_eq!(
        results,
        vec![ImageSearchResult {
            name: "library/alpine".into(),
            tags: vec!["3.19".into(), "latest".into()],
            source: SearchSource::Local,
        }]
    );
}

#[test]
fn tag_matches_are_case_insensitive_and_filtered() {
    let results = search_image_refs(refs(), "V1", 20);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "ghcr.io/acme/alpine-agent");
    assert_eq!(results[0].tags, vec!["v1"]);
}

#[test]
fn empty_query_lists_all_repositories_with_stable_limit() {
    let first = search_image_refs(refs(), "", 2);
    let second = search_image_refs(refs().into_iter().rev(), "", 2);

    assert_eq!(first, second);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].name, "ghcr.io/acme/alpine-agent");
    assert_eq!(first[1].name, "library/alpine");
}

#[test]
fn zero_limit_returns_no_results() {
    assert!(search_image_refs(refs(), "alpine", 0).is_empty());
}
