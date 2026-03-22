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
import subprocess
import sys

from claude_agent_sdk import ClaudeAgentOptions, query


def get_diff(base: str) -> str:
    # Try branch diff first (committed changes ahead of base)
    result = subprocess.run(
        ["git", "diff", f"{base}...HEAD"],
        capture_output=True,
        text=True,
        check=True,
    )
    if result.stdout.strip():
        return result.stdout
    # Fall back to uncommitted changes (staged + unstaged vs HEAD)
    result = subprocess.run(
        ["git", "diff", "HEAD"],
        capture_output=True,
        text=True,
        check=True,
    )
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

    async for message in query(
        prompt=prompt,
        options=ClaudeAgentOptions(
            allowed_tools=["Read", "Glob", "Grep"],
            permission_mode="default",
        ),
    ):
        if hasattr(message, "result"):
            print(message.result)


if __name__ == "__main__":
    asyncio.run(main())
