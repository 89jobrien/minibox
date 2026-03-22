#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
# ]
# ///
"""Pre-push sync check — fetch, assess, and attempt to resolve basic conflicts vs origin/main.

Determines whether the local branch is ahead, behind, or diverged from origin/main.
If diverged or behind, attempts a rebase and resolves obvious conflicts autonomously.
Exits 0 if clean to push, 1 if manual intervention is required.

Usage:
  ./scripts/sync-check.py              # check and attempt auto-resolve
  ./scripts/sync-check.py --dry-run    # report only, no changes

As a git pre-push hook:
  echo 'uv run scripts/sync-check.py' > .git/hooks/pre-push
  chmod +x .git/hooks/pre-push
"""

import argparse
import asyncio
import json
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

from claude_agent_sdk import ClaudeAgentOptions, query

_LOG_FILE = Path.home() / ".mbx" / "agent-runs.jsonl"


def _log_start(script: str, args: dict) -> str:
    run_id = datetime.now().isoformat()
    _LOG_FILE.parent.mkdir(parents=True, exist_ok=True)
    with _LOG_FILE.open("a") as f:
        f.write(json.dumps({"run_id": run_id, "script": script, "args": args, "status": "running"}) + "\n")
    return run_id


def _log_complete(run_id: str, script: str, args: dict, output: str, duration_s: float) -> None:
    with _LOG_FILE.open("a") as f:
        f.write(json.dumps({
            "run_id": run_id, "script": script, "args": args,
            "status": "complete", "duration_s": round(duration_s, 2), "output": output,
        }) + "\n")


def run(cmd: list[str]) -> tuple[int, str, str]:
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode, r.stdout.strip(), r.stderr.strip()


def get_sync_status() -> dict:
    _, branch, _ = run(["git", "rev-parse", "--abbrev-ref", "HEAD"])
    _, ahead, _ = run(["git", "rev-list", "origin/main..HEAD", "--count"])
    _, behind, _ = run(["git", "rev-list", "HEAD..origin/main", "--count"])
    _, local_sha, _ = run(["git", "rev-parse", "HEAD"])
    _, remote_sha, _ = run(["git", "rev-parse", "origin/main"])
    _, status, _ = run(["git", "status", "--short"])
    _, stash, _ = run(["git", "stash", "list"])
    return {
        "branch": branch,
        "ahead": int(ahead or 0),
        "behind": int(behind or 0),
        "local_sha": local_sha[:8],
        "remote_sha": remote_sha[:8],
        "dirty": bool(status.strip()),
        "status": status,
        "stash": stash,
    }


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true", help="Report only, make no changes")
    parser.add_argument("--base", default="origin/main", help="Remote ref to check against (default: origin/main)")
    args = parser.parse_args()

    print("\nSync check...\n")

    # Always fetch first
    code, _, err = run(["git", "fetch", "origin"])
    if code != 0:
        print(f"Warning: fetch failed — {err}. Working with cached remote state.")

    status = get_sync_status()
    ahead, behind = status["ahead"], status["behind"]

    print(f"  Branch:  {status['branch']}")
    print(f"  Local:   {status['local_sha']}  ({ahead} ahead of {args.base})")
    print(f"  Remote:  {status['remote_sha']}  ({behind} behind)")
    if status["dirty"]:
        print(f"  Working tree: dirty")
    print()

    # Fast-path: nothing to do
    if behind == 0 and not status["dirty"]:
        print("Clean — nothing behind, working tree clean. Safe to push.")
        return

    if args.dry_run:
        if behind > 0:
            print(f"Dry run: would attempt rebase onto {args.base} ({behind} commit(s) behind).")
        sys.exit(0)

    # Build prompt with full context
    prompt = f"""You are a git sync agent for the minibox Rust repository.

Current state:
- Branch: {status['branch']}
- Local HEAD: {status['local_sha']} ({ahead} commit(s) ahead of {args.base})
- Remote HEAD: {status['remote_sha']} ({behind} commit(s) behind {args.base})
- Working tree: {'dirty' if status['dirty'] else 'clean'}
{('- Uncommitted changes:\n' + status['status']) if status['dirty'] else ''}
{('- Stashes: ' + status['stash']) if status['stash'] else ''}

Your task:
1. If the working tree is dirty, stash uncommitted changes first (`git stash`).
2. Rebase local commits onto {args.base} (`git rebase {args.base}`).
3. If the rebase succeeds cleanly, pop any stash and report success.
4. If the rebase produces conflicts:
   a. Run `git diff --name-only --diff-filter=U` to list conflicted files.
   b. For each conflicted file, read its contents and assess the conflict.
   c. Resolve conflicts you are confident about:
      - `Cargo.lock`: always accept the incoming (theirs) version — run `git checkout --theirs Cargo.lock && cargo generate-lockfile` if cargo is available, otherwise `git checkout --theirs Cargo.lock`.
      - Files where only one side made changes (the other is the common ancestor): accept the changed version.
      - Additive conflicts (both sides added different things, no overlap): merge both additions manually.
   d. After resolving a file, stage it with `git add <file>`.
   e. Continue the rebase with `git rebase --continue`.
5. If a conflict is ambiguous or involves overlapping logic changes, use AskUserQuestion to
   show the conflicting sections and ask the user how to resolve it before proceeding.
   Only abort if the user explicitly says to abort or cannot be reached.
6. Report the final state: what was resolved automatically, what (if anything) requires manual intervention, and whether it is safe to push.

Rules:
- Do NOT force-push.
- Do NOT amend commits.
- Do NOT resolve conflicts in Rust source files with complex logic changes — abort and report those.
- Be conservative: if in doubt, abort rather than produce a broken merge.
- Always verify `git status` after each step."""

    run_id = _log_start("sync-check", {"dry_run": args.dry_run, "base": args.base})
    start = time.monotonic()
    output_parts: list[str] = []

    async for message in query(
        prompt=prompt,
        options=ClaudeAgentOptions(
            allowed_tools=["Bash", "Read", "Edit", "AskUserQuestion"],
            permission_mode="acceptEdits",
        ),
    ):
        if hasattr(message, "result"):
            print(message.result)
            output_parts.append(message.result)

    full_output = "\n".join(output_parts)
    _log_complete(run_id, "sync-check", {"dry_run": args.dry_run, "base": args.base},
                  full_output, time.monotonic() - start)

    # Exit 1 if agent flagged that manual intervention is needed
    lowered = full_output.lower()
    if any(phrase in lowered for phrase in [
        "manual intervention", "cannot be resolved", "needs manual", "aborted",
        "abort", "not safe to push", "requires manual",
    ]):
        sys.exit(1)


if __name__ == "__main__":
    asyncio.run(main())
