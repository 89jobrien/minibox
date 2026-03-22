#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
# ]
# ///
"""Automated standup — activity across all ~/dev/ repos for the last N hours.

Scans ~/dev/*/ for git repos with recent commits, collects activity per repo,
and generates a standup report. Optionally writes to an Obsidian vault.
"""

import argparse
import asyncio
import json
import subprocess
import time
from datetime import datetime
from pathlib import Path

from claude_agent_sdk import ClaudeAgentOptions, query

_LOG_FILE = Path.home() / ".mbx" / "agent-runs.jsonl"
_DEFAULT_REPOS_DIR = Path.home() / "dev"
_DEFAULT_VAULT = Path.home() / "Documents" / "Obsidian Vault" / "Reports"
_TIMELINE_FILE = "docs/STANDUP.md"


def find_repo_root() -> Path | None:
    """Walk up from cwd to find the git repo root."""
    p = Path.cwd()
    while p != p.parent:
        if (p / ".git").exists():
            return p
        p = p.parent
    return None


def append_to_timeline(standup_output: str, now: datetime) -> Path | None:
    """Append this standup's output to docs/TIMELINE.md if it exists in the repo."""
    root = find_repo_root()
    if root is None:
        return None
    timeline = root / _TIMELINE_FILE
    if not timeline.exists():
        return None
    entry = (
        f"\n## {now.strftime('%Y-%m-%d %H:%M')}\n\n"
        f"{standup_output.strip()}\n\n"
        f"---\n"
    )
    # Prepend after the header block (after the first ---)
    content = timeline.read_text()
    marker = "---\n"
    idx = content.find(marker)
    if idx != -1:
        timeline.write_text(content[: idx + len(marker)] + entry + content[idx + len(marker) :])
    else:
        timeline.write_text(content + entry)
    return timeline


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


def git(cmd: list[str], cwd: Path) -> str:
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd)
    return r.stdout.strip()


def find_active_repos(repos_dir: Path, hours: int) -> list[tuple[Path, dict]]:
    """Return repos under repos_dir with commits in the last N hours."""
    since = f"{hours} hours ago"
    active = []
    for candidate in sorted(repos_dir.iterdir()):
        if not (candidate / ".git").exists():
            continue
        # Timestamped commits: "ISO_DATETIME|SHORT_HASH|subject"
        commit_log = git(
            ["git", "log", f"--since={since}", "--all", "--format=%ai|%h|%s"],
            candidate,
        )
        if not commit_log:
            continue
        # Files actually touched by commits in the window (not diff against working tree)
        files_raw = git(
            ["git", "log", f"--since={since}", "--all", "--name-only", "--format="],
            candidate,
        )
        files = sorted(set(f for f in files_raw.splitlines() if f.strip()))
        branch = git(["git", "rev-parse", "--abbrev-ref", "HEAD"], candidate)
        status = git(["git", "status", "--short"], candidate)
        stash = git(["git", "stash", "list"], candidate)
        active.append((candidate, {
            "commit_log": commit_log,   # timestamped, one per line
            "files": files,
            "branch": branch,
            "status": status,
            "stash": stash,
        }))
    return active


def find_claude_sessions(hours: int) -> str:
    """Find recent Claude Code session excerpts."""
    sessions_dir = Path.home() / ".claude" / "projects"
    if not sessions_dir.exists():
        return ""
    cutoff = time.time() - (hours * 3600)
    recent: list[tuple[float, Path]] = []
    for jsonl in sessions_dir.rglob("*.jsonl"):
        try:
            if jsonl.stat().st_mtime > cutoff:
                recent.append((jsonl.stat().st_mtime, jsonl))
        except OSError:
            continue
    if not recent:
        return ""
    recent.sort(reverse=True)
    excerpts: list[str] = []
    for _, path in recent[:3]:
        try:
            lines = path.read_text(errors="replace").splitlines()
            msgs = []
            for line in lines[-60:]:
                try:
                    entry = json.loads(line)
                    if entry.get("type") == "assistant" and isinstance(entry.get("message"), dict):
                        content = entry["message"].get("content", "")
                        if isinstance(content, list):
                            for block in content:
                                if isinstance(block, dict) and block.get("type") == "text":
                                    text = block.get("text", "").strip()
                                    if text and len(text) > 20:
                                        msgs.append(text[:250])
                                        break
                        elif isinstance(content, str) and content.strip():
                            msgs.append(content.strip()[:250])
                except (json.JSONDecodeError, KeyError):
                    continue
            if msgs:
                excerpts.append(f"session ({path.parent.name[-8:]}):\n" + "\n".join(f"  - {m}" for m in msgs[-4:]))
        except OSError:
            continue
    return "\n\n".join(excerpts)


def build_repo_section(name: str, data: dict) -> str:
    commits = data["commit_log"].splitlines()
    files = data["files"]
    lines = [f"## {name}", f"{len(commits)} commit(s) · {len(files)} file(s) touched", ""]
    lines.append("Commits (timestamp | hash | subject):")
    for c in commits:
        lines.append(f"  {c}")
    if files:
        lines.append(f"\nFiles: {', '.join(files[:20])}")
    if data["status"]:
        lines.append(f"\nUncommitted:\n{data['status']}")
    if data["stash"]:
        lines.append(f"\nStashes:\n{data['stash']}")
    return "\n".join(lines)


async def generate_standup(repo_context: str, session_context: str, hours: int) -> str:
    session_section = f"\n\nRecent Claude session excerpts:\n{session_context}" if session_context else ""
    parts: list[str] = []
    async for message in query(
        prompt=(
            f"Generate a standup report for the last {hours}h structured as time blocks.\n\n"
            f"FORMAT — produce exactly these four sections:\n\n"
            f"## State at start\n"
            f"One short paragraph. Where were things at the beginning of this window — "
            f"what was the last stable state before this batch of work began? "
            f"Infer from the oldest commit in the window and the files touched.\n\n"
            f"## Timeline\n"
            f"Group commits into 1-hour calendar blocks (e.g. 09:00–10:00, 10:00–11:00).\n"
            f"Each block on one line: `HH:00–HH:00  description of work  (files: x, y, z)`\n"
            f"Oldest block first (chronological order). Use the actual commit timestamps provided.\n"
            f"If commits span multiple days, prefix each day's blocks with the date on its own line.\n"
            f"Summarise what was accomplished in each block — not just the commit subject verbatim.\n\n"
            f"## Current state + next\n"
            f"Two to three sentences. What is the current state of the codebase/work, "
            f"and what is the logical next step? Include any open stashes or in-flight threads.\n\n"
            f"## Concerns\n"
            f"Risks, drift, technical debt, or open questions worth flagging. "
            f"If none, say 'None.'\n\n"
            f"Rules: be terse, cite short hashes where useful, infer intent from commit messages.\n\n"
            f"--- REPOSITORY ACTIVITY ---\n{repo_context}"
            f"{session_section}"
        ),
        options=ClaudeAgentOptions(
            allowed_tools=["Read", "Glob", "Grep", "Bash"],
            permission_mode="default",
        ),
    ):
        if hasattr(message, "result"):
            parts.append(message.result)
    return "\n".join(parts)


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hours", type=int, default=24, help="Lookback window in hours (default: 24)")
    parser.add_argument("--repos-dir", type=Path, default=_DEFAULT_REPOS_DIR,
                        help=f"Root dir to scan for repos (default: {_DEFAULT_REPOS_DIR})")
    parser.add_argument("--vault", type=Path, default=None,
                        help="Write report to Obsidian vault dir (e.g. ~/Documents/Obsidian Vault/Reports)")
    parser.add_argument("--no-sessions", action="store_true", help="Skip Claude session log analysis")
    args = parser.parse_args()

    now = datetime.now()
    print(f"\nStandup — last {args.hours}h — {now.strftime('%Y-%m-%d %H:%M')}\n")

    # Collect repo activity
    active_repos = find_active_repos(args.repos_dir, args.hours)
    inactive = [
        d.name for d in sorted(args.repos_dir.iterdir())
        if (d / ".git").exists() and d not in [r for r, _ in active_repos]
    ] if args.repos_dir.exists() else []

    if not active_repos:
        print(f"No activity in {args.repos_dir} in the last {args.hours}h.")
        return

    print(f"Active repos ({len(active_repos)}): {', '.join(r.name for r, _ in active_repos)}\n")

    # Build context
    repo_sections = [build_repo_section(r.name, d) for r, d in active_repos]
    if inactive:
        repo_sections.append(f"_No activity in: {', '.join(inactive)}_")
    repo_context = "\n\n---\n\n".join(repo_sections)

    session_context = "" if args.no_sessions else find_claude_sessions(args.hours)

    run_id = _log_start("standup", {"hours": args.hours, "repos_dir": str(args.repos_dir)})
    start = time.monotonic()

    standup_output = await generate_standup(repo_context, session_context, args.hours)

    # Build full report
    frontmatter = (
        f"---\n"
        f"type: standup\n"
        f"date: {now.strftime('%Y-%m-%d')}\n"
        f"hour: \"{now.strftime('%H:00')}\"\n"
        f"repos_active: {len(active_repos)}\n"
        f"window_hours: {args.hours}\n"
        f"---\n\n"
    )
    header = f"# Standup — {now.strftime('%Y-%m-%d %H:%M')}\n\n_window: {args.hours}h_\n\n"
    full_report = frontmatter + header + standup_output + "\n\n---\n\n" + repo_context

    print(standup_output)

    # Append to docs/TIMELINE.md if present in the repo
    timeline_path = append_to_timeline(standup_output, now)
    if timeline_path:
        print(f"\nAppended to: {timeline_path.relative_to(Path.cwd())}")

    # Optionally write to Obsidian vault
    vault_dir = args.vault
    if vault_dir is None and _DEFAULT_VAULT.exists():
        vault_dir = _DEFAULT_VAULT

    if vault_dir is not None:
        vault_dir.mkdir(parents=True, exist_ok=True)
        filename = now.strftime("%Y-%m-%d %H:00") + ".md"
        out_path = vault_dir / filename
        out_path.write_text(full_report)
        print(f"\nWritten to: {out_path}")

    _log_complete(run_id, "standup", {"hours": args.hours},
                  full_report, time.monotonic() - start)


if __name__ == "__main__":
    asyncio.run(main())
