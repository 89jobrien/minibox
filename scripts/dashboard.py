#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "rich",
# ]
# ///
"""minibox agent dashboard — history and metrics from ~/.mbx/agent-runs.jsonl"""

import json
import sys as _sys
import os as _os
_sys.path.insert(0, _os.path.dirname(__file__))
import agent_log

from collections import defaultdict
from datetime import datetime
from pathlib import Path

from rich import box
from rich.columns import Columns
from rich.console import Console
from rich.panel import Panel
from rich.table import Table
from rich.text import Text
_SCRIPTS = ["ai-review", "gen-tests", "diagnose"]
_PREVIEW_LEN = 40


def load_runs() -> dict[str, dict]:
    """Read JSONL and return a dict of run_id → latest entry (complete wins over running)."""
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
            # complete always wins; running only fills if no entry yet
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
    t.add_column("Script", style="bold cyan", min_width=10)
    t.add_column("Runs", justify="right", style="white", min_width=4)
    t.add_column("Avg", justify="right", style="white", min_width=6)
    t.add_column("Last Run", style="white", min_width=16)
    t.add_column("Last Output", style="dim white", no_wrap=True, max_width=_PREVIEW_LEN)

    for script in _SCRIPTS:
        runs = by_script.get(script, [])
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
    t.add_column("Script", style="bold cyan", min_width=10)
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


def main() -> None:
    console = Console()
    runs = load_runs()

    if not runs:
        console.print(Panel(
            "[dim]No runs yet. Try:[/dim]\n\n  [cyan]just ai-review[/cyan]\n  [cyan]just gen-tests BridgeNetworking[/cyan]\n  [cyan]just diagnose[/cyan]",
            title="[bold]minibox agent dashboard[/bold]",
            border_style="dim",
        ))
        return

    by_script: dict[str, list[dict]] = defaultdict(list)
    for run in runs.values():
        by_script[run.get("script", "unknown")].append(run)

    total = len(runs)
    complete = sum(1 for r in runs.values() if r.get("status") == "complete")
    running = sum(1 for r in runs.values() if r.get("status") == "running")

    header = Text.assemble(
        ("minibox agent dashboard", "bold white"),
        ("  ·  ", "dim"),
        (f"{total} runs", "cyan"),
        ("  ", ""),
        (f"{complete} complete", "green"),
        ("  ", ""),
        *([( f"{running} running", "yellow")] if running else []),
    )

    console.print()
    console.print(Panel(header, border_style="dim cyan", padding=(0, 1)))
    console.print()
    console.print("[bold]Summary[/bold]", style="dim")
    console.print(build_summary_table(by_script))
    console.print()
    console.print("[bold]Recent Runs[/bold]", style="dim")
    console.print(build_history_table(list(runs.values())))
    console.print()


if __name__ == "__main__":
    main()
