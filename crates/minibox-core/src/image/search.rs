//! Deterministic in-memory search over cached image references.
//!
//! The index is rebuilt from the image store on each query. Local stores are
//! intentionally small, so this avoids persistent index state and invalidation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a search result was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchSource {
    /// The daemon image store.
    Local,
    /// A remote registry.
    Registry,
}

/// Repository-level image search result with all matching local tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSearchResult {
    /// Repository name, such as `library/alpine`.
    pub name: String,
    /// Sorted, deduplicated tags.
    pub tags: Vec<String>,
    /// Discovery source.
    pub source: SearchSource,
}

/// Search cached `name:tag` references and aggregate matches by repository.
///
/// Matching is ASCII case-insensitive. Exact repository or leaf-name matches
/// rank first, followed by leaf prefixes, repository prefixes, repository
/// substrings, exact tags, and tag substrings. Ties are stable by repository
/// depth and lexical name.
#[must_use]
pub fn search_image_refs<I, S>(image_refs: I, query: &str, limit: usize) -> Vec<ImageSearchResult>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if limit == 0 {
        return Vec::new();
    }

    let query = query.trim().to_ascii_lowercase();
    let mut repositories: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for image_ref in image_refs {
        let image_ref = image_ref.as_ref();
        let Some((name, tag)) = image_ref.rsplit_once(':') else {
            continue;
        };
        repositories
            .entry(name.to_owned())
            .or_default()
            .push(tag.to_owned());
    }

    let mut ranked = repositories
        .into_iter()
        .filter_map(|(name, mut tags)| {
            tags.sort();
            tags.dedup();
            rank_match(&name, &tags, &query).map(|rank| {
                (
                    rank,
                    if query.is_empty() {
                        0
                    } else {
                        name.matches('/').count()
                    },
                    name.to_ascii_lowercase(),
                    ImageSearchResult {
                        name,
                        tags,
                        source: SearchSource::Local,
                    },
                )
            })
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|left, right| (&left.0, &left.1, &left.2).cmp(&(&right.0, &right.1, &right.2)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, _, _, result)| result)
        .collect()
}

fn rank_match(name: &str, tags: &[String], query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }

    let name = name.to_ascii_lowercase();
    let leaf = name.rsplit('/').next().unwrap_or(&name);
    if name == query || leaf == query {
        Some(0)
    } else if leaf.starts_with(query) {
        Some(1)
    } else if name.starts_with(query) {
        Some(2)
    } else if name.contains(query) {
        Some(3)
    } else if tags.iter().any(|tag| tag.eq_ignore_ascii_case(query)) {
        Some(4)
    } else if tags
        .iter()
        .any(|tag| tag.to_ascii_lowercase().contains(query))
    {
        Some(5)
    } else {
        None
    }
}
