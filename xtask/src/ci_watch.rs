//! `cargo xtask ci-watch` — watch the latest GHA run with job-level detail.
//!
//! Infers the repo from `gh repo view` and the branch from `git branch --show-current`
//! (or `--branch <name>` flag). Prints a header with commit/trigger info, lists all
//! jobs with status icons before and after watching.
//!
//! Run: `cargo xtask ci-watch [--branch <branch>]`

use anyhow::{Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use xshell::{Shell, cmd};

#[derive(Deserialize)]
struct RepoInfo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Deserialize)]
struct RunSummary {
    #[serde(rename = "databaseId")]
    database_id: u64,
    #[serde(rename = "displayTitle")]
    display_title: String,
    #[serde(rename = "headSha")]
    head_sha: String,
    event: String,
    status: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "workflowName")]
    workflow_name: String,
}

#[derive(Deserialize)]
struct RunDetail {
    conclusion: Option<String>,
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    name: String,
    status: String,
    conclusion: Option<String>,
    #[serde(rename = "startedAt")]
    started_at: Option<String>,
    #[serde(rename = "completedAt")]
    completed_at: Option<String>,
}

fn status_icon(conclusion: Option<&str>, status: &str) -> &'static str {
    match conclusion {
        Some("success") => "✓",
        Some("failure") => "✗",
        Some("cancelled") => "⊘",
        Some("skipped") => "−",
        _ => match status {
            "in_progress" => "…",
            "queued" => "·",
            _ => "?",
        },
    }
}

fn elapsed(started: &str, completed: &str) -> String {
    let Ok(s) = DateTime::parse_from_rfc3339(started) else {
        return String::new();
    };
    let Ok(e) = DateTime::parse_from_rfc3339(completed) else {
        return String::new();
    };
    let secs = (e - s).num_seconds();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

fn print_jobs(jobs: &[Job]) {
    println!();
    for j in jobs {
        let icon = status_icon(j.conclusion.as_deref(), &j.status);
        let timing = match (&j.started_at, &j.completed_at) {
            (Some(s), Some(e)) => {
                let d = elapsed(s, e);
                if d.is_empty() { j.status.clone() } else { d }
            }
            _ => j.status.clone(),
        };
        println!("  {icon} {} — {timing}", j.name);
    }
    println!();
}

fn fetch_detail(sh: &Shell, run_id: &str, repo: &str) -> Result<RunDetail> {
    let json = cmd!(
        sh,
        "gh run view {run_id} --repo {repo} --json conclusion,jobs"
    )
    .read()?;
    serde_json::from_str(&json).context("parse gh run view output")
}

pub fn ci_watch(sh: &Shell, branch: Option<&str>) -> Result<()> {
    let repo_json = cmd!(sh, "gh repo view --json nameWithOwner").read()?;
    let repo: RepoInfo = serde_json::from_str(&repo_json).context("parse gh repo view output")?;
    let repo = &repo.name_with_owner;

    let current_branch;
    let br = match branch {
        Some(b) => b,
        None => {
            current_branch = cmd!(sh, "git branch --show-current").read()?;
            current_branch.trim()
        }
    };

    let runs_json = cmd!(
        sh,
        "gh run list --branch {br} --repo {repo} --limit 1
         --json databaseId,displayTitle,headSha,event,status,createdAt,workflowName"
    )
    .read()?;
    let runs: Vec<RunSummary> =
        serde_json::from_str(&runs_json).context("parse gh run list output")?;
    let run = runs.into_iter().next().context("no runs found")?;
    let run_id = run.database_id.to_string();
    let sha = &run.head_sha[..7.min(run.head_sha.len())];

    println!("\n━━━ CI Run {run_id} ━━━");
    println!("  Repo:     {repo}");
    println!("  Branch:   {br}");
    println!("  Workflow: {}", run.workflow_name);
    println!("  Trigger:  {}", run.event);
    println!("  Commit:   {sha} — {}", run.display_title);
    println!("  Started:  {}", run.created_at);
    println!("  Status:   {}", run.status);

    print_jobs(&fetch_detail(sh, &run_id, repo)?.jobs);

    let _ = cmd!(sh, "gh run watch {run_id} --repo {repo} --exit-status").run();

    let detail = fetch_detail(sh, &run_id, repo)?;
    let conclusion = detail.conclusion.as_deref().unwrap_or("unknown");
    let icon = status_icon(Some(conclusion), "completed");
    println!("\n━━━ Result: {icon} {} ━━━", conclusion.to_uppercase());
    print_jobs(&detail.jobs);

    Ok(())
}
