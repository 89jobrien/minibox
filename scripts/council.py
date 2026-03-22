#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
# ]
# ///
"""Council analysis — multi-role AI review of the current branch.

Core mode (default): Strict Critic, Creative Explorer, General Analyst
Extensive mode:      + Security Reviewer, Performance Analyst

Each role scores branch health 0.0–1.0. Synthesis produces consensus areas,
dialectic tensions, and balanced recommendations.
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

ROLES: dict[str, tuple[str, str]] = {
    "strict-critic": (
        "Strict Critic",
        "You are the STRICT CRITIC on a council of expert reviewers. Be conservative and demanding.\n"
        "Your output must include:\n"
        "- **Health Score**: 0.0–1.0 (be conservative — only near-perfect code scores above 0.85)\n"
        "- **Summary**: What concerns you most (2–3 sentences)\n"
        "- **Key Observations**: Specific findings — cite file paths and line numbers where possible\n"
        "- **Risks Identified**: Technical debt, incomplete paths, missing invariants, breaking changes\n"
        "- **Code Smells**: Quality and maintainability issues\n"
        "- **Recommendations**: Concrete risk-mitigation actions\n\n"
        "Format each observation as: 'finding — source: file/symbol or commit hash'",
    ),
    "creative-explorer": (
        "Creative Explorer",
        "You are the CREATIVE EXPLORER on a council of expert reviewers. Be optimistic and inventive.\n"
        "Your output must include:\n"
        "- **Health Score**: 0.0–1.0 (reward ambition and potential)\n"
        "- **Summary**: What excites you about this change (2–3 sentences)\n"
        "- **Innovation Opportunities**: Simpler approaches, pattern unifications, new possibilities unlocked\n"
        "- **Architectural Potential**: How this lays groundwork for future improvements\n"
        "- **Experimental Value**: What this change validates or disproves\n"
        "- **Recommendations**: Ideas to extend or amplify the value of this work\n\n"
        "Format each observation as: 'finding — source: file/symbol or commit hash'",
    ),
    "general-analyst": (
        "General Analyst",
        "You are the GENERAL ANALYST on a council of expert reviewers. Be balanced and evidence-based.\n"
        "Your output must include:\n"
        "- **Health Score**: 0.0–1.0 (weight quality, tests, and conventions equally)\n"
        "- **Summary**: Overall assessment of branch state (2–3 sentences)\n"
        "- **Progress Indicators**: What is done well, with evidence\n"
        "- **Work Patterns**: Development approach and consistency\n"
        "- **Gaps**: Missing tests, docs, or convention violations (cite CLAUDE.md where relevant)\n"
        "- **Recommendations**: Balanced improvements\n\n"
        "Format each observation as: 'finding — source: file/symbol or commit hash'",
    ),
    "security-reviewer": (
        "Security Reviewer",
        "You are the SECURITY REVIEWER on a council of expert reviewers. Focus on attack surface.\n"
        "For this Rust container runtime, scrutinise: path traversal, symlink attacks, tar extraction,\n"
        "privilege escalation, unsafe block soundness, socket auth bypasses, resource exhaustion,\n"
        "cgroup/namespace escapes, and any new attack surface introduced.\n"
        "Your output must include:\n"
        "- **Health Score**: 0.0–1.0 (any critical vuln = max 0.4)\n"
        "- **Summary**: Security posture of this change\n"
        "- **Findings**: Each rated critical / high / medium / low — cite exact code locations\n"
        "- **Recommendations**: Specific hardening actions\n\n"
        "Format each observation as: 'finding — source: file/symbol or commit hash'",
    ),
    "performance-analyst": (
        "Performance Analyst",
        "You are the PERFORMANCE ANALYST on a council of expert reviewers. Focus on efficiency.\n"
        "Your output must include:\n"
        "- **Health Score**: 0.0–1.0\n"
        "- **Summary**: Performance posture of this change\n"
        "- **Bottlenecks**: Unnecessary allocations, blocking calls in async context, redundant syscalls,\n"
        "  lock contention, inefficient algorithms — cite exact code locations\n"
        "- **Zero-Copy / Benchmark Risks**: Missed opportunities and regression risks\n"
        "- **Recommendations**: Concrete alternatives with expected impact\n\n"
        "Format each observation as: 'finding — source: file/symbol or commit hash'",
    ),
}

CORE_ROLES = ["strict-critic", "creative-explorer", "general-analyst"]
EXTENSIVE_ROLES = CORE_ROLES + ["security-reviewer", "performance-analyst"]


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


def run_git(cmd: list[str]) -> str:
    return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()


def get_branch_context(base: str) -> str:
    branch = run_git(["git", "rev-parse", "--abbrev-ref", "HEAD"])
    commits = run_git(["git", "log", f"{base}...HEAD", "--oneline"])
    files = run_git(["git", "diff", f"{base}...HEAD", "--name-only"])
    diff = run_git(["git", "diff", f"{base}...HEAD"])
    if not diff:
        diff = run_git(["git", "diff", "HEAD"])
        commits = run_git(["git", "log", "-5", "--oneline"])
        files = run_git(["git", "diff", "HEAD", "--name-only"])
    return (
        f"Branch: {branch} (vs {base})\n\n"
        f"Commits:\n{commits or '(none ahead of base)'}\n\n"
        f"Changed files:\n{files or '(none)'}\n\n"
        f"Diff:\n```diff\n{diff or '(no diff)'}\n```"
    )


async def run_role(role_key: str, context: str) -> str:
    label, persona = ROLES[role_key]
    print(f"  [{label}]", end=" ", flush=True)
    parts: list[str] = []
    async for message in query(
        prompt=(
            f"{persona}\n\n"
            f"Analyse this branch. Read relevant source files to support your findings.\n\n"
            f"{context}"
        ),
        options=ClaudeAgentOptions(
            allowed_tools=["Read", "Glob", "Grep"],
            permission_mode="default",
        ),
    ):
        if hasattr(message, "result"):
            parts.append(message.result)
    print("done")
    return "\n".join(parts)


async def synthesize(role_outputs: dict[str, str], context: str) -> str:
    print("  [Synthesis]", end=" ", flush=True)
    council_text = "\n\n---\n\n".join(
        f"### {ROLES[role][0]}\n{output}"
        for role, output in role_outputs.items()
    )
    parts: list[str] = []
    async for message in query(
        prompt=(
            "You are synthesising a multi-role council code review into a final verdict.\n\n"
            "Your synthesis must contain exactly these sections:\n\n"
            "**Health Scores**\n"
            "List each role's score and compute the meta-score (weighted average, "
            "give Strict Critic 1.5× weight).\n\n"
            "**Areas of Consensus**\n"
            "Bullet points of findings where 2+ roles agree.\n\n"
            "**Areas of Tension**\n"
            "For each disagreement use this dialectic format:\n"
            "'[Role A] sees [X] (conservative/optimistic view), AND [Role B] sees [Y], "
            "suggesting [balanced resolution].'\n\n"
            "**Balanced Recommendations**\n"
            "Top 3–5 ranked actions the developer should take, synthesising all perspectives.\n\n"
            "**Branch Health**\n"
            "One of: Good / Needs work / Significant issues — with a one-line justification.\n\n"
            f"Branch context:\n{context}\n\n"
            f"Council findings:\n{council_text}"
        ),
        options=ClaudeAgentOptions(
            allowed_tools=["Read", "Glob", "Grep"],
            permission_mode="default",
        ),
    ):
        if hasattr(message, "result"):
            parts.append(message.result)
    print("done")
    return "\n".join(parts)


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="main", help="Base branch/ref (default: main)")
    parser.add_argument(
        "--mode", choices=["core", "extensive"], default="core",
        help="core = 3 roles, extensive = 5 roles (default: core)",
    )
    parser.add_argument("--no-synthesis", action="store_true", help="Skip synthesis step")
    args = parser.parse_args()

    roles = EXTENSIVE_ROLES if args.mode == "extensive" else CORE_ROLES
    context = get_branch_context(args.base)
    if not context.strip():
        print("No branch context — nothing to analyse.")
        sys.exit(0)

    role_count = len(roles) + (0 if args.no_synthesis else 1)
    print(f"\nCouncil analysis — {args.mode} mode · {len(roles)} roles + synthesis · vs {args.base}\n")
    print(f"Running {role_count} agent calls...\n")

    run_id = _log_start("council", {"base": args.base, "mode": args.mode})
    start = time.monotonic()
    all_output: list[str] = []

    role_outputs: dict[str, str] = {}
    for role in roles:
        role_outputs[role] = await run_role(role, context)

    print()
    for role, output in role_outputs.items():
        label = ROLES[role][0]
        print(f"{'─' * 4} {label} {'─' * (56 - len(label))}")
        print(output)
        print()
        all_output.append(f"## {label}\n{output}")

    if not args.no_synthesis:
        synthesis = await synthesize(role_outputs, context)
        print("─" * 60)
        print("  SYNTHESIS")
        print("─" * 60)
        print(synthesis)
        print()
        all_output.append(f"## Synthesis\n{synthesis}")

    _log_complete(run_id, "council", {"base": args.base, "mode": args.mode},
                  "\n\n".join(all_output), time.monotonic() - start)


if __name__ == "__main__":
    asyncio.run(main())
