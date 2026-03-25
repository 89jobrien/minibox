#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "claude-agent-sdk",
# ]
# ///
"""Bench agent — AI-powered benchmark analysis, regression detection, and management.

Subcommands:
  report    Summarize latest bench results with AI commentary
  compare   Compare two runs by git SHA (or latest two VPS runs)
  regress   Detect regressions and explain probable causes
  cleanup   Identify and remove stale/duplicate result files
  trigger   Run benchmarks via xtask and analyze results

Reads bench/results/bench.jsonl + latest.json. Writes AI output to
~/.mbx/ai-logs/<sha>-bench-agent.md.
"""

import argparse
import asyncio
import time

import sys as _sys
import os as _os
_sys.path.insert(0, _os.path.dirname(__file__))
import agent_log
import bench_data

_os.environ.pop("CLAUDECODE", None)

from claude_agent_sdk import ClaudeAgentOptions, query


def _bench_context() -> str:
    """Build context string from bench data for the AI prompt."""
    parts = [bench_data.summary_text(), ""]

    history = bench_data.valid_vps_runs()
    if len(history) >= 2:
        diffs = bench_data.compare_runs(history[-2], history[-1])
        if diffs:
            parts.append("Delta (prev vs latest):")
            for d in diffs:
                parts.append(
                    f"  {d.suite}/{d.test}: "
                    f"{bench_data.format_duration(d.prev_avg_us)} -> "
                    f"{bench_data.format_duration(d.curr_avg_us)} "
                    f"({bench_data.format_pct(d.pct_change)})"
                )
            parts.append("")

    parts.append(f"Results dir: {bench_data.RESULTS_DIR}")
    parts.append(f"JSONL path: {bench_data.JSONL_PATH}")
    parts.append(f"Schema: {bench_data.SCHEMA_PATH}")
    return "\n".join(parts)


SYSTEM_PROMPT = """\
You are the minibox bench agent. You analyze benchmark results for the minibox \
container runtime. The project uses a custom Rust bench harness (crates/minibox-bench) \
that produces JSON results to bench/results/.

Key facts:
- bench.jsonl is append-only history; latest.json is the current snapshot
- VPS runs (hostname=jobrien) are the canonical source of truth; macOS runs are \
  expected to have "No such file or directory" errors (no daemon on macOS)
- Suites: codec (protocol encode/decode, ns-scale), adapter (trait dispatch, ns-scale), \
  pull/run/exec/e2e (CLI lifecycle, us/ms-scale, require daemon)
- Regressions are detected conservatively: latest avg must exceed the worst \
  prior VPS avg by more than the threshold
- bench/results/ also contains timestamped .json + .txt pairs per run

Be concise. Use tables when comparing. Cite specific numbers."""


async def cmd_report(args: argparse.Namespace) -> None:
    ctx = _bench_context()
    prompt = f"""Summarize the current minibox benchmark state.

{ctx}

Read bench/results/latest.json for full details. Produce:
1. A performance overview table (suite / test / avg / p95 / iterations)
2. Notable observations (fast/slow outliers, high variance)
3. Comparison to the previous VPS run if available
4. Any recommendations for the next bench run"""

    await _run_agent("report", prompt, args, tools=["Read", "Glob", "Grep"])


async def cmd_compare(args: argparse.Namespace) -> None:
    ctx = _bench_context()
    sha_hint = ""
    if args.sha:
        sha_hint = f"Compare runs with git SHAs containing: {', '.join(args.sha)}"
    else:
        sha_hint = "Compare the two most recent valid VPS runs."

    prompt = f"""{sha_hint}

{ctx}

Read bench/results/bench.jsonl for full history. For each test:
- Show avg, min, p95 for both runs
- Calculate % change and flag regressions (>10%) or improvements (<-10%)
- Present as a comparison table
- If regressions exist, hypothesize likely causes (check git log between the two SHAs)"""

    await _run_agent("compare", prompt, args, tools=["Read", "Glob", "Grep", "Bash"])


async def cmd_regress(args: argparse.Namespace) -> None:
    ctx = _bench_context()
    threshold = args.threshold

    regressions = bench_data.detect_regressions(threshold)
    if not regressions:
        reg_summary = f"No regressions detected above {threshold}% threshold."
    else:
        lines = [f"Detected {len(regressions)} regression(s) above {threshold}%:"]
        for r in regressions:
            lines.append(
                f"  {r.suite}/{r.test}: {bench_data.format_pct(r.pct_change)} "
                f"(worst prior {bench_data.format_duration(r.prev_avg_us)} -> {bench_data.format_duration(r.curr_avg_us)})"
            )
        reg_summary = "\n".join(lines)

    prompt = f"""Analyze benchmark regressions for minibox.

{reg_summary}

{ctx}

Steps:
1. Read bench/results/bench.jsonl for the full run history
2. For each regression, identify the git SHA range where it appeared
3. Run `git log --oneline <prev_sha>..<curr_sha>` to find commits in that range
4. Correlate file changes with the regressing benchmark (e.g. protocol changes -> codec suite)
5. Produce a regression report with:
   - Which tests regressed and by how much
   - Probable cause (specific commit or code change)
   - Whether the regression is real or noise (check iteration count, variance)
   - Suggested action (revert, optimize, accept)"""

    await _run_agent("regress", prompt, args, tools=["Read", "Glob", "Grep", "Bash"])


async def cmd_cleanup(args: argparse.Namespace) -> None:
    ctx = _bench_context()
    dry_run_note = " (DRY RUN — report only, do not delete)" if args.dry_run else ""

    prompt = f"""Audit and clean up bench/results/.{dry_run_note}

{ctx}

Steps:
1. List all files in bench/results/ with sizes
2. Identify:
   - Duplicate runs (same git_sha, same host, within minutes)
   - Failed macOS runs (all errors, no valid data)
   - Orphaned .txt files without matching .json
   - Empty timestamped directories
3. Report what should be removed and why
4. Calculate space savings
{"5. Do NOT delete anything — only report." if args.dry_run else "5. Delete identified files, but NEVER touch bench.jsonl or latest.json."}"""

    tools = ["Read", "Glob", "Grep", "Bash"] if not args.dry_run else ["Read", "Glob", "Grep"]
    await _run_agent("cleanup", prompt, args, tools=tools)


async def cmd_trigger(args: argparse.Namespace) -> None:
    suite_flag = f" --suite {args.suite}" if args.suite else ""
    vps = args.vps

    if vps:
        prompt = f"""Trigger a VPS benchmark run and analyze the results.

Steps:
1. Run `cargo xtask bench-vps{suite_flag}` to execute benchmarks on the VPS
2. Wait for it to complete (this will update bench/results/)
3. Read the updated bench/results/latest.json
4. Compare to the previous VPS run in bench.jsonl
5. Summarize results and flag any regressions"""
    else:
        prompt = f"""Trigger a local benchmark run and analyze the results.

Steps:
1. Run `cargo xtask bench{suite_flag}` to execute benchmarks locally
2. Wait for it to complete
3. Read the updated bench/results/latest.json
4. Summarize the results"""

    await _run_agent("trigger", prompt, args, tools=["Read", "Glob", "Grep", "Bash"])


async def _run_agent(
    subcmd: str,
    prompt: str,
    args: argparse.Namespace,
    tools: list[str],
) -> None:
    run_id = agent_log.log_start("bench-agent", {"subcmd": subcmd})
    start = time.monotonic()
    output_parts: list[str] = []

    print(f"bench-agent {subcmd}...\n")

    async for message in query(
        prompt=prompt,
        options=ClaudeAgentOptions(
            system_prompt=SYSTEM_PROMPT,
            allowed_tools=tools,
            permission_mode="acceptEdits",
            max_turns=getattr(args, "max_turns", 15),
        ),
    ):
        if hasattr(message, "result"):
            print(message.result)
            output_parts.append(message.result)

    duration = time.monotonic() - start
    output = "\n".join(output_parts)
    agent_log.log_complete(run_id, "bench-agent", {"subcmd": subcmd}, output, duration)

    sha = agent_log.git_short_sha()
    agent_log.save_commit_log(sha, f"bench-agent-{subcmd}", output, {
        "subcmd": subcmd,
        "duration": f"{duration:.1f}s",
    })


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--max-turns", type=int, default=15, help="Max agent turns (default: 15)")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("report", help="Summarize latest bench results")

    p_compare = sub.add_parser("compare", help="Compare bench runs")
    p_compare.add_argument("sha", nargs="*", help="Git SHAs to compare (default: latest two VPS runs)")

    p_regress = sub.add_parser("regress", help="Detect and explain regressions")
    p_regress.add_argument("--threshold", type=float, default=10.0, help="Regression threshold %% (default: 10)")

    p_cleanup = sub.add_parser("cleanup", help="Clean up stale result files")
    p_cleanup.add_argument("--dry-run", action="store_true", help="Report only, don't delete")

    p_trigger = sub.add_parser("trigger", help="Run benchmarks and analyze")
    p_trigger.add_argument("--suite", help="Specific suite (codec, adapter, pull, run, exec, e2e)")
    p_trigger.add_argument("--vps", action="store_true", help="Run on VPS instead of locally")

    args = parser.parse_args()
    handler = {
        "report": cmd_report,
        "compare": cmd_compare,
        "regress": cmd_regress,
        "cleanup": cmd_cleanup,
        "trigger": cmd_trigger,
    }[args.command]

    asyncio.run(handler(args))


if __name__ == "__main__":
    main()
