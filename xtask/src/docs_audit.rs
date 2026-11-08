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
    freshness: Vec<StaleDoc>,
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
    if crates_dir.is_dir() {
        for entry in std::fs::read_dir(&crates_dir)? {
            let entry = entry?;
            if entry.path().join("Cargo.toml").exists() {
                crate_count += 1;
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
    let registry_path = crate::utils::adapter_registry_path();
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

// ── Staleness: derived-from-git, hard-failing ────────────────────────────

/// Paths exempt from staleness collection: point-in-time decision records,
/// not living reference docs expected to track current code.
pub const FRESHNESS_EXEMPT_PREFIXES: [&str; 2] = ["docs/designs/", "docs/plans/"];

/// Cap on how many commits `substantive_last_change` walks before giving up
/// and reporting [`StaleReason::IndeterminateHistory`] rather than guessing.
pub const MAX_SUBSTANTIVE_WALK: usize = 50;

/// True if `rel` falls under a [`FRESHNESS_EXEMPT_PREFIXES`] prefix.
pub fn is_exempt_path(rel: &str) -> bool {
    FRESHNESS_EXEMPT_PREFIXES
        .iter()
        .any(|prefix| rel.starts_with(prefix))
}

/// Git queries the staleness rule depends on, injectable so the
/// substantive-change walk is testable without a repository on disk.
pub trait GitHistory {
    fn is_shallow_repository(&self) -> Result<bool>;
    fn last_change(&self, path: &str) -> Result<Option<String>>;
    fn commit_subject(&self, sha: &str) -> Result<String>;
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool>;
    fn commits_touching(&self, path: &str) -> Result<Vec<String>>;
    fn commit_changed_lines(&self, sha: &str, path: &str) -> Result<Vec<String>>;
}

/// [`GitHistory`] backed by real `git` invocations via `xshell`.
pub struct ShellGitHistory {
    pub sh: Shell,
}

impl GitHistory for ShellGitHistory {
    fn is_shallow_repository(&self) -> Result<bool> {
        let out = cmd!(self.sh, "git rev-parse --is-shallow-repository")
            .read()
            .context("git rev-parse --is-shallow-repository failed")?;
        Ok(out.trim() == "true")
    }

    fn last_change(&self, path: &str) -> Result<Option<String>> {
        let out = cmd!(self.sh, "git log -1 --format=%H -- {path}")
            .read()
            .unwrap_or_default();
        let sha = out.trim();
        Ok(if sha.is_empty() {
            None
        } else {
            Some(sha.to_string())
        })
    }

    fn commit_subject(&self, sha: &str) -> Result<String> {
        let out = cmd!(self.sh, "git log -1 --format=%s {sha}")
            .read()
            .with_context(|| format!("git log failed for {sha}"))?;
        Ok(out.trim().to_string())
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        let output = cmd!(
            self.sh,
            "git merge-base --is-ancestor {ancestor} {descendant}"
        )
        .ignore_status()
        .output()
        .with_context(|| format!("git merge-base --is-ancestor {ancestor} {descendant}"))?;
        Ok(output.status.success())
    }

    fn commits_touching(&self, path: &str) -> Result<Vec<String>> {
        let out = cmd!(self.sh, "git log --format=%H -- {path}")
            .read()
            .unwrap_or_default();
        Ok(out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn commit_changed_lines(&self, sha: &str, path: &str) -> Result<Vec<String>> {
        let out = cmd!(self.sh, "git show {sha} -U0 -- {path}")
            .read()
            .with_context(|| format!("git show {sha} -- {path} failed"))?;
        let mut lines = Vec::new();
        for line in out.lines() {
            let Some(body) = line.strip_prefix('+').or_else(|| line.strip_prefix('-')) else {
                continue;
            };
            // A leading "++"/"--" is a file header, not content.
            if body.starts_with('+') || body.starts_with('-') {
                continue;
            }
            lines.push(body.to_string());
        }
        Ok(lines)
    }
}

/// Why a doc was reported stale.
#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StaleReason {
    /// A declared source has a commit after the doc's last substantive change.
    SourceChangedAfterDoc,
    /// A declared source path does not exist in the working tree.
    SourcePathMissing,
    /// A declared source path exists but has no commits.
    SourceUntracked,
    /// The doc's history is exhausted by stamp-only commits, or the walk
    /// exceeded [`MAX_SUBSTANTIVE_WALK`] before a substantive commit was found.
    IndeterminateHistory,
}

#[derive(Debug, Serialize)]
pub struct StaleSource {
    pub path: String,
    pub last_change: String,
    pub last_change_subject: String,
}

#[derive(Debug, Serialize)]
pub struct StaleDoc {
    pub doc: String,
    pub reason: StaleReason,
    pub sources: Vec<StaleSource>,
}

#[derive(Debug, Serialize)]
pub struct StalenessReport {
    pub checked_docs: usize,
    pub exempt_docs: usize,
    pub untracked_docs: usize,
    pub stale: Vec<StaleDoc>,
}

/// Walk `doc_rel`'s commit history newest-first, skipping commits whose
/// changed lines are entirely stamp lines (see
/// [`crate::feature_matrix_date::is_stamp_line`]), capped at
/// [`MAX_SUBSTANTIVE_WALK`]. Returns `None` if the walk is exhausted or
/// capped before a substantive commit is found.
fn substantive_last_change(history: &dyn GitHistory, doc_rel: &str) -> Result<Option<String>> {
    let commits = history.commits_touching(doc_rel)?;
    for sha in commits.iter().take(MAX_SUBSTANTIVE_WALK) {
        let changed = history.commit_changed_lines(sha, doc_rel)?;
        if !changed.is_empty()
            && changed
                .iter()
                .any(|l| !crate::feature_matrix_date::is_stamp_line(l))
        {
            return Ok(Some(sha.clone()));
        }
    }
    Ok(None)
}

/// Recursively collect every `.md` file under `dir`.
fn collect_markdown_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// Derived-from-git staleness: a doc is stale when a declared source has
/// changed (per [`substantive_last_change`]) after the doc's own last
/// substantive change.
pub fn check_staleness(sh: &Shell, root: &Path) -> Result<StalenessReport> {
    let history = ShellGitHistory { sh: sh.clone() };
    check_staleness_with(&history, root)
}

fn check_staleness_with(history: &dyn GitHistory, root: &Path) -> Result<StalenessReport> {
    if history.is_shallow_repository()? {
        bail!(
            "docs-audit: repository is a shallow clone; staleness requires full history \
             (fetch-depth: 0)"
        );
    }

    let docs_dir = root.join("docs");
    let mut md_files = Vec::new();
    if docs_dir.is_dir() {
        collect_markdown_files(&docs_dir, &mut md_files)?;
    }

    let mut checked_docs = 0usize;
    let mut exempt_docs = 0usize;
    let mut untracked_docs = 0usize;
    let mut stale = Vec::new();

    for path in md_files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let content = std::fs::read_to_string(&path)?;
        let Some(fm) = crate::frontmatter::frontmatter_block(&content) else {
            continue;
        };

        // Shape (provenance contract) is checked regardless of exemption:
        // a design/plan doc may be exempt from staleness, but it still must
        // not carry the deleted source_sha scalar or a malformed
        // regenerated_from entry.
        let violations = crate::frontmatter::validate_provenance_contract(fm, &rel);
        if !violations.is_empty() {
            bail!("docs-audit: provenance contract violation(s) in {rel}: {violations:?}");
        }

        if is_exempt_path(&rel) {
            exempt_docs += 1;
            continue;
        }

        let sources = crate::frontmatter::frontmatter_list(fm, "sources");
        if sources.is_empty() {
            continue;
        }

        if history.commits_touching(&rel)?.is_empty() {
            untracked_docs += 1;
            continue;
        }

        checked_docs += 1;

        let Some(doc_sha) = substantive_last_change(history, &rel)? else {
            stale.push(StaleDoc {
                doc: rel,
                reason: StaleReason::IndeterminateHistory,
                sources: Vec::new(),
            });
            continue;
        };

        let mut stale_sources = Vec::new();
        let mut doc_level_reason = None;
        for source in &sources {
            if !root.join(source).exists() {
                doc_level_reason = Some(StaleReason::SourcePathMissing);
                break;
            }
            let Some(src_sha) = history.last_change(source)? else {
                doc_level_reason = Some(StaleReason::SourceUntracked);
                break;
            };
            if src_sha != doc_sha && history.is_ancestor(&doc_sha, &src_sha)? {
                stale_sources.push(StaleSource {
                    path: source.clone(),
                    last_change: src_sha.clone(),
                    last_change_subject: history.commit_subject(&src_sha)?,
                });
            }
        }

        if let Some(reason) = doc_level_reason {
            stale.push(StaleDoc {
                doc: rel,
                reason,
                sources: Vec::new(),
            });
        } else if !stale_sources.is_empty() {
            stale.push(StaleDoc {
                doc: rel,
                reason: StaleReason::SourceChangedAfterDoc,
                sources: stale_sources,
            });
        }
    }

    Ok(StalenessReport {
        checked_docs,
        exempt_docs,
        untracked_docs,
        stale,
    })
}

/// Run the derived staleness check and fail if any doc is stale.
///
/// Suitable for wiring into `cargo xtask verify` so stale docs block the
/// gate rather than just being reported.
pub fn enforce_staleness(sh: &Shell, root: &Path) -> Result<()> {
    let report = check_staleness(sh, root)?;

    if report.stale.is_empty() {
        return Ok(());
    }

    let details: Vec<String> = report
        .stale
        .iter()
        .map(|d| {
            if d.sources.is_empty() {
                format!("  {} ({:?})", d.doc, d.reason)
            } else {
                let sources = d
                    .sources
                    .iter()
                    .map(|s| format!("{} ({})", s.path, s.last_change_subject))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("  {} ({:?}): {}", d.doc, d.reason, sources)
            }
        })
        .collect();
    bail!(
        "docs-audit: {} doc(s) stale:\n{}",
        report.stale.len(),
        details.join("\n")
    );
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

    let registry_path = crate::utils::adapter_registry_path();
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

fn check_adapter_manifest_drift(root: &Path) -> Result<()> {
    crate::context::check_adapter_docs(root).with_context(|| {
        "adapter documentation drift between xtask/context.toml and docs/core/FEATURE_MATRIX.mbx.md"
            .to_string()
    })
}

// ── Public entry point ───────────────────────────────────────────────────

/// Checks documented code facts, adapter metadata, and agent definitions for drift.
///
/// Full mode also audits freshness and coverage, then writes `xtask/docs-audit-report.json`.
pub fn run(sh: &Shell, root: &Path, mode: Mode) -> Result<()> {
    eprintln!("--- docs-audit ---");

    check_adapter_manifest_drift(root)?;
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
            let staleness = check_staleness(sh, root)?;
            for d in &staleness.stale {
                eprintln!("docs-audit: STALE {} ({:?})", d.doc, d.reason);
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

            eprintln!(
                "docs-audit (full): {} mismatches, {} checked, {} stale, {} exempt, {} untracked, {} gaps",
                mismatches,
                staleness.checked_docs,
                staleness.stale.len(),
                staleness.exempt_docs,
                staleness.untracked_docs,
                coverage_gaps.len()
            );
            let freshness = staleness.stale;

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

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"schema_version = 1
[[adapters]]
id = "zeta"
maturity = "blocked"
platforms = ["windows"]
default_roles = []
[adapters.capabilities]
build = "blocked"
run = "limited"
[[adapters]]
id = "alpha"
maturity = "production"
platforms = ["linux", "macos"]
default_roles = []
[adapters.capabilities]
build = "no"
run = "yes"
[[profiles]]
id = "native-macos"
target = "aarch64-apple-darwin"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
[[profiles]]
id = "native-linux-gnu"
target = "x86_64-unknown-linux-gnu"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
[[profiles]]
id = "native-linux-musl"
target = "x86_64-unknown-linux-musl"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
[[profiles]]
id = "native-windows"
target = "x86_64-pc-windows-msvc"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
"#;

    const DOCUMENT: &str = "before\n<!-- BEGIN GENERATED: adapter-suites -->\n| Adapter | Platforms | Maturity | Default roles |\n| --- | --- | --- | --- |\n| `alpha` | `linux`, `macos` | Production | -- |\n| `zeta` | `windows` | Blocked | -- |\n<!-- END GENERATED: adapter-suites -->\nmiddle\n<!-- BEGIN GENERATED: adapter-capabilities -->\n| Capability | `alpha` | `zeta` |\n| --- | --- | --- |\n| `build` | No | Blocked |\n| `run` | Yes | Limited |\n<!-- END GENERATED: adapter-capabilities -->\nafter\n";

    fn write_fixture(root: &Path, manifest: &str, document: &str) {
        std::fs::create_dir_all(root.join("xtask")).expect("xtask fixture should be created");
        std::fs::create_dir_all(root.join("docs/core")).expect("docs fixture should be created");
        std::fs::write(root.join("xtask/context.toml"), manifest)
            .expect("manifest fixture should be written");
        std::fs::write(root.join("docs/core/FEATURE_MATRIX.mbx.md"), document)
            .expect("matrix fixture should be written");
    }

    fn assert_drift(root: &Path, manifest: &str, document: &str) {
        write_fixture(root, manifest, document);
        let first = check_adapter_manifest_drift(root)
            .expect_err("drift should fail the docs audit")
            .to_string();
        let second = check_adapter_manifest_drift(root)
            .expect_err("repeated drift should fail identically")
            .to_string();
        assert_eq!(first, second);
        assert!(first.contains("xtask/context.toml"), "{first}");
        assert!(first.contains("docs/core/FEATURE_MATRIX.mbx.md"), "{first}");
    }

    #[derive(Default)]
    struct FakeGitHistory {
        shallow: bool,
        commits_touching: BTreeMap<String, Vec<String>>,
        changed_lines: BTreeMap<String, Vec<String>>,
        last_change: BTreeMap<String, String>,
        subjects: BTreeMap<String, String>,
        ancestors: BTreeSet<(String, String)>,
    }

    impl GitHistory for FakeGitHistory {
        fn is_shallow_repository(&self) -> Result<bool> {
            Ok(self.shallow)
        }
        fn last_change(&self, path: &str) -> Result<Option<String>> {
            Ok(self.last_change.get(path).cloned())
        }
        fn commit_subject(&self, sha: &str) -> Result<String> {
            Ok(self.subjects.get(sha).cloned().unwrap_or_default())
        }
        fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
            Ok(self
                .ancestors
                .contains(&(ancestor.to_string(), descendant.to_string())))
        }
        fn commits_touching(&self, path: &str) -> Result<Vec<String>> {
            Ok(self.commits_touching.get(path).cloned().unwrap_or_default())
        }
        fn commit_changed_lines(&self, sha: &str, _path: &str) -> Result<Vec<String>> {
            Ok(self.changed_lines.get(sha).cloned().unwrap_or_default())
        }
    }

    #[test]
    fn substantive_change_skips_stamp_only_commit() {
        let mut fake = FakeGitHistory::default();
        fake.commits_touching
            .insert("doc.md".into(), vec!["c2".into(), "c1".into()]);
        fake.changed_lines
            .insert("c2".into(), vec!["Last updated: 2026-01-01".into()]);
        fake.changed_lines
            .insert("c1".into(), vec!["some real content".into()]);

        let result = substantive_last_change(&fake, "doc.md").expect("walk should succeed");
        assert_eq!(result, Some("c1".to_string()));
    }

    #[test]
    fn substantive_change_indeterminate_when_history_is_stamp_only() {
        let mut fake = FakeGitHistory::default();
        fake.commits_touching
            .insert("doc.md".into(), vec!["c1".into()]);
        fake.changed_lines
            .insert("c1".into(), vec!["Last updated: 2026-01-01".into()]);

        let result = substantive_last_change(&fake, "doc.md").expect("walk should succeed");
        assert_eq!(result, None);
    }

    #[test]
    fn check_staleness_errors_on_shallow_repo() {
        let fake = FakeGitHistory {
            shallow: true,
            ..Default::default()
        };
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let err = check_staleness_with(&fake, temp.path())
            .expect_err("shallow repository should refuse rather than report a false pass");
        assert!(err.to_string().contains("shallow"));
    }

    #[test]
    fn check_staleness_skips_docs_without_sources() {
        let fake = FakeGitHistory::default();
        let temp = tempfile::tempdir().expect("temp dir should be created");
        std::fs::create_dir_all(temp.path().join("docs/core")).expect("fixture dir");
        std::fs::write(
            temp.path().join("docs/core/ARCHITECTURE.mbx.md"),
            "---\nstatus: active\n---\n# No sources\n",
        )
        .expect("fixture doc");

        let report = check_staleness_with(&fake, temp.path()).expect("check should succeed");
        assert_eq!(report.checked_docs, 0);
        assert!(report.stale.is_empty());
    }

    #[test]
    fn check_staleness_skips_exempt_design_docs() {
        let fake = FakeGitHistory::default();
        let temp = tempfile::tempdir().expect("temp dir should be created");
        std::fs::create_dir_all(temp.path().join("docs/designs")).expect("fixture dir");
        std::fs::write(
            temp.path()
                .join("docs/designs/2026-01-01-example-design.md"),
            "---\nstatus: proposed\nsources:\n  - xtask/src/docs_audit.rs\n---\n# Design\n",
        )
        .expect("fixture doc");

        let report = check_staleness_with(&fake, temp.path()).expect("check should succeed");
        assert_eq!(report.exempt_docs, 1);
        assert_eq!(report.checked_docs, 0);
        assert!(report.stale.is_empty());
    }

    #[test]
    fn check_staleness_reports_source_changed_after_doc() {
        let mut fake = FakeGitHistory::default();
        fake.commits_touching
            .insert("docs/core/ARCHITECTURE.mbx.md".into(), vec!["doc1".into()]);
        fake.changed_lines
            .insert("doc1".into(), vec!["real doc content".into()]);
        fake.last_change
            .insert("xtask/src/docs_audit.rs".into(), "src2".into());
        fake.subjects
            .insert("src2".into(), "feat: change the audited behavior".into());
        fake.ancestors.insert(("doc1".into(), "src2".into()));

        let temp = tempfile::tempdir().expect("temp dir should be created");
        std::fs::create_dir_all(temp.path().join("docs/core")).expect("fixture dir");
        std::fs::create_dir_all(temp.path().join("xtask/src")).expect("fixture dir");
        std::fs::write(
            temp.path().join("docs/core/ARCHITECTURE.mbx.md"),
            "---\nstatus: active\nsources:\n  - xtask/src/docs_audit.rs\n---\n# Doc\n",
        )
        .expect("fixture doc");
        std::fs::write(temp.path().join("xtask/src/docs_audit.rs"), "// src").expect("fixture");

        let report = check_staleness_with(&fake, temp.path()).expect("check should succeed");
        assert_eq!(report.checked_docs, 1);
        assert_eq!(report.stale.len(), 1);
        assert!(matches!(
            report.stale[0].reason,
            StaleReason::SourceChangedAfterDoc
        ));
        assert_eq!(report.stale[0].sources[0].path, "xtask/src/docs_audit.rs");
    }

    #[test]
    fn check_staleness_reports_source_path_missing() {
        let mut fake = FakeGitHistory::default();
        fake.commits_touching
            .insert("docs/core/ARCHITECTURE.mbx.md".into(), vec!["doc1".into()]);
        fake.changed_lines
            .insert("doc1".into(), vec!["real doc content".into()]);

        let temp = tempfile::tempdir().expect("temp dir should be created");
        std::fs::create_dir_all(temp.path().join("docs/core")).expect("fixture dir");
        std::fs::write(
            temp.path().join("docs/core/ARCHITECTURE.mbx.md"),
            "---\nstatus: active\nsources:\n  - xtask/src/nonexistent.rs\n---\n# Doc\n",
        )
        .expect("fixture doc");

        let report = check_staleness_with(&fake, temp.path()).expect("check should succeed");
        assert_eq!(report.stale.len(), 1);
        assert!(matches!(
            report.stale[0].reason,
            StaleReason::SourcePathMissing
        ));
    }

    #[test]
    fn check_staleness_rejects_legacy_source_sha() {
        let fake = FakeGitHistory::default();
        let temp = tempfile::tempdir().expect("temp dir should be created");
        std::fs::create_dir_all(temp.path().join("docs/core")).expect("fixture dir");
        std::fs::write(
            temp.path().join("docs/core/ARCHITECTURE.mbx.md"),
            "---\nstatus: active\nsources:\n  - xtask/src/docs_audit.rs\nsource_sha: deadbeef\n---\n# Doc\n",
        )
        .expect("fixture doc");

        let err = check_staleness_with(&fake, temp.path())
            .expect_err("a returned legacy source_sha scalar should fail the check");
        assert!(err.to_string().contains("LegacySourceSha"));
    }

    #[test]
    fn docs_audit_rejects_adapter_manifest_drift() {
        let temp = tempfile::tempdir().expect("temporary docs root should be created");
        write_fixture(temp.path(), MANIFEST, DOCUMENT);
        check_adapter_manifest_drift(temp.path()).expect("exact generated blocks should pass");

        assert_drift(
            temp.path(),
            &MANIFEST.replacen(
                "maturity = \"production\"",
                "maturity = \"experimental\"",
                1,
            ),
            DOCUMENT,
        );
        assert_drift(
            temp.path(),
            &MANIFEST.replacen("build = \"no\"", "build = \"yes\"", 1),
            DOCUMENT,
        );
        assert_drift(
            temp.path(),
            &MANIFEST.replacen("id = \"alpha\"", "id = \"beta\"", 1),
            DOCUMENT,
        );
        assert_drift(
            temp.path(),
            &MANIFEST.replacen(
                "platforms = [\"linux\", \"macos\"]",
                "platforms = [\"linux\"]",
                1,
            ),
            DOCUMENT,
        );
        assert_drift(
            temp.path(),
            &MANIFEST.replacen("default_roles = []", "default_roles = [\"custom\"]", 1),
            DOCUMENT,
        );
        assert_drift(
            temp.path(),
            MANIFEST,
            &DOCUMENT.replace("| `alpha` | `linux`, `macos` | Production | -- |\n", ""),
        );
        assert_drift(
            temp.path(),
            MANIFEST,
            &DOCUMENT.replace(
                "<!-- END GENERATED: adapter-suites -->",
                "| `extra` | `linux` | Stub | -- |\n<!-- END GENERATED: adapter-suites -->",
            ),
        );
    }
}
