//! Lightweight Docker ignore matching for native build contexts.

use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, Clone)]
struct Rule {
    pattern: String,
    negated: bool,
    directory_only: bool,
    anchored: bool,
}

/// Ordered `.dockerignore` rules using Docker-style last-match precedence.
#[derive(Debug, Clone, Default)]
pub(super) struct DockerIgnore {
    rules: Vec<Rule>,
}

impl DockerIgnore {
    pub(super) fn load(context: &Path) -> Result<Self> {
        let path = context.join(".dockerignore");
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        Ok(Self::parse(&contents))
    }

    fn parse(contents: &str) -> Self {
        let rules = contents
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                let (negated, line) = line
                    .strip_prefix('!')
                    .map_or((false, line), |pattern| (true, pattern));
                if line.is_empty() {
                    return None;
                }
                let directory_only = line.ends_with('/');
                let anchored = line.starts_with('/');
                let pattern = line
                    .trim_start_matches('/')
                    .trim_end_matches('/')
                    .replace('\\', "/");
                Some(Rule {
                    pattern,
                    negated,
                    directory_only,
                    anchored,
                })
            })
            .collect();
        Self { rules }
    }

    pub(super) fn is_ignored(&self, relative: &Path, is_directory: bool) -> bool {
        let path = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let mut ignored = false;
        for rule in &self.rules {
            if rule.matches(&path, is_directory) {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

impl Rule {
    fn matches(&self, path: &str, is_directory: bool) -> bool {
        if path.is_empty() {
            return false;
        }
        let parts: Vec<&str> = path.split('/').collect();
        for length in 1..=parts.len() {
            let candidate = parts[..length].join("/");
            let candidate_is_directory = length < parts.len() || is_directory;
            if self.directory_only && !candidate_is_directory {
                continue;
            }
            if self.matches_candidate(&candidate) {
                return true;
            }
        }
        false
    }

    fn matches_candidate(&self, candidate: &str) -> bool {
        if self.anchored {
            return glob_matches(&self.pattern, candidate);
        }
        if self.pattern.contains('/') {
            if glob_matches(&self.pattern, candidate) {
                return true;
            }
            return candidate
                .match_indices('/')
                .any(|(index, _)| glob_matches(&self.pattern, &candidate[index + 1..]));
        }
        candidate
            .split('/')
            .any(|component| glob_matches(&self.pattern, component))
    }
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut memo = vec![vec![None; value.len() + 1]; pattern.len() + 1];
    glob_matches_at(pattern, value, 0, 0, &mut memo)
}

fn glob_matches_at(
    pattern: &[u8],
    value: &[u8],
    pattern_index: usize,
    value_index: usize,
    memo: &mut [Vec<Option<bool>>],
) -> bool {
    if let Some(result) = memo[pattern_index][value_index] {
        return result;
    }
    let result = if pattern_index == pattern.len() {
        value_index == value.len()
    } else if pattern[pattern_index] == b'*' {
        let recursive = pattern.get(pattern_index + 1) == Some(&b'*');
        let next_pattern = pattern_index + usize::from(recursive) + 1;
        let zero_directories = recursive
            && pattern.get(next_pattern) == Some(&b'/')
            && glob_matches_at(pattern, value, next_pattern + 1, value_index, memo);
        zero_directories
            || glob_matches_at(pattern, value, next_pattern, value_index, memo)
            || (value_index < value.len()
                && (recursive || value[value_index] != b'/')
                && glob_matches_at(pattern, value, pattern_index, value_index + 1, memo))
    } else if value_index < value.len()
        && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
    {
        glob_matches_at(pattern, value, pattern_index + 1, value_index + 1, memo)
    } else {
        false
    };
    memo[pattern_index][value_index] = Some(result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_blanks_negation_and_directory_patterns_follow_order() {
        let rules = DockerIgnore::parse("# comment\n\ntarget/\n*.txt\n!keep.txt\n");
        assert!(rules.is_ignored(Path::new("target/debug/app"), false));
        assert!(rules.is_ignored(Path::new("nested/drop.txt"), false));
        assert!(!rules.is_ignored(Path::new("nested/keep.txt"), false));
    }

    #[test]
    fn anchored_and_unanchored_patterns_differ() {
        let rules = DockerIgnore::parse("/root-only\ncache/data\n**/secret\n");
        assert!(rules.is_ignored(Path::new("root-only"), false));
        assert!(!rules.is_ignored(Path::new("nested/root-only"), false));
        assert!(rules.is_ignored(Path::new("nested/cache/data/file"), false));
        assert!(rules.is_ignored(Path::new("secret"), false));
        assert!(rules.is_ignored(Path::new("nested/secret"), false));
    }
}
