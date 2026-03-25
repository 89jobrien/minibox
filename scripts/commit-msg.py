#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
# ]
# ///
"""Generate a commit message from staged changes and repo context.

Reads staged diff, recent commit history (for style), and branch name,
then proposes a conventional commit message. Optionally stages all and commits.
"""

import argparse
import asyncio
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

import sys as _sys
import os as _os
_sys.path.insert(0, _os.path.dirname(__file__))
import agent_log

_os.environ.pop("CLAUDECODE", None)
if _os.environ.get("ANTHROPIC_API_KEY", "").startswith("op://"):
    _os.environ.pop("ANTHROPIC_API_KEY")

from claude_agent_sdk import ClaudeAgentOptions, query

_MAX_DIFF_BYTES = 64 * 1024  # 64 KB — fall back to stat summary above this


def run(cmd: list[str]) -> str:
    return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()


def get_context(stage_all: bool) -> dict:
    if stage_all:
        subprocess.run(["git", "add", "-A"], check=True)

    staged_diff_full = run(["git", "diff", "--cached"])
    staged_stat = run(["git", "diff", "--cached", "--stat"])
    if len(staged_diff_full.encode()) > _MAX_DIFF_BYTES:
        staged_diff = f"(diff too large — {len(staged_diff_full.encode()) // 1024} KB; using stat only)\n{staged_stat}"
    else:
        staged_diff = staged_diff_full
    unstaged_stat = run(["git", "diff", "--stat"])
    branch = run(["git", "rev-parse", "--abbrev-ref", "HEAD"])
    recent_log = run(["git", "log", "-8", "--oneline"])
    status = run(["git", "status", "--short"])

    return {
        "branch": branch,
        "staged_diff": staged_diff,
        "staged_stat": staged_stat,
        "unstaged_stat": unstaged_stat,
        "recent_log": recent_log,
        "status": status,
        "has_staged": bool(staged_diff_full.strip()),
    }


async def generate_message(ctx: dict) -> str:
    parts: list[str] = []
    async for message in query(
        prompt=(
            "Generate a git commit message for the staged changes below.\n\n"
            "Rules:\n"
            "- Follow the existing commit style shown in the recent log\n"
            "- Use conventional commits format: `type(scope): description`\n"
            "  Types: feat, fix, docs, refactor, test, chore, perf, ci\n"
            "  Scope: crate name, module, or area (e.g. linuxbox, standup, justfile)\n"
            "- First line: ≤72 chars, imperative mood, no period\n"
            "- If the change warrants it, add a blank line then a short body (2–4 lines max)\n"
            "- Do NOT add 'Co-Authored-By' lines — those are added separately\n"
            "- Output ONLY the commit message, nothing else — no explanation, no markdown fences\n\n"
            f"Branch: {ctx['branch']}\n\n"
            f"Recent commits (style reference):\n{ctx['recent_log']}\n\n"
            f"Staged changes ({ctx['staged_stat'] or 'none'}):\n"
            f"```diff\n{ctx['staged_diff'] or '(nothing staged)'}\n```\n\n"
            + (f"Unstaged (not included):\n{ctx['unstaged_stat']}\n" if ctx["unstaged_stat"] else "")
        ),
        options=ClaudeAgentOptions(
            allowed_tools=["Read", "Glob", "Grep"],
            permission_mode="default",
        ),
    ):
        if hasattr(message, "result"):
            parts.append(message.result)
    return "\n".join(parts).strip()


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("-a", "--stage", action="store_true",
                        help="Stage all changes (git add -A) before generating")
    parser.add_argument("-c", "--commit", action="store_true",
                        help="Commit with the generated message after confirming")
    parser.add_argument("-y", "--yes", action="store_true",
                        help="Skip confirmation and commit immediately (implies --commit)")
    args = parser.parse_args()

    if args.yes:
        args.commit = True

    ctx = get_context(args.stage)

    if not ctx["has_staged"]:
        if ctx["status"]:
            print("Nothing staged. Use -a to stage all, or `git add` files first.")
        else:
            print("Working tree is clean — nothing to commit.")
        sys.exit(1)

    print(f"Generating commit message for {ctx['staged_stat']}...\n")

    run_id = agent_log.log_start("commit-msg", {"stage": args.stage, "commit": args.commit})
    start = time.monotonic()

    msg = await generate_message(ctx)

    print("─" * 60)
    print(msg)
    print("─" * 60)

    if args.commit:
        if not args.yes:
            if not sys.stdin.isatty():
                print("\nNon-interactive session — use -y to commit without confirmation.")
                agent_log.log_complete(run_id, "commit-msg", {"stage": args.stage, "commit": False},
                              msg, time.monotonic() - start)
                sys.exit(0)
            print("\nCommit with this message? [y/N] ", end="", flush=True)
            answer = input().strip().lower()
            if answer != "y":
                print("Aborted.")
                agent_log.log_complete(run_id, "commit-msg", {"stage": args.stage, "commit": False},
                              msg, time.monotonic() - start)
                sys.exit(0)

        full_msg = msg + "\n\nCo-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
        result = subprocess.run(["git", "commit", "-m", full_msg])
        if result.returncode == 0:
            print("\nCommitted.")
        else:
            print("\nCommit failed — check git output above.")
            sys.exit(1)

    agent_log.log_complete(run_id, "commit-msg", {"stage": args.stage, "commit": args.commit},
                  msg, time.monotonic() - start)


if __name__ == "__main__":
    asyncio.run(main())
