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
#[allow(dead_code)]
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

fn elapsed(started: &str, completed: &str) -> Option<String> {
    let Ok(s) = DateTime::parse_from_rfc3339(started) else {
        return None;
    };
    let Ok(e) = DateTime::parse_from_rfc3339(completed) else {
        return None;
    };
    let secs = (e - s).num_seconds();
    if secs <= 0 {
        return None;
    }
    if secs < 60 {
        Some(format!("{secs}s"))
    } else {
        Some(format!("{}m{}s", secs / 60, secs % 60))
    }
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
    let br = if let Some(b) = branch {
        b
    } else {
        current_branch = cmd!(sh, "git branch --show-current").read()?;
        current_branch.trim()
    };

    // Fetch enough runs to capture all workflows triggered by the latest push.
    let runs_json = cmd!(
        sh,
        "gh run list --branch {br} --repo {repo} --limit 15
         --json databaseId,displayTitle,headSha,event,status,createdAt,workflowName"
    )
    .read()?;
    let all_runs: Vec<RunSummary> =
        serde_json::from_str(&runs_json).context("parse gh run list output")?;

    if all_runs.is_empty() {
        anyhow::bail!("no runs found for branch {br}");
    }

    // Group by HEAD SHA — only show runs matching the latest commit.
    let head_sha = all_runs[0].head_sha.clone();
    let runs: Vec<RunSummary> = all_runs
        .into_iter()
        .filter(|r| r.head_sha == head_sha)
        .collect();

    let sha = &head_sha[..7.min(head_sha.len())];
    println!("\n━━━ CI Watch — {repo} @ {br} ━━━");
    println!("  Commit:    {sha} — {}", runs[0].display_title);
    println!("  Trigger:   {}", runs[0].event);
    println!("  Workflows: {}", runs.len());
    println!();

    // Print initial status for all workflows.
    for run in &runs {
        let detail = fetch_detail(sh, &run.database_id.to_string(), repo)?;
        let conclusion = detail.conclusion.as_deref();
        let icon = status_icon(conclusion, &run.status);
        println!(
            "  {icon} {} — {}",
            run.workflow_name,
            conclusion.unwrap_or(&run.status)
        );
    }

    // Watch any runs that haven't completed yet.
    let pending: Vec<&RunSummary> = runs.iter().filter(|r| r.status != "completed").collect();

    if !pending.is_empty() {
        println!("\nWaiting on {} workflow(s)...\n", pending.len());
        for run in &pending {
            let run_id = run.database_id.to_string();
            println!("  Watching: {}", run.workflow_name);
            let _ = cmd!(sh, "gh run watch {run_id} --repo {repo} --exit-status").run();
        }
    }

    // Final aggregate summary.
    println!("\n━━━ Final Results ━━━");
    let mut passed = 0usize;
    let mut failed = 0usize;
    for run in &runs {
        let run_id = run.database_id.to_string();
        let detail = fetch_detail(sh, &run_id, repo)?;
        let conclusion = detail.conclusion.as_deref().unwrap_or("unknown");
        let icon = status_icon(Some(conclusion), "completed");
        match conclusion {
            "success" => passed += 1,
            "failure" => failed += 1,
            _ => {}
        }
        println!("\n  {icon} {}", run.workflow_name);
        for j in &detail.jobs {
            let jicon = status_icon(j.conclusion.as_deref(), &j.status);
            let timing = match (&j.started_at, &j.completed_at) {
                (Some(s), Some(e)) if j.conclusion.is_some() => {
                    elapsed(s, e).unwrap_or_else(|| j.status.clone())
                }
                _ => j.status.clone(),
            };
            println!("    {jicon} {} — {timing}", j.name);
        }
    }

    let overall = if failed > 0 { "FAILURE" } else { "SUCCESS" };
    let overall_icon = if failed > 0 { "✗" } else { "✓" };
    println!(
        "\n━━━ Overall: {overall_icon} {overall} ({passed}/{} workflows passed) ━━━\n",
        runs.len()
    );

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}
