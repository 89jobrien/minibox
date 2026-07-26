# Cloude Code ToolBox — MCP & Skills awareness

_Generated: 2026-05-18T16:16:26.941753Z_

## How to use this report

- **Saved copy:** `.claude/cloude-code-toolbox-mcp-skills-awareness.md` — refreshed when you run **Scan MCP & Skills awareness** in IntelliJ.

---

## MCP — workspace

Workspace `/Users/joe/dev/minibox/.vscode/mcp.json` _(folder: minibox)_

- **/Users/joe/dev/minibox/.vscode/mcp.json** — _File exists — servers defined_

| Server id | Kind  | Detail                                     |
| --------- | ----- | ------------------------------------------ |
| github    | stdio | npx -y @modelcontextprotocol/server-github |
| personal  | stdio | /Users/joe/.local/bin/personal-mcp         |

## MCP — user profile

- **/Users/joe/Library/Application Support/Code/User/mcp.json** — _File exists — servers defined_

| Server id | Kind  | Detail                             |
| --------- | ----- | ---------------------------------- |
| personal  | stdio | /Users/joe/.local/bin/personal-mcp |

## Skills (local `SKILL.md` folders)

### Project-scoped

- **whatidid** — `/Users/joe/dev/minibox/.claude/skills/whatidid`
    - Generate a daily activity report from Claude Code session data. Harvests

- **wave-integration** — `/Users/joe/dev/minibox/.claude/skills/wave-integration`
    - Use when merging multiple parallel agent branches (waves) into a single integration

### User-scoped

- **agents-skill-save** — `/Users/joe/.agents/skills/agents-skill-save`
    - Use when creating a new local skill or fixing an existing skill that was saved to the wrong skills tree. Symptoms - a skill was written under ~/.claude/skills, the active setup uses ~/.agents/skills, or skill instruction

- **release-readiness-check** — `/Users/joe/.agents/skills/release-readiness-check`
    - Use before cutting a release to verify tags, affected crates, gates, binaries, and target remote. Symptoms - you are about to tag a release, create a GitHub release, or automate publishing and want to catch the obvious f

- **token-cost-optimizer** — `/Users/joe/.agents/skills/token-cost-optimizer`
    - Use when analyzing Claude or agent token usage, ccusage reports, output-to-input token ratios, model spend, or ways to lower AI coding costs. Symptoms - the user asks "why is Claude expensive", "analyze token usage", "re

- **godmode** — `/Users/joe/.agents/skills/godmode`
    - Use when the user asks to use godmode in Warp/Oz, wants the godmode task graph/session workflow, mentions godmode handon/handoff/dispatch/task-driven development, or needs parity with the local Claude Code godmode plugin

- **workspace-bump-commit** — `/Users/joe/.agents/skills/workspace-bump-commit`
    - Use when applying version bumps to selected Rust workspace crates and creating the release commit cleanly. Symptoms - you already know which crates should change, and you need to run `cargo set-version`, stage the right

- **rust-release-workflow-author** — `/Users/joe/.agents/skills/rust-release-workflow-author`
    - Use when creating or editing a GitHub Actions release workflow for a Rust workspace with version bumps, tags, affected-crate selection, binary packaging, and GitHub releases. Symptoms - you need a manual release workflow

- **notfiles-release-workflow** — `/Users/joe/.agents/skills/notfiles-release-workflow`
    - Use when applying the global Rust release-workflow pattern to the notfiles repo. Symptoms - you need the repo-specific release file, affected-crate exclusions, binary list, or GitHub-vs-Gitea cautions for notfiles.

- **repo-gap-backlog** — `/Users/joe/.agents/skills/repo-gap-backlog`
    - Use when a local project exists but does not yet have a GitHub repo or issue backlog, and you want to turn a completion review into concrete GitHub issues. Symptoms - local code is ahead of repo setup, there is no README

- **session-wrap-commit-push** — `/Users/joe/.agents/skills/session-wrap-commit-push`
    - Use when ending a coding session and you want to capture handoff state, commit all current changes, and push the branch. Symptoms - you finished a work slice, updated HANDOFF state, and want one repeatable closeout flow

- **baml-add-types** — `/Users/joe/.agents/skills/baml-add-types`
    - Use when adding new BAML types/functions to crux-agentic.

- **workspace-release-impact** — `/Users/joe/.agents/skills/workspace-release-impact`
    - Use when deciding which Rust workspace crates need a version bump from a set of changes. Symptoms - a shared crate changed, downstream crates may also need bumps, or a release process should version only affected package

- **dual-forge-pr-merge** — `/Users/joe/.agents/skills/dual-forge-pr-merge`
    - Use when a repo is mirrored across GitHub and Gitea and you need to decide where to open or merge a PR. Symptoms - the branch tracks one forge, another forge has a different `main`, or GitHub tooling is available but the

- **remote-upstream-triage** — `/Users/joe/.agents/skills/remote-upstream-triage`
    - Use when git push, PR creation, or merge flow fails because the current branch has no upstream, the wrong remote is selected, or remote branches have drifted across forges. Symptoms - `git push` says no configured destin

- **gh-bulk-issues** — `/Users/joe/.agents/skills/gh-bulk-issues`
    - Use when creating multiple GitHub issues from a structured

---

_Report from Cloude Code ToolBox (IntelliJ)._
