#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
# ]
# ///
"""Generate unit tests for a new minibox domain trait adapter."""

import argparse
import asyncio
import json
import time
from datetime import datetime
from pathlib import Path

from claude_agent_sdk import ClaudeAgentOptions, query

_LOG_FILE = Path.home() / ".mbx" / "agent-runs.jsonl"

TRAIT_HINTS = {
    "BridgeNetworking": "crates/minibox-lib/src/adapters/",
    "PseudoTerminal": "crates/minibox-lib/src/adapters/",
    "ContainerExec": "crates/minibox-lib/src/adapters/",
    "LogStore": "crates/minibox-lib/src/adapters/",
    "StateStore": "crates/minibox-lib/src/adapters/",
}


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


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trait", help="Domain trait to generate tests for (e.g. BridgeNetworking)")
    parser.add_argument("--output", help="Output file path (default: Claude decides)", default=None)
    args = parser.parse_args()

    output_hint = f"\nWrite the tests to: {args.output}" if args.output else ""
    adapter_dir = TRAIT_HINTS.get(args.trait, "crates/minibox-lib/src/adapters/")

    prompt = f"""Generate unit tests for a new `{args.trait}` adapter in minibox-lib.

Steps:
1. Read `crates/minibox-lib/src/domain.rs` to understand the `{args.trait}` trait definition
2. Read `crates/minibox-lib/src/adapters/mocks.rs` to understand the mock adapter pattern
3. Read 2-3 existing test modules (e.g. in `crates/minibox-lib/src/container/` or
   `crates/minibox-lib/src/image/`) to match the project's test style
4. Generate a complete test module for a `Mock{args.trait}` adapter that:
   - Implements `{args.trait}` from domain.rs
   - Covers the happy path, error conditions, and edge cases
   - Follows the AAA pattern (Arrange / Act / Assert)
   - Uses `#[tokio::test]` for async tests
   - Matches existing naming conventions exactly
5. Write the tests to `{adapter_dir}` alongside the existing adapters{output_hint}

Do not invent trait methods — only implement what is defined in domain.rs."""

    print(f"Generating tests for {args.trait}...\n")

    run_id = _log_start("gen-tests", {"trait": args.trait})
    start = time.monotonic()
    output_parts: list[str] = []

    async for message in query(
        prompt=prompt,
        options=ClaudeAgentOptions(
            allowed_tools=["Read", "Glob", "Grep", "Write"],
            permission_mode="acceptEdits",
        ),
    ):
        if hasattr(message, "result"):
            print(message.result)
            output_parts.append(message.result)

    _log_complete(run_id, "gen-tests", {"trait": args.trait}, "\n".join(output_parts), time.monotonic() - start)


if __name__ == "__main__":
    asyncio.run(main())
