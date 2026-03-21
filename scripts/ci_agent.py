#!/usr/bin/env python3
"""CI failure diagnosis agent — fetches failed job logs and asks Claude to diagnose."""

import json
import os
import sys
import urllib.request
import urllib.error

GITEA_URL = os.environ["GITEA_URL"]
GITEA_TOKEN = os.environ["CI_AGENT_TOKEN"]
REPO = os.environ["REPO"]
RUN_ID = os.environ["RUN_ID"]
COMMIT_SHA = os.environ["COMMIT_SHA"]
ANTHROPIC_API_KEY = os.environ["ANTHROPIC_API_KEY"]

GITEA_HEADERS = {"Authorization": f"token {GITEA_TOKEN}", "Content-Type": "application/json"}
ANTHROPIC_HEADERS = {
    "x-api-key": ANTHROPIC_API_KEY,
    "anthropic-version": "2023-06-01",
    "content-type": "application/json",
}

MAX_LOG_BYTES = 30_000  # keep prompt under context limits


def gitea_get(path: str) -> dict:
    req = urllib.request.Request(f"{GITEA_URL}/api/v1{path}", headers=GITEA_HEADERS)
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


def gitea_post(path: str, body: dict) -> None:
    data = json.dumps(body).encode()
    req = urllib.request.Request(
        f"{GITEA_URL}/api/v1{path}", data=data, headers=GITEA_HEADERS, method="POST"
    )
    with urllib.request.urlopen(req) as r:
        r.read()


def fetch_job_log(job_id: int) -> str:
    url = f"{GITEA_URL}/api/v1/repos/{REPO}/actions/jobs/{job_id}/logs"
    req = urllib.request.Request(url, headers=GITEA_HEADERS)
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        return f"(log unavailable: {e})"
    text = raw.decode("utf-8", errors="replace")
    if len(text) > MAX_LOG_BYTES:
        # Keep the tail — errors tend to be at the end
        text = "...[truncated]...\n" + text[-MAX_LOG_BYTES:]
    return text


def ask_claude(prompt: str) -> str:
    body = {
        "model": "claude-opus-4-6",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": prompt}],
    }
    data = json.dumps(body).encode()
    req = urllib.request.Request(
        "https://api.anthropic.com/v1/messages",
        data=data,
        headers=ANTHROPIC_HEADERS,
        method="POST",
    )
    with urllib.request.urlopen(req) as r:
        resp = json.loads(r.read())
    return next(b["text"] for b in resp["content"] if b["type"] == "text")


def set_commit_status(state: str, description: str) -> None:
    """Post a commit status — visible in PR/commit view."""
    try:
        gitea_post(
            f"/repos/{REPO}/statuses/{COMMIT_SHA}",
            {
                "context": "ci/agent-diagnosis",
                "state": state,
                "description": description[:140],
            },
        )
    except Exception as e:
        print(f"  (could not post commit status: {e})")


def main() -> None:
    print(f"=== CI Diagnosis Agent — run {RUN_ID} ===")

    # Fetch all jobs for this run
    jobs_data = gitea_get(f"/repos/{REPO}/actions/runs/{RUN_ID}/jobs?limit=50")
    jobs = jobs_data.get("jobs", [])

    failed = [j for j in jobs if j.get("conclusion") == "failure"]
    if not failed:
        print("No failed jobs found — nothing to diagnose.")
        sys.exit(0)

    print(f"Failed jobs: {[j['name'] for j in failed]}")

    # Build log context
    sections = []
    for job in failed:
        job_id = job["id"]
        job_name = job.get("name", f"job-{job_id}")
        print(f"  Fetching logs: {job_name} (id={job_id})")
        log = fetch_job_log(job_id)
        sections.append(f"### Job: {job_name}\n```\n{log}\n```")

    logs_text = "\n\n".join(sections)

    prompt = f"""You are a Rust/CI expert. The following CI jobs failed in the minibox project \
(a Linux container runtime written in Rust, using cargo xtask for all CI tasks).

{logs_text}

Analyze the failures and provide:
1. **Root cause** — what exactly failed and why
2. **Fix** — the minimal code or config change needed
3. **Confidence** — how certain you are (high/medium/low)

Be concise. Focus on actionable fixes."""

    print("\nAsking Claude for diagnosis...")
    set_commit_status("pending", "CI diagnosis in progress...")

    try:
        diagnosis = ask_claude(prompt)
        print("\n" + "=" * 60)
        print("DIAGNOSIS")
        print("=" * 60)
        print(diagnosis)
        print("=" * 60)
        # Post a brief status — full diagnosis is in this job's log
        first_line = diagnosis.split("\n")[0][:120]
        set_commit_status("failure", f"Diagnosed: {first_line}")
    except Exception as e:
        print(f"Claude API error: {e}")
        set_commit_status("error", f"Diagnosis failed: {e}"[:140])
        sys.exit(1)


if __name__ == "__main__":
    main()
