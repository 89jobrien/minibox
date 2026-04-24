#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! anyhow = "1"
//! clap = { version = "4", features = ["derive"] }
//! ```

//! Pre-push sync check: fetch origin, detect ahead/behind/diverged, report status.
//! Exits 0 if safe to push, 1 if manual intervention needed.

use anyhow::{Context, Result};
use clap::Parser;
use std::process::Command;

#[derive(Parser)]
#[command(about = "Pre-push sync check — detect divergence from remote before pushing")]
struct Args {
    /// Report only, make no changes (always true — this script never mutates)
    #[arg(long)]
    dry_run: bool,

    /// Remote ref to check against
    #[arg(long, default_value = "origin/main")]
    base: String,
}

#[derive(Debug)]
enum SyncState {
    UpToDate,
    AheadOnly { ahead: usize },
    BehindOnly { behind: usize },
    Diverged { ahead: usize, behind: usize },
}

fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("git {}", args.join(" ")))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_ok(args: &[&str]) -> Result<bool> {
    Ok(Command::new("git")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?
        .success())
}

fn count_commits(range: &str) -> Result<usize> {
    let out = git(&["rev-list", "--count", range])?;
    Ok(out.parse().unwrap_or(0))
}

fn sync_state(base: &str) -> Result<SyncState> {
    let ahead = count_commits(&format!("{base}..HEAD"))?;
    let behind = count_commits(&format!("HEAD..{base}"))?;
    Ok(match (ahead, behind) {
        (0, 0) => SyncState::UpToDate,
        (a, 0) => SyncState::AheadOnly { ahead: a },
        (0, b) => SyncState::BehindOnly { behind: b },
        (a, b) => SyncState::Diverged { ahead: a, behind: b },
    })
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Fetch so remote ref is current
    print!("fetching origin... ");
    if git_ok(&["fetch", "origin", "--quiet"])? {
        println!("ok");
    } else {
        println!("WARN: fetch failed (offline?) — using cached remote state");
    }

    // Check if base ref exists
    if !git_ok(&["rev-parse", "--verify", "--quiet", &args.base])? {
        println!("WARN: base ref '{}' not found — skipping divergence check", args.base);
        println!("status: unknown (base ref missing)");
        return Ok(());
    }

    let state = sync_state(&args.base)?;

    match &state {
        SyncState::UpToDate => {
            println!("status: up to date with {}", args.base);
            println!("safe to push: YES");
        }
        SyncState::AheadOnly { ahead } => {
            println!("status: ahead of {} by {} commit(s)", args.base, ahead);
            println!("safe to push: YES");
        }
        SyncState::BehindOnly { behind } => {
            println!("status: behind {} by {} commit(s) — integrate before pushing", args.base, behind);
            println!("safe to push: NO");
            println!();
            println!("commits on remote you don't have:");
            let log = git(&["log", "--oneline", &format!("HEAD..{}", args.base)])?;
            for line in log.lines() {
                println!("  {line}");
            }
            println!();
            println!("fix: git merge {}", args.base);
            std::process::exit(1);
        }
        SyncState::Diverged { ahead, behind } => {
            println!(
                "status: diverged — {} commit(s) ahead, {} commit(s) behind {}",
                ahead, behind, args.base
            );
            println!("safe to push: NO");
            println!();
            println!("your commits not on remote:");
            let yours = git(&["log", "--oneline", &format!("{}..HEAD", args.base)])?;
            for line in yours.lines() {
                println!("  {line}");
            }
            println!();
            println!("remote commits you don't have:");
            let theirs = git(&["log", "--oneline", &format!("HEAD..{}", args.base)])?;
            for line in theirs.lines() {
                println!("  {line}");
            }
            println!();
            println!("fix: git merge {} (or see /sync-check skill)", args.base);
            std::process::exit(1);
        }
    }

    Ok(())
}
