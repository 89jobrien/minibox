//! `cargo xtask daily-orchestration` — headless daily workflow runner.
//!
//! Local mode keeps the existing all-repo daily workflow behavior.
//! CI mode is single-repo scoped and configured from environment variables:
//!
//! - `ORCH_REPOSITORY`, falling back to `GITHUB_REPOSITORY`
//! - `ORCH_BRANCH`, falling back to `GITHUB_REF_NAME`
//! - `ORCH_OUTPUT`: `step-summary`, `issue`, or `pull-request`
//! - `ORCH_CREATE_ISSUES`: boolean, default `true`
//! - `ORCH_CREATE_PR`: boolean, default `false`
//! - `ORCH_ALLOW_DIRECT_PUSH`: boolean, default `false`
//! - `ORCH_CLAUDE_BIN`: default `claude`
//! - `ORCH_ALLOWED_TOOLS`: default `Bash,Read,Write,Edit,Glob,Grep,Agent`

use anyhow::{Context, Result, bail};
use chrono::Local;
use std::{env, fmt, process::Command};

const ALLOWED_TOOLS: &str = "Bash,Read,Write,Edit,Glob,Grep,Agent";
const LOCAL_PROMPT: &str = "Run the daily orchestration workflow across all repos. Follow the /daily-orchestration skill exactly: pull all repos, analyze health, fix P0s, write Obsidian daily note, push. Do not prompt for confirmation — this is a non-interactive run.";

pub fn run(dry_run: bool, ci: bool) -> Result<()> {
    let config = if ci {
        RunConfig::ci_from_env()?
    } else {
        RunConfig::local()
    };
    let start_timestamp = timestamp();
    eprintln!(
        "[daily-orchestration] Starting at {start_timestamp} ({})",
        config.mode_name()
    );

    if dry_run {
        config.print_dry_run();
        return Ok(());
    }

    let output = Command::new(config.claude_bin())
        .arg("--allowedTools")
        .arg(config.allowed_tools())
        .arg("-p")
        .arg(config.prompt())
        .output()
        .context("failed to launch `claude`; ensure the Claude CLI is in PATH and authenticated")?;

    let end_timestamp = timestamp();

    if !output.status.success() {
        eprintln!("[daily-orchestration] FAILED at {end_timestamp}");
        print_output(&output.stdout);
        print_error(&output.stderr);
        bail!("daily orchestration failed with status {}", output.status);
    }

    print_output(&output.stdout);
    print_error(&output.stderr);
    eprintln!("[daily-orchestration] Completed at {end_timestamp}");
    Ok(())
}

enum RunConfig {
    Local {
        claude_bin: String,
        allowed_tools: String,
    },
    Ci(CiConfig),
}

struct CiConfig {
    repository: String,
    branch: Option<String>,
    output: CiOutput,
    create_issues: bool,
    create_pr: bool,
    allow_direct_push: bool,
    claude_bin: String,
    allowed_tools: String,
    token_env: Option<&'static str>,
}

#[derive(Clone, Copy)]
enum CiOutput {
    StepSummary,
    Issue,
    PullRequest,
}

impl RunConfig {
    fn local() -> Self {
        Self::Local {
            claude_bin: env_or_default("ORCH_CLAUDE_BIN", "claude"),
            allowed_tools: env_or_default("ORCH_ALLOWED_TOOLS", ALLOWED_TOOLS),
        }
    }

    fn ci_from_env() -> Result<Self> {
        let repository = env_optional("ORCH_REPOSITORY")
            .or_else(|| env_optional("GITHUB_REPOSITORY"))
            .context("CI mode requires ORCH_REPOSITORY or GITHUB_REPOSITORY")?;
        let branch = env_optional("ORCH_BRANCH").or_else(|| env_optional("GITHUB_REF_NAME"));
        let output = env_optional("ORCH_OUTPUT")
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(CiOutput::StepSummary);
        let create_issues = env_bool("ORCH_CREATE_ISSUES", true)?;
        let create_pr = env_bool("ORCH_CREATE_PR", false)?;
        let allow_direct_push = env_bool("ORCH_ALLOW_DIRECT_PUSH", false)?;
        let claude_bin = env_or_default("ORCH_CLAUDE_BIN", "claude");
        let allowed_tools = env_or_default("ORCH_ALLOWED_TOOLS", ALLOWED_TOOLS);
        let token_env = if env_optional("GH_TOKEN").is_some() {
            Some("GH_TOKEN")
        } else if env_optional("GITHUB_TOKEN").is_some() {
            Some("GITHUB_TOKEN")
        } else {
            None
        };

        Ok(Self::Ci(CiConfig {
            repository,
            branch,
            output,
            create_issues,
            create_pr,
            allow_direct_push,
            claude_bin,
            allowed_tools,
            token_env,
        }))
    }

    fn mode_name(&self) -> &'static str {
        match self {
            Self::Local { .. } => "local",
            Self::Ci(_) => "ci",
        }
    }

    fn claude_bin(&self) -> &str {
        match self {
            Self::Local { claude_bin, .. } => claude_bin,
            Self::Ci(config) => &config.claude_bin,
        }
    }

    fn allowed_tools(&self) -> &str {
        match self {
            Self::Local { allowed_tools, .. } => allowed_tools,
            Self::Ci(config) => &config.allowed_tools,
        }
    }

    fn prompt(&self) -> String {
        match self {
            Self::Local { .. } => LOCAL_PROMPT.to_string(),
            Self::Ci(config) => config.prompt(),
        }
    }

    fn print_dry_run(&self) {
        eprintln!("[daily-orchestration] DRY RUN - would invoke:");
        eprintln!(
            "  {} --allowedTools {:?} -p <{} prompt>",
            self.claude_bin(),
            self.allowed_tools(),
            self.mode_name()
        );

        if let Self::Ci(config) = self {
            eprintln!("[daily-orchestration] CI config:");
            eprintln!("  repository: {}", config.repository);
            eprintln!(
                "  branch: {}",
                config.branch.as_deref().unwrap_or("<unset>")
            );
            eprintln!("  output: {}", config.output);
            eprintln!("  create issues: {}", config.create_issues);
            eprintln!("  create PR: {}", config.create_pr);
            eprintln!("  allow direct push: {}", config.allow_direct_push);
            eprintln!(
                "  token env present: {}",
                config.token_env.unwrap_or("<none>")
            );
        }
    }
}

impl CiConfig {
    fn prompt(&self) -> String {
        let branch = self
            .branch
            .as_deref()
            .map_or("the current checked-out ref".to_string(), |branch| {
                format!("the `{branch}` ref")
            });
        let issue_policy = if self.create_issues {
            "Create or update GitHub issues for concrete P0/P1 findings when useful."
        } else {
            "Do not create GitHub issues; include findings in the configured summary output only."
        };
        let pr_policy = if self.create_pr {
            "If you make code changes, create a branch and open a pull request for review."
        } else {
            "Do not make code changes or open pull requests; report recommended fixes only."
        };
        let push_policy = if self.allow_direct_push {
            "Direct pushes are allowed only when the repository permissions and branch protection allow them."
        } else {
            "Do not push directly to the default or protected branch."
        };
        let token_policy = match self.token_env {
            Some(name) => format!("Use GitHub authentication from the `{name}` environment variable."),
            None => "No GitHub token environment variable was detected; use read-only local checkout operations unless authentication is already configured.".to_string(),
        };

        format!(
            "\
Run a CI-compatible daily orchestration workflow for exactly one repository: `{repository}`.

This is a single-repo GitHub Actions run. Configuration comes from environment variables, not local machine paths.

Scope and safety rules:
- Operate only on `{repository}` and the current CI checkout for {branch}.
- Do not inspect, clone, pull, or modify any other repository.
- Do not use local workstation paths such as `/Users`, `~/dev`, Obsidian vaults, dotfiles, `~/.secrets`, or other home-directory state.
- Do not invoke 1Password, `op`, `op run`, env-file secret loading, or any local secret manager.
- {token_policy}
- {push_policy}
- Do not prompt for confirmation.

Workflow:
- Inspect this repository's recent commits, open issues, pull requests, and GitHub Actions health.
- Identify P0/P1 failures, security issues, and regressions for this repository only.
- {issue_policy}
- {pr_policy}
- Write the final daily summary to {output}.
- Also print the final summary to stdout for the workflow log.
",
            repository = self.repository,
            branch = branch,
            token_policy = token_policy,
            push_policy = push_policy,
            issue_policy = issue_policy,
            pr_policy = pr_policy,
            output = self.output.instruction(),
        )
    }
}

impl CiOutput {
    fn instruction(self) -> &'static str {
        match self {
            Self::StepSummary => {
                "the file named by `GITHUB_STEP_SUMMARY` when it is set; otherwise stdout"
            }
            Self::Issue => "a GitHub issue in the target repository",
            Self::PullRequest => "the pull request body when a PR is created; otherwise stdout",
        }
    }
}

impl fmt::Display for CiOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StepSummary => f.write_str("step-summary"),
            Self::Issue => f.write_str("issue"),
            Self::PullRequest => f.write_str("pull-request"),
        }
    }
}

impl std::str::FromStr for CiOutput {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "step-summary" => Ok(Self::StepSummary),
            "issue" => Ok(Self::Issue),
            "pull-request" => Ok(Self::PullRequest),
            other => bail!(
                "invalid ORCH_OUTPUT {other:?}; expected step-summary, issue, or pull-request"
            ),
        }
    }
}

fn env_optional(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_or_default(name: &str, default: &str) -> String {
    env_optional(name).unwrap_or_else(|| default.to_string())
}

fn env_bool(name: &str, default: bool) -> Result<bool> {
    let Some(value) = env_optional(name) else {
        return Ok(default);
    };

    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("invalid boolean value for {name}: {value:?}"),
    }
}

fn timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn print_output(bytes: &[u8]) {
    if !bytes.is_empty() {
        print!("{}", String::from_utf8_lossy(bytes));
    }
}

fn print_error(bytes: &[u8]) {
    if !bytes.is_empty() {
        eprint!("{}", String::from_utf8_lossy(bytes));
    }
}
