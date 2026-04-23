#!/usr/bin/env rust-script
//! Pre-push AI code review — security and correctness focused for minibox.
//!
//! Usage: ./scripts/ai-review.rs [--base <ref>]
//!
//! Requires: rust-script, claude CLI on PATH, ANTHROPIC_API_KEY in env.
//!
//! ```cargo
//! [dependencies]
//! anyhow = "1"
//! clap = { version = "4", features = ["derive"] }
//! chrono = "0.4"
//! serde = { version = "1", features = ["derive"] }
//! serde_json = "1"
//! ```

use anyhow::{bail, Context, Result};
use chrono::Local;
use clap::Parser;
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::Command,
    time::Instant,
};

#[derive(Parser)]
#[command(about = "Pre-push AI code review for minibox")]
struct Args {
    /// Base branch/ref to diff against
    #[arg(long, default_value = "main")]
    base: String,
}

// ---------------------------------------------------------------------------
// Telemetry (mirrors agent_log.py sinks)
// ---------------------------------------------------------------------------

fn log_dir() -> PathBuf {
    dirs_home().join(".minibox")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

#[derive(Serialize)]
struct LogEntry<'a> {
    run_id: &'a str,
    script: &'a str,
    args: serde_json::Value,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<&'a str>,
}

fn log_start(run_id: &str, base: &str) -> Result<()> {
    let dir = log_dir();
    fs::create_dir_all(&dir)?;
    let entry = LogEntry {
        run_id,
        script: "ai-review",
        args: serde_json::json!({ "base": base }),
        status: "running",
        duration_s: None,
        output: None,
    };
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("agent-runs.jsonl"))?;
    writeln!(f, "{}", serde_json::to_string(&entry)?)?;
    Ok(())
}

fn log_complete(run_id: &str, base: &str, output: &str, duration_s: f64) -> Result<()> {
    let entry = LogEntry {
        run_id,
        script: "ai-review",
        args: serde_json::json!({ "base": base }),
        status: "complete",
        duration_s: Some((duration_s * 100.0).round() / 100.0),
        output: Some(output),
    };
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir().join("agent-runs.jsonl"))?;
    writeln!(f, "{}", serde_json::to_string(&entry)?)?;
    Ok(())
}

fn save_commit_log(sha: &str, content: &str, base: &str) -> Result<PathBuf> {
    let dir = log_dir().join("ai-logs");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{sha}-ai-review.md"));
    let date = Local::now().format("%Y-%m-%d %H:%M").to_string();
    let header = format!("# ai-review · {sha}\n\n- **base**: {base}\n- **date**: {date}\n\n---\n\n");
    fs::write(&path, format!("{header}{content}"))?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

fn git_short_sha() -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .context("git rev-parse failed")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn get_diff(base: &str) -> Result<String> {
    // Try three-dot diff first (changes on this branch vs base).
    let out = Command::new("git")
        .args(["diff", &format!("{base}...HEAD")])
        .output()
        .context("git diff failed")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "git diff failed — base ref '{base}' may not exist locally.\n  stderr: {}",
            stderr.trim()
        );
    }

    let diff = String::from_utf8_lossy(&out.stdout).to_string();
    if !diff.trim().is_empty() {
        return Ok(diff);
    }

    // Fallback: unstaged changes against HEAD.
    let out = Command::new("git")
        .args(["diff", "HEAD"])
        .output()
        .context("git diff HEAD failed")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("git diff HEAD failed.\n  stderr: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

// ---------------------------------------------------------------------------
// Review via `claude` CLI
// ---------------------------------------------------------------------------

fn build_prompt(diff: &str, base: &str) -> String {
    format!(
        r#"Review this diff for the minibox project — a Linux container runtime in Rust.

Focus on:
- **Security**: path traversal, symlink attacks, tar extraction safety, socket auth bypass
- **Correctness**: cgroup v2 semantics, namespace/clone flag usage, pivot_root ordering,
  overlay mount flags, pipe fd handling across clone()
- **Protocol**: breaking changes to JSON-over-newline types in protocol.rs
- **Unsafe blocks**: soundness, missing invariant comments
- **Error handling**: silent failures in container init (post-fork context — no unwrap)

For each issue: file + line, severity (critical/major/minor), and a concrete fix.
If no issues, say so clearly.

Diff versus {base}:

```diff
{diff}
```"#
    )
}

fn run_review(prompt: &str) -> Result<String> {
    // Stream output live to the terminal; capture it for logging.
    // `claude -p <prompt>` sends a single non-interactive query.
    let output = Command::new("claude")
        .args(["-p", prompt])
        .output()
        .context("failed to run 'claude' — is it on PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("claude exited with error:\n{}", stderr.trim());
    }

    let result = String::from_utf8_lossy(&output.stdout).to_string();
    // Print to terminal (claude -p doesn't stream, so we print after completion).
    print!("{result}");
    Ok(result)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let args = Args::parse();

    let diff = get_diff(&args.base)?;
    if diff.trim().is_empty() {
        println!("No changes versus {} — nothing to review.", args.base);
        return Ok(());
    }

    let sha = git_short_sha()?;
    println!("Reviewing diff vs {} @ {}...\n", args.base, sha);

    let run_id = Local::now().to_rfc3339();
    // Best-effort telemetry — don't fail the review if logging breaks.
    let _ = log_start(&run_id, &args.base);

    let prompt = build_prompt(&diff, &args.base);
    let start = Instant::now();

    let result = run_review(&prompt);
    let elapsed = start.elapsed().as_secs_f64();

    let output = result.unwrap_or_else(|e| {
        eprintln!("error: review failed: {e}");
        String::new()
    });

    let _ = log_complete(&run_id, &args.base, &output, elapsed);

    if !output.is_empty() {
        match save_commit_log(&sha, &output, &args.base) {
            Ok(path) => println!("\nLogged to: {}", path.display()),
            Err(e) => eprintln!("warn: could not save commit log: {e}"),
        }
    }

    Ok(())
}
