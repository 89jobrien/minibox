#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
# ]
# ///
"""Pre-push AI code review — security and correctness focused for minibox."""

import argparse
import asyncio
import json
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


def get_diff(base: str) -> str:
    result = subprocess.run(["git", "diff", f"{base}...HEAD"], capture_output=True, text=True, check=True)
    if result.stdout.strip():
        return result.stdout
    result = subprocess.run(["git", "diff", "HEAD"], capture_output=True, text=True, check=True)
    return result.stdout


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="main", help="Base branch/ref (default: main)")
    args = parser.parse_args()

    diff = get_diff(args.base)
    if not diff.strip():
        print(f"No changes versus {args.base} — nothing to review.")
        sys.exit(0)

    prompt = f"""Review this diff for the minibox project — a Linux container runtime in Rust.

Focus on:
- **Security**: path traversal, symlink attacks, tar extraction safety, socket auth bypass
- **Correctness**: cgroup v2 semantics, namespace/clone flag usage, pivot_root ordering,
  overlay mount flags, pipe fd handling across clone()
- **Protocol**: breaking changes to JSON-over-newline types in protocol.rs
- **Unsafe blocks**: soundness, missing invariant comments
- **Error handling**: silent failures in container init (post-fork context — no unwrap)

For each issue: file + line, severity (critical/major/minor), and a concrete fix.
If no issues, say so clearly.

```diff
{diff}
```"""

    sha = agent_log.git_short_sha()
    print(f"Reviewing diff vs {args.base} @ {sha}...\n")

    run_id = agent_log.log_start("ai-review", {"base": args.base})
    start = time.monotonic()
    output_parts: list[str] = []

    async for message in query(
        prompt=prompt,
        options=ClaudeAgentOptions(
            allowed_tools=["Read", "Glob", "Grep"],
            permission_mode="default",
        ),
    ):
        if hasattr(message, "result"):
            print(message.result)
            output_parts.append(message.result)

    full_output = "\n".join(output_parts)
    agent_log.log_complete(run_id, "ai-review", {"base": args.base}, full_output, time.monotonic() - start)
    log_path = agent_log.save_commit_log(sha, "ai-review", full_output, {
        "base": args.base,
        "date": datetime.now().strftime("%Y-%m-%d %H:%M"),
    })
    print(f"\nLogged to: {log_path}")


if __name__ == "__main__":
    asyncio.run(main())
