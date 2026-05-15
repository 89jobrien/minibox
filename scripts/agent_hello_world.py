#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "openai>=1.30",
# ]
# ///
"""
Minibox hello-world agent demo.

Spawns N parallel tasks, each running `echo "hello from agent <n>"` inside
an ephemeral smolvm machine defined by scripts/smolfiles/agent-hello.toml,
then asks an LLM to summarise the results.

Usage:
    uv run scripts/agent_hello_world.py
    uv run scripts/agent_hello_world.py --agents 5
"""
import argparse
import asyncio
import sys
from pathlib import Path

from openai import AsyncOpenAI

REPO_ROOT = Path(__file__).resolve().parent.parent
SMOLFILE = REPO_ROOT / "scripts" / "smolfiles" / "agent-hello.toml"


async def run_agent(n: int, stagger_s: float = 0.5) -> tuple[int, str]:
    """Run echo in an ephemeral smolvm machine and return (n, output)."""
    if stagger_s > 0:
        await asyncio.sleep(n * stagger_s)
    proc = await asyncio.create_subprocess_exec(
        "smolvm", "machine", "run",
        "--smolfile", str(SMOLFILE),
        "--", "echo", f"hello from agent {n}",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await proc.communicate()
    if proc.returncode != 0:
        err = stderr.decode().strip()
        return n, f"ERROR (exit {proc.returncode}): {err}"
    return n, stdout.decode().strip()


async def summarise(client: AsyncOpenAI, results: list[tuple[int, str]]) -> str:
    lines = "\n".join(f"agent-{n}: {out}" for n, out in sorted(results))
    resp = await client.chat.completions.create(
        model="gpt-4o-mini",
        messages=[
            {
                "role": "system",
                "content": "You are a concise assistant summarising agent run results.",
            },
            {
                "role": "user",
                "content": (
                    f"These agents each ran `echo` in an isolated smolvm VM:\n\n"
                    f"{lines}\n\n"
                    "Give a one-sentence summary confirming they all ran successfully "
                    "(or note any failures)."
                ),
            },
        ],
        max_tokens=100,
    )
    return resp.choices[0].message.content.strip()


async def run(n: int) -> None:
    print(f"Launching {n} smolvm hello-world agents in parallel ...\n")

    results: list[tuple[int, str]] = await asyncio.gather(
        *[run_agent(i, stagger_s=0.5) for i in range(n)]
    )

    print("--- raw outputs ---")
    for idx, out in sorted(results):
        print(f"  agent-{idx}: {out}")
    print()

    client = AsyncOpenAI()
    summary = await summarise(client, results)
    print("--- summary ---")
    print(summary)
    print("---------------")


def main() -> None:
    parser = argparse.ArgumentParser(description="Minibox hello-world agent demo")
    parser.add_argument(
        "--agents", "-n",
        type=int,
        default=3,
        metavar="N",
        help="Number of parallel smolvm agents (default: 3)",
    )
    args = parser.parse_args()

    if not SMOLFILE.exists():
        print(f"error: smolfile not found: {SMOLFILE}", file=sys.stderr)
        sys.exit(1)

    asyncio.run(run(args.agents))


if __name__ == "__main__":
    main()
