"""CI failure diagnosis agent — orchestrates providers and Gitea issue creation."""

import json
import os
import sys
import urllib.request
import urllib.error

from .providers import DiagnosisUnavailable, ask_with_fallback
from .issues import (
    add_comment,
    create_issue,
    ensure_label_exists,
    find_issue_for_commit,
    set_commit_status,
)

MAX_LOG_BYTES = 30_000


def _require_env(key: str) -> str:
    val = os.environ.get(key)
    if not val:
        print(f"ERROR: required env var {key} is not set", file=sys.stderr)
        sys.exit(1)
    return val


def gitea_get(path: str, gitea_url: str, headers: dict) -> dict:
    req = urllib.request.Request(f"{gitea_url}/api/v1{path}", headers=headers)
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


def fetch_job_log(job_id: int, gitea_url: str, repo: str, headers: dict) -> str:
    url = f"{gitea_url}/api/v1/repos/{repo}/actions/jobs/{job_id}/logs"
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        return f"(log unavailable: {e})"
    text = raw.decode("utf-8", errors="replace")
    if len(text) > MAX_LOG_BYTES:
        text = "...[truncated]...\n" + text[-MAX_LOG_BYTES:]
    return text


def main() -> None:
    gitea_url = _require_env("GITEA_URL")
    token = _require_env("CI_AGENT_TOKEN")
    repo = _require_env("REPO")
    run_id = _require_env("RUN_ID")
    sha = _require_env("COMMIT_SHA")

    headers = {"Authorization": f"token {token}", "Content-Type": "application/json"}

    print(f"=== CI Diagnosis Agent — run {run_id} ===")

    jobs_data = gitea_get(
        f"/repos/{repo}/actions/runs/{run_id}/jobs?limit=50", gitea_url, headers
    )
    jobs = jobs_data.get("jobs", [])
    failed = [j for j in jobs if j.get("conclusion") == "failure"]
    if not failed:
        print("No failed jobs found — nothing to diagnose.")
        sys.exit(0)

    print(f"Failed jobs: {[j['name'] for j in failed]}")

    sections = []
    for job in failed:
        job_name = job.get("name", f"job-{job['id']}")
        print(f"  Fetching logs: {job_name} (id={job['id']})")
        log = fetch_job_log(job["id"], gitea_url, repo, headers)
        sections.append(f"### Job: {job_name}\n```\n{log}\n```")

    logs_text = "\n\n".join(sections)
    prompt = (
        "You are a Rust/CI expert. The following CI jobs failed in the minibox project "
        "(a Linux container runtime written in Rust, using cargo xtask for all CI tasks).\n\n"
        f"{logs_text}\n\n"
        "Analyze the failures and provide:\n"
        "1. **Root cause** — what exactly failed and why\n"
        "2. **Fix** — the minimal code or config change needed\n"
        "3. **Confidence** — how certain you are (high/medium/low)\n\n"
        "Be concise. Focus on actionable fixes."
    )

    print("\nAsking LLM for diagnosis...")
    set_commit_status(gitea_url, repo, sha, "pending", "CI diagnosis in progress...", headers)

    try:
        diagnosis, provider = ask_with_fallback(prompt)
    except DiagnosisUnavailable as e:
        print(f"All LLM providers failed: {e}")
        diagnosis = f"All LLM providers failed — raw logs attached.\n\n{logs_text}"
        provider = "none"

    print("\n" + "=" * 60)
    print("DIAGNOSIS")
    print("=" * 60)
    print(diagnosis)
    print("=" * 60)

    first_line = diagnosis.split("\n")[0][:120]
    state = "error" if provider == "none" else "failure"
    set_commit_status(gitea_url, repo, sha, state, f"Diagnosed: {first_line}", headers)

    failed_job_names = [j.get("name", f"job-{j['id']}") for j in failed]
    try:
        ensure_label_exists(gitea_url, repo, headers)
        existing = find_issue_for_commit(gitea_url, repo, sha, headers)
        if existing:
            print(f"Appending to existing issue #{existing}")
            add_comment(gitea_url, repo, existing, diagnosis, provider, headers)
        else:
            number = create_issue(
                gitea_url, repo, sha, diagnosis,
                provider, failed_job_names, run_id, headers,
            )
            print(f"Opened issue #{number}")
    except Exception as e:
        print(f"Warning: could not create/update Gitea issue: {e}", file=sys.stderr)


if __name__ == "__main__":
    main()
