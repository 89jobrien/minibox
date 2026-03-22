"""Shared logging helpers for minibox AI agent scripts.

Writes to two sinks:
  ~/.mbx/agent-runs.jsonl   — structured JSONL telemetry (run start + completion)
  ~/.mbx/ai-logs/<sha>-<script>.md  — markdown output pinned to commit SHA
"""

import json
import subprocess
from datetime import datetime
from pathlib import Path

LOG_FILE = Path.home() / ".mbx" / "agent-runs.jsonl"
AI_LOGS_DIR = Path.home() / ".mbx" / "ai-logs"


def git_short_sha() -> str:
    return subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],
        capture_output=True, text=True,
    ).stdout.strip()


def log_start(script: str, args: dict) -> str:
    run_id = datetime.now().isoformat()
    LOG_FILE.parent.mkdir(parents=True, exist_ok=True)
    with LOG_FILE.open("a") as f:
        f.write(json.dumps({"run_id": run_id, "script": script, "args": args, "status": "running"}) + "\n")
    return run_id


def log_complete(run_id: str, script: str, args: dict, output: str, duration_s: float) -> None:
    with LOG_FILE.open("a") as f:
        f.write(json.dumps({
            "run_id": run_id,
            "script": script,
            "args": args,
            "status": "complete",
            "duration_s": round(duration_s, 2),
            "output": output,
        }) + "\n")


def save_commit_log(sha: str, script: str, content: str, meta: dict) -> Path:
    """Write LLM output to ~/.mbx/ai-logs/<sha>-<script>.md."""
    AI_LOGS_DIR.mkdir(parents=True, exist_ok=True)
    out = AI_LOGS_DIR / f"{sha}-{script}.md"
    header = (
        f"# {script} · {sha}\n\n"
        + "\n".join(f"- **{k}**: {v}" for k, v in meta.items())
        + "\n\n---\n\n"
    )
    out.write_text(header + content)
    return out
