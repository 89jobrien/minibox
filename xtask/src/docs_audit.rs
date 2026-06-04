//! Docs audit — verify `docs/core/` facts against code and detect staleness.
//!
//! Two modes:
//! - `--quick`: extract `<!-- fact:key=value -->` markers from docs, compare
//!   against code truth. Fast, suitable for CI / `cargo xtask verify`.
//! - `--full`: quick checks + git-based freshness analysis + coverage gap
//!   detection + agentlint. Produces a JSON report.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use xshell::{Shell, cmd};

// ── Types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum Mode {
    Quick { strict: bool },
    Full,
}

#[derive(Debug, Serialize)]
struct AuditReport {
    timestamp: String,
    quick_checks: Vec<FactResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    freshness: Vec<FreshnessResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    coverage_gaps: Vec<CoverageGap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agentlint: Option<AgentlintResult>,
}

#[derive(Debug, Serialize)]
struct FactResult {
    check: String,
    status: String,
    code_value: String,
    doc_value: String,
    file: String,
    line: usize,
}

#[derive(Debug, Serialize)]
struct FreshnessResult {
    doc: String,
    last_modified: String,
    code_commits_since: usize,
    status: String,
}

#[derive(Debug, Serialize)]
struct CoverageGap {
    entity: String,
    entity_type: String,
    missing_from: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AgentlintResult {
    exit_code: i32,
    stdout: String,
}

// ── Fact extraction from code ────────────────────────────────────────────

fn code_facts(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut facts = BTreeMap::new();

    // crate_count: dirs under crates/ with Cargo.toml (excluding xtask)
    let crates_dir = root.join("crates");
    let mut crate_count = 0u32;
    let mut crate_names = BTreeSet::new();
    if crates_dir.is_dir() {
        for entry in std::fs::read_dir(&crates_dir)? {
            let entry = entry?;
            if entry.path().join("Cargo.toml").exists() {
                crate_count += 1;
                if let Some(name) = entry.file_name().to_str() {
                    crate_names.insert(name.to_string());
                }
            }
        }
    }
    facts.insert("crate_count".into(), crate_count.to_string());

    // workspace_version
    let root_toml =
        std::fs::read_to_string(root.join("Cargo.toml")).context("read root Cargo.toml")?;
    for line in root_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("version")
            && trimmed.contains('=')
            && let Some(v) = trimmed.split('=').nth(1)
        {
            let v = v.trim().trim_matches('"');
            facts.insert("workspace_version".into(), v.to_string());
            break;
        }
    }

    // adapter_suites: parse AdapterSuite enum variants
    let registry_path = crates_dir.join("miniboxd/src/adapter_registry.rs");
    if registry_path.exists() {
        let content =
            std::fs::read_to_string(&registry_path).context("read adapter_registry.rs")?;
        let suites = parse_enum_variants(&content, "AdapterSuite");
        if !suites.is_empty() {
            // Convert PascalCase to lowercase for comparison
            let lowercase: Vec<String> = suites.iter().map(|s| s.to_lowercase()).collect();
            let mut sorted = lowercase;
            sorted.sort();
            facts.insert("adapter_suites".into(), sorted.join(","));
        }
        facts.insert(
            "adapter_suite_count".into(),
            parse_enum_variants(&content, "AdapterSuite")
                .len()
                .to_string(),
        );
    }

    // protocol variant counts
    let protocol_path = crates_dir.join("minibox-core/src/protocol.rs");
    if protocol_path.exists() {
        let content = std::fs::read_to_string(&protocol_path).context("read protocol.rs")?;
        let req = parse_enum_variants(&content, "DaemonRequest");
        let resp = parse_enum_variants(&content, "DaemonResponse");
        facts.insert("protocol_request_variants".into(), req.len().to_string());
        facts.insert("protocol_response_variants".into(), resp.len().to_string());
    }

    // domain_trait_count
    let domain_path = crates_dir.join("minibox-core/src/domain.rs");
    if domain_path.exists() {
        let content = std::fs::read_to_string(&domain_path).context("read domain.rs")?;
        let count = content
            .lines()
            .filter(|l| {
                let t = l.trim();
                t.starts_with("pub trait ") || t.starts_with("pub(crate) trait ")
            })
            .count();
        // Also check domain/ submodules
        let domain_dir = crates_dir.join("minibox-core/src/domain");
        let sub_count = if domain_dir.is_dir() {
            count_traits_in_dir(&domain_dir)?
        } else {
            0
        };
        facts.insert("domain_trait_count".into(), (count + sub_count).to_string());
    }

    Ok(facts)
}

/// Parse top-level enum variant names from Rust source.
///
/// Handles both unit variants (`Foo,`) and struct/tuple variants
/// (`Foo { ... },` / `Foo(...)`) by looking for lines that start with
/// an uppercase letter after trimming, before hitting the closing `}`.
fn parse_enum_variants(src: &str, enum_name: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let pattern = format!("enum {enum_name}");
    let mut inside = false;
    let mut brace_depth: i32 = 0;

    for line in src.lines() {
        let trimmed = line.trim();

        if !inside {
            if trimmed.contains(&pattern) && trimmed.contains('{') {
                inside = true;
                brace_depth = 1;
            }
            continue;
        }

        // Track brace depth to skip struct variant bodies
        for ch in trimmed.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                _ => {}
            }
        }

        if brace_depth <= 0 {
            break;
        }

        // Only look at top-level variants (depth == 1)
        if brace_depth == 1 {
            // Skip doc comments and attributes
            if trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            // A variant line starts with an uppercase letter
            if let Some(first) = trimmed.chars().next()
                && first.is_uppercase()
            {
                // Extract name (up to first non-ident char)
                let name: String = trimmed
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    variants.push(name);
                }
            }
        }
    }

    variants
}

fn count_traits_in_dir(dir: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let content = std::fs::read_to_string(&path)?;
            count += content
                .lines()
                .filter(|l| {
                    let t = l.trim();
                    t.starts_with("pub trait ") || t.starts_with("pub(crate) trait ")
                })
                .count();
        }
    }
    Ok(count)
}

// ── Fact extraction from docs ────────────────────────────────────────────

#[derive(Debug)]
struct DocFact {
    file: String,
    line: usize,
    key: String,
    value: String,
}

fn doc_facts(root: &Path) -> Result<Vec<DocFact>> {
    let docs_dir = root.join("docs/core");
    let mut facts = Vec::new();

    if !docs_dir.is_dir() {
        return Ok(facts);
    }

    for entry in std::fs::read_dir(&docs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        let content = std::fs::read_to_string(&path)?;

        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("<!-- fact:")
                && let Some(kv) = rest.strip_suffix(" -->")
                && let Some((k, v)) = kv.split_once('=')
            {
                facts.push(DocFact {
                    file: filename.clone(),
                    line: i + 1,
                    key: k.to_string(),
                    value: v.to_string(),
                });
            }
        }
    }

    Ok(facts)
}

// ── Compare ──────────────────────────────────────────────────────────────

fn compare_facts(code: &BTreeMap<String, String>, docs: &[DocFact]) -> Vec<FactResult> {
    let mut results = Vec::new();

    for df in docs {
        let code_val = code.get(&df.key).cloned().unwrap_or_default();
        let status = if code_val == df.value {
            "pass"
        } else {
            "MISMATCH"
        };
        results.push(FactResult {
            check: df.key.clone(),
            status: status.to_string(),
            code_value: code_val,
            doc_value: df.value.clone(),
            file: df.file.clone(),
            line: df.line,
        });
    }

    results
}

// ── Freshness (full mode only) ───────────────────────────────────────────

fn check_freshness(sh: &Shell, root: &Path) -> Result<Vec<FreshnessResult>> {
    let docs_dir = root.join("docs/core");
    let mut results = Vec::new();

    if !docs_dir.is_dir() {
        return Ok(results);
    }

    for entry in std::fs::read_dir(&docs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();

        // Read watches from YAML frontmatter
        let content = std::fs::read_to_string(&path)?;
        let watches = parse_watches(&content);
        if watches.is_empty() {
            continue;
        }

        // Get doc last-modified date
        let doc_rel = format!("docs/core/{filename}");
        let last_modified = cmd!(sh, "git log -1 --format=%aI -- {doc_rel}")
            .read()
            .unwrap_or_default();

        if last_modified.is_empty() {
            continue;
        }

        // Count commits touching watched paths since doc was modified
        let date_arg = format!("--since={last_modified}");
        let mut total_commits = 0usize;
        for watch_glob in &watches {
            let count_str = cmd!(sh, "git log --oneline {date_arg} -- {watch_glob}")
                .read()
                .unwrap_or_default();
            total_commits += count_str.lines().filter(|l| !l.is_empty()).count();
        }

        let status = if total_commits > 15 {
            "stale"
        } else if total_commits > 5 {
            "aging"
        } else {
            "fresh"
        };

        // Truncate ISO date to YYYY-MM-DD
        let date_short = last_modified
            .split('T')
            .next()
            .unwrap_or(&last_modified)
            .to_string();

        results.push(FreshnessResult {
            doc: filename,
            last_modified: date_short,
            code_commits_since: total_commits,
            status: status.to_string(),
        });
    }

    Ok(results)
}

/// Parse `watches:` list from YAML frontmatter.
fn parse_watches(content: &str) -> Vec<String> {
    let mut watches = Vec::new();
    let mut in_frontmatter = false;
    let mut in_watches = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                break; // End of frontmatter
            }
            in_frontmatter = true;
            continue;
        }
        if !in_frontmatter {
            continue;
        }

        if trimmed.starts_with("watches:") {
            in_watches = true;
            continue;
        }

        if in_watches {
            if trimmed.starts_with("- ") {
                let val = trimmed.strip_prefix("- ").unwrap().trim();
                watches.push(val.to_string());
            } else if !trimmed.is_empty() {
                // Another YAML key — done with watches
                break;
            }
        }
    }

    watches
}

// ── Coverage gaps (full mode only) ───────────────────────────────────────

fn check_coverage(root: &Path) -> Result<Vec<CoverageGap>> {
    let mut gaps = Vec::new();

    // Collect workspace crate names
    let crates_dir = root.join("crates");
    let mut crate_names = BTreeSet::new();
    if crates_dir.is_dir() {
        for entry in std::fs::read_dir(&crates_dir)? {
            let entry = entry?;
            if entry.path().join("Cargo.toml").exists()
                && let Some(name) = entry.file_name().to_str()
            {
                crate_names.insert(name.to_string());
            }
        }
    }

    // Check CRATE_INVENTORY mentions
    let inv_path = root.join("docs/core/CRATE_INVENTORY.mbx.md");
    let arch_path = root.join("docs/core/ARCHITECTURE.mbx.md");

    let inv_content = std::fs::read_to_string(&inv_path).unwrap_or_default();
    let arch_content = std::fs::read_to_string(&arch_path).unwrap_or_default();

    for name in &crate_names {
        let mut missing = Vec::new();
        if !inv_content.contains(name.as_str()) {
            missing.push("CRATE_INVENTORY.mbx.md".to_string());
        }
        if !arch_content.contains(name.as_str()) {
            missing.push("ARCHITECTURE.mbx.md".to_string());
        }
        if !missing.is_empty() {
            gaps.push(CoverageGap {
                entity: name.clone(),
                entity_type: "crate".to_string(),
                missing_from: missing,
            });
        }
    }

    // Check adapter suites mentioned in FEATURE_MATRIX
    let fm_path = root.join("docs/core/FEATURE_MATRIX.mbx.md");
    let fm_content = std::fs::read_to_string(&fm_path).unwrap_or_default();

    let registry_path = crates_dir.join("miniboxd/src/adapter_registry.rs");
    if registry_path.exists() {
        let content = std::fs::read_to_string(&registry_path)?;
        let suites = parse_enum_variants(&content, "AdapterSuite");
        for suite in &suites {
            let lower = suite.to_lowercase();
            if !fm_content.to_lowercase().contains(&lower) {
                gaps.push(CoverageGap {
                    entity: lower,
                    entity_type: "adapter_suite".to_string(),
                    missing_from: vec!["FEATURE_MATRIX.mbx.md".to_string()],
                });
            }
        }
    }

    Ok(gaps)
}

// ── Agentlint (both modes) ──────────────────────────────────────────────

fn run_agentlint(sh: &Shell, root: &Path, json: bool) -> Result<AgentlintResult> {
    let docs_dir = root.join("docs/core");
    let docs_str = docs_dir.to_string_lossy().to_string();
    let format_flag = if json { "json" } else { "gnu" };

    let output = cmd!(
        sh,
        "agentlint {docs_str} --difficulty painful --format {format_flag} --exit-zero"
    )
    .output()
    .context("run agentlint")?;

    Ok(AgentlintResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
    })
}

// ── Public entry point ───────────────────────────────────────────────────

pub fn run(sh: &Shell, root: &Path, mode: Mode) -> Result<()> {
    eprintln!("--- docs-audit ---");

    let code = code_facts(root)?;
    let docs = doc_facts(root)?;

    if docs.is_empty() {
        eprintln!(
            "docs-audit: no <!-- fact:key=value --> markers found in docs/core/. \
             Add markers to enable checking."
        );
        // Still run agentlint and coverage in full mode
    }

    let quick_checks = compare_facts(&code, &docs);

    // Print quick results
    let mut mismatches = 0usize;
    for r in &quick_checks {
        if r.status == "MISMATCH" {
            mismatches += 1;
            eprintln!(
                "docs-audit: MISMATCH {} -- code={} doc={} ({}:{})",
                r.check, r.code_value, r.doc_value, r.file, r.line
            );
        }
    }

    // Run agentlint in both modes (it's fast)
    let agentlint = run_agentlint(sh, root, matches!(mode, Mode::Full)).ok();

    match mode {
        Mode::Quick { strict } => {
            let pass_count = quick_checks.len() - mismatches;
            eprintln!(
                "docs-audit (quick): {} checks, {} passed, {} mismatches",
                quick_checks.len(),
                pass_count,
                mismatches
            );
            if let Some(ref al) = agentlint
                && !al.stdout.trim().is_empty()
            {
                eprintln!("agentlint: {}", al.stdout.trim());
            }
            if strict && mismatches > 0 {
                bail!("docs-audit: {mismatches} fact mismatch(es) in strict mode");
            }
        }
        Mode::Full => {
            eprintln!("--- docs-audit: freshness ---");
            let freshness = check_freshness(sh, root)?;
            for f in &freshness {
                if f.status != "fresh" {
                    eprintln!(
                        "docs-audit: {} {} (last modified {}, {} commits since)",
                        f.status.to_uppercase(),
                        f.doc,
                        f.last_modified,
                        f.code_commits_since
                    );
                }
            }

            eprintln!("--- docs-audit: coverage ---");
            let coverage_gaps = check_coverage(root)?;
            for g in &coverage_gaps {
                eprintln!(
                    "docs-audit: GAP {} ({}) not in {}",
                    g.entity,
                    g.entity_type,
                    g.missing_from.join(", ")
                );
            }

            let stale_count = freshness.iter().filter(|f| f.status == "stale").count();
            let aging_count = freshness.iter().filter(|f| f.status == "aging").count();
            eprintln!(
                "docs-audit (full): {} mismatches, {} stale, {} aging, {} gaps",
                mismatches,
                stale_count,
                aging_count,
                coverage_gaps.len()
            );

            // Write JSON report
            let report = AuditReport {
                timestamp: chrono::Utc::now().to_rfc3339(),
                quick_checks,
                freshness,
                coverage_gaps,
                agentlint,
            };

            let report_path = root.join("xtask/docs-audit-report.json");
            let json = serde_json::to_string_pretty(&report).context("serialize report")?;
            std::fs::write(&report_path, &json).context("write docs-audit-report.json")?;
            eprintln!("docs-audit: report written to {}", report_path.display());
        }
    }

    Ok(())
}
