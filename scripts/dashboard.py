#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "rich",
# ]
# ///
"""minibox agent dashboard — history, metrics, and benchmark data."""

import json
import sys as _sys
import os as _os
_sys.path.insert(0, _os.path.dirname(__file__))
import agent_log
import bench_data

from collections import defaultdict
from datetime import datetime
from pathlib import Path

from rich import box
from rich.columns import Columns
from rich.console import Console
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

_SCRIPTS = ["ai-review", "gen-tests", "diagnose", "bench-agent", "council", "meta-agent"]
_PREVIEW_LEN = 40


def load_runs() -> dict[str, dict]:
    """Read JSONL and return a dict of run_id -> latest entry (complete wins over running)."""
    if not agent_log.LOG_FILE.exists():
        return {}
    runs: dict[str, dict] = {}
    with agent_log.LOG_FILE.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue
            run_id = entry.get("run_id", "")
            existing = runs.get(run_id)
            if existing is None or entry.get("status") == "complete":
                runs[run_id] = entry
    return runs


def fmt_ts(iso: str) -> str:
    try:
        dt = datetime.fromisoformat(iso)
        return dt.strftime("%Y-%m-%d %H:%M")
    except ValueError:
        return iso[:16]


def fmt_args(args: dict) -> str:
    parts = []
    for k, v in args.items():
        if v is not None:
            parts.append(f"{k}={v}")
    return " ".join(parts) or "—"


def preview(text: str) -> str:
    first = next((ln.strip() for ln in text.splitlines() if ln.strip()), "")
    first = first.lstrip("#").strip()
    return first[:_PREVIEW_LEN] + "…" if len(first) > _PREVIEW_LEN else first


def build_summary_table(by_script: dict[str, list[dict]]) -> Table:
    t = Table(box=box.SIMPLE_HEAD, show_edge=False, pad_edge=False)
    t.add_column("Script", style="bold cyan", min_width=12)
    t.add_column("Runs", justify="right", style="white", min_width=4)
    t.add_column("Avg", justify="right", style="white", min_width=6)
    t.add_column("Last Run", style="white", min_width=16)
    t.add_column("Last Output", style="dim white", no_wrap=True, max_width=_PREVIEW_LEN)

    for script in _SCRIPTS:
        runs = by_script.get(script, [])
        if not runs:
            continue
        complete = [r for r in runs if r.get("status") == "complete"]
        running = [r for r in runs if r.get("status") == "running"]
        durations = [r["duration_s"] for r in complete if "duration_s" in r]
        avg_dur = f"{sum(durations)/len(durations):.1f}s" if durations else "—"
        sorted_runs = sorted(runs, key=lambda r: r.get("run_id", ""), reverse=True)
        last = sorted_runs[0] if sorted_runs else None
        last_ts = fmt_ts(last["run_id"]) if last else "—"
        last_out = preview(last.get("output", "")) if last and last.get("status") == "complete" else ("⠿ running" if running else "—")
        t.add_row(script, str(len(runs)), avg_dur, last_ts, last_out)
    return t


def build_history_table(all_runs: list[dict], limit: int = 20) -> Table:
    t = Table(box=box.SIMPLE_HEAD, show_edge=False, pad_edge=False)
    t.add_column("Time", style="white", min_width=16)
    t.add_column("Script", style="bold cyan", min_width=12)
    t.add_column("Status", min_width=8)
    t.add_column("Dur", justify="right", style="white", min_width=6)
    t.add_column("Output", style="dim white", no_wrap=True, max_width=_PREVIEW_LEN)

    recent = sorted(all_runs, key=lambda r: r.get("run_id", ""), reverse=True)[:limit]
    for run in recent:
        status = run.get("status", "?")
        status_cell = Text("● done", style="green") if status == "complete" else Text("⠿ live", style="yellow")
        dur = f"{run['duration_s']:.1f}s" if "duration_s" in run else "—"
        out = preview(run.get("output", "")) if status == "complete" else ""
        t.add_row(fmt_ts(run.get("run_id", "")), run.get("script", "?"), status_cell, dur, out)
    return t


def build_bench_table() -> Table | None:
    """Build a table of latest benchmark results from bench/results/."""
    latest = bench_data.load_latest()
    if not latest or not latest.is_valid:
        return None

    t = Table(box=box.SIMPLE_HEAD, show_edge=False, pad_edge=False)
    t.add_column("Suite", style="bold magenta", min_width=8)
    t.add_column("Test", style="white", min_width=14)
    t.add_column("Avg", justify="right", style="white", min_width=8)
    t.add_column("P95", justify="right", style="white", min_width=8)
    t.add_column("Min", justify="right", style="dim white", min_width=8)
    t.add_column("Iter", justify="right", style="dim white", min_width=4)
    t.add_column("Δ prev", justify="right", min_width=8)

    # Get previous run for delta column
    history = bench_data.valid_vps_runs()
    prev = history[-2] if len(history) >= 2 else None

    for suite in latest.suites:
        for test in suite.tests:
            if test.iterations == 0:
                continue
            avg = bench_data.format_duration(test.avg_us)
            p95 = bench_data.format_duration(test.p95_us)
            min_val = bench_data.format_duration(test.min_us)

            delta_text = Text("—", style="dim")
            if prev:
                prev_test = prev.test_by_name(suite.name, test.name)
                if prev_test and prev_test.avg_us and test.avg_us:
                    pct = ((test.avg_us - prev_test.avg_us) / prev_test.avg_us) * 100
                    pct_str = bench_data.format_pct(pct)
                    if pct > 10:
                        delta_text = Text(pct_str, style="bold red")
                    elif pct < -10:
                        delta_text = Text(pct_str, style="bold green")
                    else:
                        delta_text = Text(pct_str, style="dim")

            t.add_row(suite.name, test.name, avg, p95, min_val, str(test.iterations), delta_text)
    return t


def build_bench_header(latest: bench_data.BenchRun) -> Text:
    """Build the bench section header with metadata."""
    history = bench_data.valid_vps_runs()
    regressions = bench_data.detect_regressions()

    parts: list[tuple[str, str]] = [
        ("Benchmarks", "bold white"),
        ("  ·  ", "dim"),
        (f"{latest.git_sha[:8]}", "cyan"),
        ("  ", ""),
        (f"{latest.hostname}", "dim white"),
        ("  ", ""),
        (f"{fmt_ts(latest.timestamp)}", "dim white"),
    ]
    if regressions:
        parts.extend([("  ", ""), (f"{len(regressions)} regression(s) vs worst prior", "bold red")])
    parts.extend([("  ", ""), (f"{len(history)} VPS runs", "dim")])

    return Text.assemble(*parts)


def main() -> None:
    console = Console()
    runs = load_runs()

    # ── Agent section ────────────────────────────────────────────────────
    if not runs:
        has_agents = False
    else:
        has_agents = True

    # ── Bench section ────────────────────────────────────────────────────
    latest = bench_data.load_latest()
    has_bench = latest is not None and latest.is_valid

    if not has_agents and not has_bench:
        console.print(Panel(
            "[dim]No data yet. Try:[/dim]\n\n"
            "  [cyan]just ai-review[/cyan]\n"
            "  [cyan]just bench[/cyan]\n"
            "  [cyan]uv run scripts/bench-agent.py report[/cyan]",
            title="[bold]minibox dashboard[/bold]",
            border_style="dim",
        ))
        return

    console.print()

    if has_agents:
        by_script: dict[str, list[dict]] = defaultdict(list)
        for run in runs.values():
            by_script[run.get("script", "unknown")].append(run)

        total = len(runs)
        complete = sum(1 for r in runs.values() if r.get("status") == "complete")
        running = sum(1 for r in runs.values() if r.get("status") == "running")

        header = Text.assemble(
            ("Agents", "bold white"),
            ("  ·  ", "dim"),
            (f"{total} runs", "cyan"),
            ("  ", ""),
            (f"{complete} complete", "green"),
            ("  ", ""),
            *([( f"{running} running", "yellow")] if running else []),
        )

        console.print(Panel(header, border_style="dim cyan", padding=(0, 1)))
        console.print()
        console.print("[bold]Summary[/bold]", style="dim")
        console.print(build_summary_table(by_script))
        console.print()
        console.print("[bold]Recent Runs[/bold]", style="dim")
        console.print(build_history_table(list(runs.values())))
        console.print()

    if has_bench:
        bench_hdr = build_bench_header(latest)
        console.print(Panel(bench_hdr, border_style="dim magenta", padding=(0, 1)))
        console.print()
        bench_table = build_bench_table()
        if bench_table:
            console.print(bench_table)
            console.print()

        # Show storage stats
        file_count = bench_data.result_file_count()
        size_mb = bench_data.result_dir_size_mb()
        console.print(
            f"  [dim]bench/results/: {file_count} files, {size_mb:.1f} MB[/dim]"
        )
        console.print()


if __name__ == "__main__":
    main()
