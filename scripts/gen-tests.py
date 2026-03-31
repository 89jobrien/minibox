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
import os as _os
import sys as _sys
import time

_sys.path.insert(0, _os.path.dirname(__file__))
import agent_log

_os.environ.pop("CLAUDECODE", None)
if _os.environ.get("ANTHROPIC_API_KEY", "").startswith("op://"):
    _os.environ.pop("ANTHROPIC_API_KEY")

from claude_agent_sdk import ClaudeAgentOptions, query

TRAIT_HINTS = {
    "BridgeNetworking": "crates/mbx/src/adapters/",
    "PseudoTerminal": "crates/mbx/src/adapters/",
    "ContainerExec": "crates/mbx/src/adapters/",
    "LogStore": "crates/mbx/src/adapters/",
    "StateStore": "crates/mbx/src/adapters/",
}


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "trait", help="Domain trait to generate tests for (e.g. BridgeNetworking)"
    )
    parser.add_argument(
        "--output", help="Output file path (default: Claude decides)", default=None
    )
    args = parser.parse_args()

    output_hint = f"\nWrite the tests to: {args.output}" if args.output else ""
    adapter_dir = TRAIT_HINTS.get(args.trait, "crates/mbx/src/adapters/")

    prompt = f"""Generate unit tests for a new `{args.trait}` adapter in mbx.

Steps:
1. Read `crates/mbx/src/domain.rs` to understand the `{args.trait}` trait definition
2. Read `crates/mbx/src/adapters/mocks.rs` to understand the mock adapter pattern
3. Read 2-3 existing test modules (e.g. in `crates/mbx/src/container/` or
   `crates/mbx/src/image/`) to match the project's test style
4. Generate a complete test module for a `Mock{args.trait}` adapter that:
   - Implements `{args.trait}` from domain.rs
   - Covers the happy path, error conditions, and edge cases
   - Follows the AAA pattern (Arrange / Act / Assert)
   - Uses `#[tokio::test]` for async tests
   - Matches existing naming conventions exactly
5. Write the tests to `{adapter_dir}` alongside the existing adapters{output_hint}

Do not invent trait methods — only implement what is defined in domain.rs."""

    print(f"Generating tests for {args.trait}...\n")

    run_id = agent_log.log_start("gen-tests", {"trait": args.trait})
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

    agent_log.log_complete(
        run_id,
        "gen-tests",
        {"trait": args.trait},
        "\n".join(output_parts),
        time.monotonic() - start,
    )


if __name__ == "__main__":
    asyncio.run(main())
