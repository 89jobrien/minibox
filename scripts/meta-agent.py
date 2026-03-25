#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
#   "requests",
# ]
# ///
"""Meta-agent: designs, configures, and spawns parallel agents from user intent.

Workflow:
  1. Fetch + cache Claude Agent SDK docs (24h TTL)
  2. Collect repo context (project rules, git log, structure)
  3. Designer agent → JSON agent plan (2–5 independent agents)
  4. Execute all designed agents concurrently
  5. Synthesize results into a final report
  6. Log output to ~/.mbx/ai-logs/<sha>-meta-agent.md

Works in any git repo — context is discovered dynamically from CLAUDE.md,
AGENTS.md, README.md and git state. Not minibox-specific.
"""

import argparse
import asyncio
import html.parser
import json
import subprocess
import sys
import time
from datetime import datetime, timedelta
from pathlib import Path

import requests

import sys as _sys
import os as _os
_sys.path.insert(0, _os.path.dirname(__file__))
import agent_log

# Allow spawning nested claude subprocesses when running inside a Claude Code session
_os.environ.pop("CLAUDECODE", None)

from claude_agent_sdk import ClaudeAgentOptions, query

# ── Constants ─────────────────────────────────────────────────────────────────

_CACHE_DIR = Path.home() / ".mbx" / "cache"
_SDK_DOCS_CACHE = _CACHE_DIR / "sdk-docs.md"
_SDK_DOCS_TTL_HOURS = 24
_SDK_DOCS_URLS = [
    "https://docs.anthropic.com/en/docs/claude-code/sdk",
    "https://docs.anthropic.com/en/docs/claude-code/sdk/sdk-python",
]

ALL_TOOLS = ["Read", "Glob", "Grep", "Bash", "Write", "Edit"]
SAFE_TOOLS = ["Read", "Glob", "Grep"]

# ── HTML → plain text ─────────────────────────────────────────────────────────

class _TextExtractor(html.parser.HTMLParser):
    _SKIP = {"script", "style", "nav", "footer", "header"}

    def __init__(self) -> None:
        super().__init__()
        self._skip_depth = 0
        self._parts: list[str] = []

    def handle_starttag(self, tag: str, attrs: list) -> None:
        if tag in self._SKIP:
            self._skip_depth += 1

    def handle_endtag(self, tag: str) -> None:
        if tag in self._SKIP and self._skip_depth:
            self._skip_depth -= 1

    def handle_data(self, data: str) -> None:
        if not self._skip_depth:
            stripped = data.strip()
            if stripped:
                self._parts.append(stripped)

    def text(self) -> str:
        return "\n".join(self._parts)


def _html_to_text(html: str) -> str:
    p = _TextExtractor()
    p.feed(html)
    return p.text()


# ── SDK docs fetch + cache ────────────────────────────────────────────────────

def fetch_sdk_docs(force_refresh: bool = False) -> str:
    """Return cached SDK docs, refreshing if stale or missing."""
    if not force_refresh and _SDK_DOCS_CACHE.exists():
        age = datetime.now() - datetime.fromtimestamp(_SDK_DOCS_CACHE.stat().st_mtime)
        if age < timedelta(hours=_SDK_DOCS_TTL_HOURS):
            return _SDK_DOCS_CACHE.read_text()

    _CACHE_DIR.mkdir(parents=True, exist_ok=True)
    sections: list[str] = []
    for url in _SDK_DOCS_URLS:
        try:
            resp = requests.get(url, timeout=15, headers={"User-Agent": "meta-agent/1.0"})
            resp.raise_for_status()
            text = _html_to_text(resp.text) if "<html" in resp.text[:200].lower() else resp.text
            sections.append(f"<!-- {url} -->\n{text[:20000]}")
        except Exception as e:
            sections.append(f"<!-- failed to fetch {url}: {e} -->")

    content = "\n\n---\n\n".join(sections)
    _SDK_DOCS_CACHE.write_text(content)
    return content


# ── Repo context ──────────────────────────────────────────────────────────────

def get_repo_context() -> str:
    """Discover and collect project context from the current repo."""
    def run(cmd: list[str]) -> str:
        return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()

    # Discover rule/context files in priority order
    candidates = ["CLAUDE.md", "AGENTS.md", "GEMINI.md", "README.md"]
    rule_dirs = [".claude/rules", ".cursor/rules", "docs"]
    rule_files: list[Path] = []
    for name in candidates:
        p = Path(name)
        if p.exists():
            rule_files.append(p)
    for d in rule_dirs:
        rule_files.extend(sorted(Path(d).glob("*.md")) if Path(d).exists() else [])

    rules_text = ""
    char_budget = 6000
    for f in rule_files:
        content = f.read_text(errors="replace")
        chunk = content[:char_budget]
        rules_text += f"\n### {f}\n{chunk}\n"
        char_budget -= len(chunk)
        if char_budget <= 0:
            break

    git_log = run(["git", "log", "--oneline", "-20"])
    git_status = run(["git", "status", "--short"])
    git_stat = run(["git", "diff", "HEAD", "--stat"])
    branch = run(["git", "rev-parse", "--abbrev-ref", "HEAD"])

    # Shallow structure — skip build artifacts and hidden dirs
    try:
        entries = sorted(
            p for p in Path(".").rglob("*")
            if not any(part.startswith((".git", "target", "node_modules", "__pycache__", ".worktrees"))
                       for part in p.parts)
            and p.stat().st_size < 10_000_000
        )
        structure = "\n".join(str(p) for p in entries[:150])
    except Exception:
        structure = "(could not enumerate structure)"

    return (
        f"## Project rules\n{rules_text}\n\n"
        f"## Branch: {branch}\n"
        f"## Recent commits\n{git_log}\n\n"
        f"## Working tree\n{git_status}\n{git_stat}\n\n"
        f"## Structure (first 150 paths)\n{structure}"
    )


# ── Agent phases ──────────────────────────────────────────────────────────────

async def design_agents(task: str, repo_ctx: str, sdk_docs: str) -> list[dict]:
    """Designer agent: produce a parallel agent plan as JSON."""
    print("  [Designer]", end=" ", flush=True)
    parts: list[str] = []
    async for message in query(
        prompt=(
            "You are a meta-agent designer. Given a task, repo context, and the Claude Agent SDK docs, "
            "design the smallest set of parallel agents that efficiently accomplishes the task.\n\n"
            "Output ONLY a valid JSON array — no markdown fences, no explanation — with this schema:\n"
            '[\n'
            '  {\n'
            '    "name": "kebab-case-name",\n'
            '    "role": "one sentence describing this agent\'s independent concern",\n'
            '    "prompt": "complete self-contained prompt for this agent",\n'
            '    "tools": ["Read", "Glob", "Grep"]\n'
            '  }\n'
            ']\n\n'
            "Rules:\n"
            "- 2–5 agents; each must have a distinct, non-overlapping concern\n"
            "- Prompts must be fully self-contained (the agent has no other context)\n"
            "- Include the repo context in each prompt only where relevant to that agent's concern\n"
            "- Available tools: Read, Glob, Grep (safe reads); Bash, Write, Edit (modifications)\n"
            "- Only grant Write/Edit/Bash when the agent genuinely needs to modify or execute\n"
            "- Do NOT include a synthesis agent — synthesis is handled externally\n\n"
            f"## Task\n{task}\n\n"
            f"## Repo context\n{repo_ctx[:4000]}\n\n"
            f"## Claude Agent SDK docs (for designing agent prompts correctly)\n{sdk_docs[:6000]}"
        ),
        options=ClaudeAgentOptions(allowed_tools=SAFE_TOOLS, permission_mode="default",
                                   stderr=lambda line: print(f"[designer-stderr] {line}", flush=True)),
    ):
        if hasattr(message, "result"):
            parts.append(message.result)
    print("done")

    raw = "\n".join(parts).strip()
    if raw.startswith("```"):
        raw = raw.split("\n", 1)[1].rsplit("```", 1)[0].strip()
    try:
        plan = json.loads(raw)
        assert isinstance(plan, list) and all(isinstance(a, dict) for a in plan)
        return plan
    except (json.JSONDecodeError, AssertionError) as e:
        print(f"\nWarning: designer returned invalid JSON ({e}) — using single fallback agent.")
        return [{"name": "analyst", "role": "General analysis", "prompt": task, "tools": SAFE_TOOLS}]


async def run_agent(spec: dict) -> tuple[str, str]:
    """Execute one designed agent. Returns (name, output)."""
    name = spec.get("name", "agent")
    print(f"  [{name}]", end=" ", flush=True)
    tools = [t for t in spec.get("tools", SAFE_TOOLS) if t in ALL_TOOLS] or SAFE_TOOLS
    parts: list[str] = []
    async for message in query(
        prompt=spec["prompt"],
        options=ClaudeAgentOptions(allowed_tools=tools, permission_mode="default"),
    ):
        if hasattr(message, "result"):
            parts.append(message.result)
    print("done")
    return name, "\n".join(parts)


async def synthesize(task: str, agent_outputs: dict[str, str]) -> str:
    """Synthesize all agent outputs into a final report."""
    print("  [Synthesis]", end=" ", flush=True)
    combined = "\n\n---\n\n".join(
        f"### {name}\n{output}" for name, output in agent_outputs.items()
    )
    parts: list[str] = []
    async for message in query(
        prompt=(
            "Synthesize the outputs from multiple parallel agents into a single coherent report.\n\n"
            "Sections (use exactly these headings):\n\n"
            "**Summary** — 2–3 sentences: what the agents found and the overall verdict\n\n"
            "**Key Findings** — deduplicated bullet points from all agents, grouped by theme\n\n"
            "**Recommended Actions** — ranked list; include who/what/why for each\n\n"
            "**Open Questions** — anything unresolved or needing follow-up\n\n"
            f"Original task: {task}\n\n"
            f"Agent outputs:\n{combined}"
        ),
        options=ClaudeAgentOptions(allowed_tools=SAFE_TOOLS, permission_mode="default"),
    ):
        if hasattr(message, "result"):
            parts.append(message.result)
    print("done")
    return "\n".join(parts)


# ── Entry point ───────────────────────────────────────────────────────────────

async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("task", nargs="?", help="Task description (or pipe via stdin)")
    parser.add_argument("--no-synthesis", action="store_true", help="Skip synthesis step")
    parser.add_argument("--refresh-docs", action="store_true",
                        help="Force re-fetch SDK docs even if cache is fresh")
    args = parser.parse_args()

    task = args.task
    if not task:
        if not sys.stdin.isatty():
            task = sys.stdin.read().strip()
        if not task:
            parser.error("Provide a task as an argument or via stdin")

    sha = agent_log.git_short_sha()
    now = datetime.now()
    print(f"\nmeta-agent @ {sha} — {now.strftime('%Y-%m-%d %H:%M')}\n")
    print(f"Task: {task}\n")

    print("Collecting repo context...", end=" ", flush=True)
    repo_ctx = get_repo_context()
    print("done")

    print("Fetching SDK docs...", end=" ", flush=True)
    sdk_docs = fetch_sdk_docs(force_refresh=args.refresh_docs)
    cache_age = datetime.now() - datetime.fromtimestamp(_SDK_DOCS_CACHE.stat().st_mtime)
    print(f"cached ({int(cache_age.total_seconds() / 3600)}h old)")

    run_id = agent_log.log_start("meta-agent", {"task": task[:120], "sha": sha})
    start = time.monotonic()
    all_output: list[str] = []

    # Phase 1: design
    print("\nDesigning agent configuration...")
    plan = await design_agents(task, repo_ctx, sdk_docs)
    plan_lines = [
        f"- **{a['name']}**: {a.get('role', '')} (tools: {', '.join(a.get('tools', SAFE_TOOLS))})"
        for a in plan
    ]
    plan_md = "\n".join(plan_lines)
    print(f"\nPlan ({len(plan)} agents):\n{plan_md}\n")
    all_output.append(f"## Agent Plan\n{plan_md}")

    # Phase 2: execute in parallel
    print(f"Running {len(plan)} agents in parallel...")
    results = await asyncio.gather(*[run_agent(spec) for spec in plan])
    print()

    agent_outputs: dict[str, str] = {}
    for name, output in results:
        agent_outputs[name] = output
        sep = "─" * 4
        print(f"{sep} {name} {'─' * max(0, 56 - len(name))}")
        print(output)
        print()
        all_output.append(f"## {name}\n{output}")

    # Phase 3: synthesize
    if not args.no_synthesis:
        print("Synthesizing...")
        synthesis = await synthesize(task, agent_outputs)
        print("─" * 60)
        print("  SYNTHESIS")
        print("─" * 60)
        print(synthesis)
        print()
        all_output.append(f"## Synthesis\n{synthesis}")

    full_output = "\n\n".join(all_output)
    agent_log.log_complete(
        run_id, "meta-agent", {"task": task[:120]},
        full_output, time.monotonic() - start,
    )
    log_path = agent_log.save_commit_log(sha, "meta-agent", full_output, {
        "task": task[:120],
        "agents": len(plan),
        "date": now.strftime("%Y-%m-%d %H:%M"),
    })
    print(f"\nLogged to: {log_path}")


if __name__ == "__main__":
    asyncio.run(main())
