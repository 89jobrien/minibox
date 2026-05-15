//! `cargo xtask context` — machine-readable repo context snapshot.
//!
//! Outputs a single JSON document describing workspace shape: crate graph,
//! adapter wiring, test counts, recent commits. Designed for cold-start
//! LLM sessions that need project context without reading every file.

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use xshell::{Shell, cmd};

// ─── Output schema ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ContextSnapshot {
    snapshot_version: u32,
    commit: String,
    branch: String,
    timestamp: String,
    workspace: WorkspaceInfo,
    crates: Vec<CrateInfo>,
    adapters: BTreeMap<String, AdapterInfo>,
    tests: TestSummary,
    ci_workflows: Vec<String>,
    recent_commits: Vec<CommitInfo>,
}

#[derive(Serialize)]
struct WorkspaceInfo {
    version: String,
    edition: String,
    rust_version: String,
}

#[derive(Serialize)]
struct CrateInfo {
    name: String,
    kind: Vec<String>,
    deps: Vec<String>,
    test_count: usize,
    src_files: usize,
    lines: usize,
}

#[derive(Serialize)]
struct AdapterInfo {
    platform: String,
    status: String,
}

#[derive(Serialize)]
struct TestSummary {
    total: usize,
    by_crate: BTreeMap<String, usize>,
}

#[derive(Serialize)]
struct CommitInfo {
    hash: String,
    subject: String,
}

// ─── Data collection ─────────────────────────────────────────────────────────

fn git_info(sh: &Shell) -> Result<(String, String, String)> {
    let commit = cmd!(sh, "git rev-parse --short HEAD").read()?;
    let branch = cmd!(sh, "git branch --show-current").read()?;
    let timestamp = cmd!(sh, "date -u +%Y-%m-%dT%H:%M:%SZ").read()?;
    Ok((
        commit.trim().to_string(),
        branch.trim().to_string(),
        timestamp.trim().to_string(),
    ))
}

fn recent_commits(sh: &Shell) -> Result<Vec<CommitInfo>> {
    let log = cmd!(sh, "git log --oneline -10").read()?;
    Ok(log
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let (hash, subject) = line.split_once(' ').unwrap_or((line, ""));
            CommitInfo {
                hash: hash.to_string(),
                subject: subject.to_string(),
            }
        })
        .collect())
}

/// Parse `cargo metadata --no-deps` for crate graph.
fn crate_graph(sh: &Shell) -> Result<Vec<CrateInfo>> {
    let raw = cmd!(sh, "cargo metadata --no-deps --format-version 1")
        .read()
        .context("cargo metadata")?;
    let meta: serde_json::Value = serde_json::from_str(&raw).context("parse cargo metadata")?;

    let workspace_members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let packages = meta["packages"]
        .as_array()
        .context("no packages in metadata")?;

    let mut crates = Vec::new();
    for pkg in packages {
        let name = pkg["name"].as_str().unwrap_or("").to_string();
        let pkg_id = pkg["id"].as_str().unwrap_or("");
        if !workspace_members.iter().any(|m| m.contains(&name)) {
            continue;
        }

        let targets = pkg["targets"].as_array();
        let empty_vec = vec![];
        let kind: Vec<String> = targets
            .map(|ts| {
                ts.iter()
                    .flat_map(|t| {
                        t["kind"]
                            .as_array()
                            .unwrap_or(&empty_vec)
                            .iter()
                            .filter_map(|k| k.as_str().map(String::from))
                    })
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();

        let deps: Vec<String> = pkg["dependencies"]
            .as_array()
            .map(|ds| {
                ds.iter()
                    .filter_map(|d| d["name"].as_str().map(String::from))
                    .filter(|n| workspace_members.iter().any(|m| m.contains(n.as_str())))
                    .collect()
            })
            .unwrap_or_default();

        let manifest_path = pkg["manifest_path"].as_str().unwrap_or("");
        let crate_dir = std::path::Path::new(manifest_path)
            .parent()
            .unwrap_or(Path::new("."));

        let (src_files, lines) = count_source(crate_dir);

        crates.push(CrateInfo {
            name,
            kind,
            deps,
            test_count: 0, // filled in later
            src_files,
            lines,
        });

        // suppress unused variable warning
        let _ = pkg_id;
    }
    Ok(crates)
}

/// Count .rs files and total lines under a crate directory.
fn count_source(crate_dir: &Path) -> (usize, usize) {
    let src_dir = crate_dir.join("src");
    let dir = if src_dir.is_dir() {
        &src_dir
    } else {
        crate_dir
    };
    let mut files = 0usize;
    let mut lines = 0usize;
    if let Ok(entries) = walkdir(dir) {
        for path in entries {
            if path.extension().is_some_and(|e| e == "rs") {
                files += 1;
                if let Ok(content) = std::fs::read_to_string(&path) {
                    lines += content.lines().count();
                }
            }
        }
    }
    (files, lines)
}

/// Simple recursive file listing (avoids adding walkdir dep).
fn walkdir(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    walkdir_inner(dir, &mut out)?;
    Ok(out)
}

fn walkdir_inner(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walkdir_inner(&entry.path(), out)?;
        } else if ft.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}

/// Parse `cargo nextest list` output for test counts per crate.
fn test_counts(sh: &Shell) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    // nextest list outputs lines like: "minibox-core protocol::tests::test_name"
    // (space-separated: crate_name test_path)
    let output = cmd!(sh, "cargo nextest list --workspace")
        .read()
        .unwrap_or_default();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // First word is the crate name
        if let Some(crate_name) = line.split_whitespace().next() {
            *counts.entry(crate_name.to_string()).or_default() += 1;
        }
    }
    counts
}

fn workspace_version(sh: &Shell) -> Result<WorkspaceInfo> {
    let raw = cmd!(sh, "cargo metadata --no-deps --format-version 1")
        .read()
        .context("cargo metadata for version")?;
    let meta: serde_json::Value = serde_json::from_str(&raw)?;

    // workspace version from first workspace package
    let version = meta["packages"]
        .as_array()
        .and_then(|ps| {
            ps.iter()
                .find(|p| p["name"].as_str() == Some("minibox"))
                .and_then(|p| p["version"].as_str().map(String::from))
        })
        .unwrap_or_else(|| "unknown".to_string());

    let edition = meta["packages"]
        .as_array()
        .and_then(|ps| {
            ps.iter()
                .find(|p| p["name"].as_str() == Some("minibox"))
                .and_then(|p| p["edition"].as_str().map(String::from))
        })
        .unwrap_or_else(|| "2024".to_string());

    let rust_version = cmd!(sh, "rustc --version")
        .read()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .unwrap_or("unknown")
        .to_string();

    Ok(WorkspaceInfo {
        version,
        edition,
        rust_version,
    })
}

fn ci_workflows(root: &Path) -> Vec<String> {
    let wf_dir = root.join(".github/workflows");
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(wf_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && (name.ends_with(".yml") || name.ends_with(".yaml"))
            {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names
}

fn adapter_table() -> BTreeMap<String, AdapterInfo> {
    let mut m = BTreeMap::new();
    let entries = [
        ("native", "linux", "production"),
        ("gke", "linux", "production"),
        ("smolvm", "any", "production"),
        ("krun", "macos", "production"),
        ("colima", "macos", "experimental"),
        ("vz", "macos", "blocked"),
        ("wsl2", "windows", "stub"),
        ("hcs", "windows", "stub"),
        ("docker_desktop", "macos", "stub"),
    ];
    for (name, platform, status) in entries {
        m.insert(
            name.to_string(),
            AdapterInfo {
                platform: platform.to_string(),
                status: status.to_string(),
            },
        );
    }
    m
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn context(sh: &Shell, root: &Path, save: bool) -> Result<()> {
    let (commit, branch, timestamp) = git_info(sh)?;
    let workspace = workspace_version(sh)?;
    let mut crates = crate_graph(sh)?;
    let counts = test_counts(sh);

    let mut total_tests = 0usize;
    for c in &mut crates {
        if let Some(&n) = counts.get(&c.name) {
            c.test_count = n;
            total_tests += n;
        }
    }

    let by_crate: BTreeMap<String, usize> = counts.iter().map(|(k, &v)| (k.clone(), v)).collect();

    let snapshot = ContextSnapshot {
        snapshot_version: 1,
        commit,
        branch,
        timestamp,
        workspace,
        crates,
        adapters: adapter_table(),
        tests: TestSummary {
            total: total_tests,
            by_crate,
        },
        ci_workflows: ci_workflows(root),
        recent_commits: recent_commits(sh)?,
    };

    let json = serde_json::to_string_pretty(&snapshot).context("serialize snapshot")?;

    if save {
        let dir = root.join("artifacts/context");
        std::fs::create_dir_all(&dir).context("create artifacts/context")?;

        let latest = dir.join("snapshot.json");
        std::fs::write(&latest, &json).context("write snapshot.json")?;

        let jsonl = dir.join("history.jsonl");
        use std::io::Write;
        let compact = serde_json::to_string(&snapshot)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl)
            .context("open history.jsonl")?;
        writeln!(f, "{compact}")?;

        eprintln!("Context snapshot saved to {}", latest.display());
    } else {
        println!("{json}");
    }

    Ok(())
}
