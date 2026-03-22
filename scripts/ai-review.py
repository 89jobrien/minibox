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
            "run_id": run_id,
            "script": script,
            "args": args,
            "status": "complete",
            "duration_s": round(duration_s, 2),
            "output": output,
        }) + "\n")


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

    print(f"Reviewing diff vs {args.base}...\n")

    run_id = _log_start("ai-review", {"base": args.base})
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

    _log_complete(run_id, "ai-review", {"base": args.base}, "\n".join(output_parts), time.monotonic() - start)


if __name__ == "__main__":
    asyncio.run(main())
