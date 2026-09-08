//! `cargo xtask context` — machine-readable repo context snapshot.
//!
//! Outputs a single JSON document describing workspace shape: crate graph,
//! adapter wiring, test counts, recent commits. Designed for cold-start
//! LLM sessions that need project context without reading every file.

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

#[allow(dead_code)]
mod model;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextOptions {
    pub save: bool,
    pub strict: bool,
    pub validate_all: bool,
    pub evidence_dir: Option<PathBuf>,
}

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
    context_map: ContextMap,
}

#[derive(Debug, Default, Serialize)]
struct ContextMap {
    crate_assignments: Vec<CrateAssignment>,
    file_assignments: Vec<FileAssignment>,
    task_slices: Vec<TaskSlice>,
}

#[derive(Debug, Serialize)]
struct CrateAssignment {
    crate_name: String,
    lines: usize,
}

#[derive(Debug, Serialize)]
struct FileAssignment {
    path: String,
    responsibility: String,
}

#[derive(Debug, Serialize)]
struct TaskSlice {
    id: String,
    title: String,
    depends_on: Vec<String>,
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
            .unwrap_or_else(|| Path::new("."));

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

fn derive_crate_assignments(crates: &[CrateInfo]) -> Vec<CrateAssignment> {
    let mut assignments: Vec<_> = crates
        .iter()
        .map(|crate_info| CrateAssignment {
            crate_name: crate_info.name.clone(),
            lines: crate_info.lines,
        })
        .collect();
    assignments.sort_by(|left, right| {
        right
            .lines
            .cmp(&left.lines)
            .then_with(|| left.crate_name.cmp(&right.crate_name))
    });
    assignments
}

fn derive_file_assignments() -> Vec<FileAssignment> {
    let mut assignments = [
        (
            "xtask/src/context.rs",
            "context snapshot schema and derivation",
        ),
        ("xtask/src/main.rs", "info context command dispatch"),
        (
            "xtask/schema/cli.schema.json",
            "machine-readable command contract",
        ),
        (
            "docs/core/XTASK_CLI.mbx.md",
            "human-readable context output contract",
        ),
    ]
    .into_iter()
    .map(|(path, responsibility)| FileAssignment {
        path: path.to_string(),
        responsibility: responsibility.to_string(),
    })
    .collect::<Vec<_>>();
    assignments.sort_by(|left, right| left.path.cmp(&right.path));
    assignments
}

fn derive_task_slices() -> Vec<TaskSlice> {
    vec![
        TaskSlice {
            id: "t1".to_string(),
            title: "Collect repository context".to_string(),
            depends_on: Vec::new(),
        },
        TaskSlice {
            id: "t2".to_string(),
            title: "Derive crate assignments".to_string(),
            depends_on: vec!["t1".to_string()],
        },
        TaskSlice {
            id: "t3".to_string(),
            title: "Derive file assignments".to_string(),
            depends_on: vec!["t1".to_string()],
        },
        TaskSlice {
            id: "t4".to_string(),
            title: "Serialize and save context snapshot".to_string(),
            depends_on: vec!["t2".to_string(), "t3".to_string()],
        },
    ]
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

// qual:allow(iosp) reason: "recursive fs traversal"
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
                && (name.to_ascii_lowercase().ends_with(".yml")
                    || name.to_ascii_lowercase().ends_with(".yaml"))
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

fn persist_snapshot(root: &Path, snapshot: &ContextSnapshot) -> Result<std::path::PathBuf> {
    let dir = root.join("artifacts/context");
    std::fs::create_dir_all(&dir).context("create artifacts/context")?;

    let latest = dir.join("snapshot.json");
    let json = serde_json::to_string_pretty(snapshot).context("serialize snapshot")?;
    std::fs::write(&latest, json).context("write snapshot.json")?;

    let jsonl = dir.join("history.jsonl");
    use std::io::Write;
    let compact = serde_json::to_string(snapshot).context("serialize history record")?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&jsonl)
        .context("open history.jsonl")?;
    writeln!(file, "{compact}").context("append history.jsonl")?;

    Ok(latest)
}

// ─── Entry point ─────────────────────────────────────────────────────────────

// qual:allow(iosp) reason: "xtask entrypoint: shells out + reads fs + aggregates into snapshot"
pub fn context(sh: &Shell, root: &Path, options: &ContextOptions) -> Result<()> {
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
    let context_map = ContextMap {
        crate_assignments: derive_crate_assignments(&crates),
        file_assignments: derive_file_assignments(),
        task_slices: derive_task_slices(),
    };

    let snapshot = ContextSnapshot {
        snapshot_version: 2,
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
        context_map,
    };

    if options.save {
        let latest = persist_snapshot(root, &snapshot)?;
        eprintln!("Context snapshot saved to {}", latest.display());
    } else {
        let json = serde_json::to_string_pretty(&snapshot).context("serialize snapshot")?;
        println!("{json}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_info(name: &str, lines: usize) -> CrateInfo {
        CrateInfo {
            name: name.to_string(),
            kind: vec!["lib".to_string()],
            deps: Vec::new(),
            test_count: 0,
            src_files: 1,
            lines,
        }
    }

    fn snapshot_fixture() -> ContextSnapshot {
        ContextSnapshot {
            snapshot_version: 2,
            commit: "abc1234".to_string(),
            branch: "develop".to_string(),
            timestamp: "2026-09-06T00:00:00Z".to_string(),
            workspace: WorkspaceInfo {
                version: "0.33.0".to_string(),
                edition: "2024".to_string(),
                rust_version: "1.89.0".to_string(),
            },
            crates: Vec::new(),
            adapters: BTreeMap::new(),
            tests: TestSummary {
                total: 0,
                by_crate: BTreeMap::new(),
            },
            ci_workflows: Vec::new(),
            recent_commits: Vec::new(),
            context_map: ContextMap::default(),
        }
    }

    #[test]
    fn context_snapshot_includes_context_map() {
        let value = serde_json::to_value(snapshot_fixture()).expect("snapshot should serialize");
        assert_eq!(value["snapshot_version"], 2);
        let context_map = value
            .get("context_map")
            .expect("snapshot v2 should include context_map");
        assert!(context_map.get("crate_assignments").is_some());
        assert!(context_map.get("file_assignments").is_some());
        assert!(context_map.get("task_slices").is_some());
    }

    #[test]
    fn crate_assignments_are_sorted_and_stable() {
        let crates = vec![
            crate_info("zeta", 100),
            crate_info("middle", 50),
            crate_info("alpha", 100),
        ];

        let assignments = derive_crate_assignments(&crates);
        let names: Vec<&str> = assignments
            .iter()
            .map(|assignment| assignment.crate_name.as_str())
            .collect();

        assert_eq!(names, vec!["alpha", "zeta", "middle"]);
        assert_eq!(assignments[0].lines, 100);
        assert_eq!(assignments[2].lines, 50);
    }

    #[test]
    fn file_assignments_cover_xtask_info_context_surface() {
        let assignments = derive_file_assignments();
        let paths: Vec<&str> = assignments
            .iter()
            .map(|assignment| assignment.path.as_str())
            .collect();

        assert_eq!(
            paths,
            vec![
                "docs/core/XTASK_CLI.mbx.md",
                "xtask/schema/cli.schema.json",
                "xtask/src/context.rs",
                "xtask/src/main.rs",
            ]
        );
        assert!(
            assignments
                .iter()
                .all(|assignment| !assignment.responsibility.is_empty())
        );
    }

    #[test]
    fn task_slices_define_expected_dependency_graph() {
        let slices = derive_task_slices();
        let dependencies: BTreeMap<&str, Vec<&str>> = slices
            .iter()
            .map(|slice| {
                (
                    slice.id.as_str(),
                    slice.depends_on.iter().map(String::as_str).collect(),
                )
            })
            .collect();

        assert_eq!(dependencies["t1"], Vec::<&str>::new());
        assert_eq!(dependencies["t2"], vec!["t1"]);
        assert_eq!(dependencies["t3"], vec!["t1"]);
        assert_eq!(dependencies["t4"], vec!["t2", "t3"]);
        assert!(slices.iter().all(|slice| !slice.title.is_empty()));
    }

    #[test]
    fn save_mode_persists_context_map_in_snapshot_and_history() {
        let temp = tempfile::tempdir().expect("temporary output root should be created");
        let snapshot = snapshot_fixture();

        let saved = persist_snapshot(temp.path(), &snapshot)
            .expect("snapshot and history should be persisted");
        assert_eq!(saved, temp.path().join("artifacts/context/snapshot.json"));

        let latest = std::fs::read_to_string(&saved).expect("snapshot.json should be readable");
        let latest: serde_json::Value =
            serde_json::from_str(&latest).expect("snapshot.json should contain valid JSON");
        assert_eq!(latest["snapshot_version"], 2);
        assert!(latest["context_map"].is_object());

        let history = std::fs::read_to_string(temp.path().join("artifacts/context/history.jsonl"))
            .expect("history.jsonl should be readable");
        let record: serde_json::Value =
            serde_json::from_str(history.trim()).expect("history line should contain valid JSON");
        assert_eq!(record["context_map"], latest["context_map"]);
    }
}
