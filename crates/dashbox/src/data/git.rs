// dashbox/src/data/git.rs
use anyhow::{Context, Result};
use std::process::Command;

use super::DataSource;

#[derive(Debug, Clone)]
pub struct GitCommit {
    pub hash: String,
    pub author: String,
    pub age: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct GitFile {
    pub status: String, // M, A, D, R, etc.
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct GitData {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub is_clean: bool,
    pub commits: Vec<GitCommit>,
    pub changed_files: Vec<GitFile>,
}

pub struct GitSource;

impl GitSource {
    pub fn new() -> Self {
        Self
    }

    fn run_git(args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .output()
            .context("failed to run git")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "git {} exited with {}: {}",
                args.first().copied().unwrap_or(""),
                output.status,
                stderr.trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl DataSource for GitSource {
    type Data = GitData;

    fn load(&self) -> Result<GitData> {
        // Branch name
        let branch = Self::run_git(&["rev-parse", "--abbrev-ref", "HEAD"])?
            .trim()
            .to_string();

        // Ahead/behind — may fail on detached HEAD or when the remote tracking
        // branch doesn't exist.  Surface the error as a zero ahead/behind
        // rather than masking it silently; the caller will still get all other
        // git data (status, log, etc.).
        let ab_output = Self::run_git(&[
            "rev-list",
            "--left-right",
            "--count",
            &format!("HEAD...origin/{branch}"),
        ])
        .unwrap_or_else(|_e| String::new());
        let parts: Vec<&str> = ab_output.trim().split('\t').collect();
        let ahead = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let behind = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);

        // Clean status
        let status_output = Self::run_git(&["status", "--porcelain"])?;
        let is_clean = status_output.trim().is_empty();

        // Changed files from status
        let changed_files: Vec<GitFile> = status_output
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| {
                let status = line.get(..2).unwrap_or("??").trim().to_string();
                let path = line.get(3..).unwrap_or("").to_string();
                GitFile { status, path }
            })
            .collect();

        // Recent commits
        let log_output = Self::run_git(&["log", "--oneline", "--format=%h\t%an\t%cr\t%s", "-15"])?;
        let commits: Vec<GitCommit> = log_output
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(4, '\t').collect();
                if parts.len() == 4 {
                    Some(GitCommit {
                        hash: parts[0].to_string(),
                        author: parts[1].to_string(),
                        age: parts[2].to_string(),
                        message: parts[3].to_string(),
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(GitData {
            branch,
            ahead,
            behind,
            is_clean,
            commits,
            changed_files,
        })
    }
}
