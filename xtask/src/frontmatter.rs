#![allow(dead_code)]

//! Line-oriented frontmatter parsing for the doc provenance contract.
//!
//! xtask has no YAML dependency, so this parses only the strict subset the
//! docs actually use: `key: value`, `  - list item`, and one level of
//! `  key: value` under a mapping key. Anything else is left unparsed.

/// A violation of the doc provenance contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractViolation {
    /// The deleted `source_sha` scalar has returned.
    LegacySourceSha,
    /// A `regenerated_from` key is not declared in `sources`.
    UndeclaredRegeneratedSource { key: String },
    /// A `regenerated_from` value is not a 40-character hex SHA.
    MalformedRegeneratedValue { key: String, value: String },
}

/// Extract the text between the opening and closing `---` delimiters.
pub fn frontmatter_block(content: &str) -> Option<&str> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// Look up a top-level scalar value for `key`.
///
/// Exact key match: `sources_extra:` does not satisfy a lookup of `sources`.
pub fn frontmatter_scalar<'a>(fm: &'a str, key: &str) -> Option<&'a str> {
    for line in fm.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(val) = rest.strip_prefix(':') {
                let val = val.trim();
                if !val.is_empty() {
                    let val = val.trim_matches('"').trim_matches('\'');
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
            }
        }
    }
    None
}

/// Collect the list items declared under a top-level `key:`.
pub fn frontmatter_list(fm: &str, key: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut in_list = false;
    for line in fm.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_top_level = !line.starts_with(' ') && !line.starts_with('\t');
        if is_top_level {
            if let Some(rest) = trimmed.strip_prefix(key) {
                let rest = rest.trim_start();
                if rest.is_empty() || rest == ":" {
                    in_list = true;
                    continue;
                }
            }
            in_list = false;
            continue;
        }
        if in_list {
            if let Some(item) = trimmed.strip_prefix("- ") {
                let item = item.trim().trim_matches('"').trim_matches('\'');
                if !item.is_empty() {
                    result.push(item.to_string());
                }
            } else if trimmed == "-" {
                // empty item, ignore
            } else if trimmed.contains(':') {
                in_list = false;
            }
        }
    }
    result
}

/// Collect the one-level mapping declared under a top-level `key:`.
pub fn frontmatter_mapping(fm: &str, key: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut in_map = false;
    for line in fm.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_top_level = !line.starts_with(' ') && !line.starts_with('\t');
        if is_top_level {
            if let Some(rest) = trimmed.strip_prefix(key) {
                let rest = rest.trim_start();
                if rest.is_empty() || rest == ":" {
                    in_map = true;
                    continue;
                }
            }
            in_map = false;
            continue;
        }
        if in_map {
            if trimmed.starts_with("- ") {
                in_map = false;
                continue;
            }
            if let Some((k, v)) = trimmed.split_once(':') {
                let k = k.trim();
                let v = v.trim().trim_matches('"').trim_matches('\'');
                if !k.is_empty() {
                    result.push((k.to_string(), v.to_string()));
                }
            }
        }
    }
    result
}

fn is_valid_sha(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Validate the doc provenance contract.
pub fn validate_provenance_contract(fm: &str, _rel: &str) -> Vec<ContractViolation> {
    let mut violations = Vec::new();

    // Legacy scalar must not return.
    for line in fm.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("source_sha") {
            let rest = rest.trim_start();
            if rest.starts_with(':') {
                violations.push(ContractViolation::LegacySourceSha);
                break;
            }
        }
    }

    let sources: Vec<String> = frontmatter_list(fm, "sources");

    let has_regenerated = fm.lines().any(|l| {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("regenerated_from") {
            let rest = rest.trim_start();
            rest.is_empty() || rest.starts_with(':')
        } else {
            false
        }
    });

    let mapping = frontmatter_mapping(fm, "regenerated_from");
    if has_regenerated && mapping.is_empty() {
        violations.push(ContractViolation::MalformedRegeneratedValue {
            key: String::new(),
            value: String::new(),
        });
    }

    for (key, value) in mapping {
        if !sources.contains(&key) {
            violations.push(ContractViolation::UndeclaredRegeneratedSource { key: key.clone() });
        }
        if !is_valid_sha(&value) {
            violations.push(ContractViolation::MalformedRegeneratedValue {
                key: key.clone(),
                value: value.clone(),
            });
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_block_extracts() {
        let content = "---\nstatus: done\n---\n# Title\n";
        let fm = frontmatter_block(content).unwrap();
        assert!(fm.contains("status: done"));
    }

    #[test]
    fn scalar_exact_key() {
        let fm = "sources:\nsources_extra: x\n";
        assert_eq!(frontmatter_scalar(fm, "sources"), None);
        let fm2 = "sources: foo\nsources_extra: bar\n";
        assert_eq!(frontmatter_scalar(fm2, "sources"), Some("foo"));
        assert_eq!(frontmatter_scalar(fm2, "sources_extra"), Some("bar"));
    }

    #[test]
    fn list_and_mapping() {
        let fm = "sources:\n  - a\n  - b\nregenerated_from:\n  a: 1111111111111111111111111111111111111111\n";
        assert_eq!(frontmatter_list(fm, "sources"), vec!["a", "b"]);
        let m = frontmatter_mapping(fm, "regenerated_from");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].0, "a");
    }
}
