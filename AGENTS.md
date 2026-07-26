# AGENTS.md

Agent-specific guidance for minibox. Claude Code users should read
`CLAUDE.md` first — this file covers subagent conventions, Codex
integration, and persistent context.

## Memory Bank

Persistent context lives in `.ctx/memory-bank/`. Read before
substantive work; update after milestones.

| File                        | Purpose                          |
| --------------------------- | -------------------------------- |
| `projectbrief.mbx.md`      | Scope, non-goals, success criteria |
| `productContext.mbx.md`    | Problem, users, UX principles    |
| `systemPatterns.mbx.md`    | Crate layout, data flow, conventions |
| `techContext.mbx.md`       | Stack, env vars, build commands  |
| `activeContext.mbx.md`     | Current focus and in-progress work |
| `progress.mbx.md`          | What works, backlog, known issues |

Context layers (read deeper after foundations):
**projectbrief** -> **productContext** / **systemPatterns** / **techContext**
-> **activeContext** -> **progress**.

## Subagent Rules

- Each subagent must run `git branch --show-current` before every commit.
  If the answer is `main`, STOP — do not commit to main directly.
- Never use `--no-verify` on git commits.
- Worktree subagents MUST merge their branch back and remove the worktree
  before reporting done. An orphaned worktree means the task is incomplete.
- After subagents complete, verify their changes were committed
  (`git log --oneline -3`). A HANDOFF with `commits: []` is incomplete.
- Cap parallel subagents at 5 concurrent to avoid API rate limits.
- If tests fail, debug and retry up to 3 times before escalating.

## Codex / Cargo AI

This repo does not use Cargo AI agent definitions. If Codex is used as
the coding agent, it should follow the same phased workflow described in
`CLAUDE.md` (ORIENT -> PLAN -> ACT -> VERIFY -> SHIP) and read memory-bank
files before substantive work.

## Task Graph

Tasks live in `.ctx/GODMODE.tasks.yaml` (gitignored). Use `godmode task`
CLI for state transitions. Independent chains can run in parallel via
`godmode:parallel-agents`. A task is runnable when all `depends_on` items
are `done`.
