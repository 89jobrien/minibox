# /// script
# requires-python = ">=3.11"
# dependencies = [
# ]
# ///
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
from datetime import datetime, timezone
from html import escape
from pathlib import Path


def test_build_index(tmp_path: Path) -> None:
    (tmp_path / "2026-03-17T00-00-00Z").mkdir()
    index = build_index(tmp_path)
    assert index["runs"][0]["id"] == "2026-03-17T00-00-00Z"


def test_build_index_includes_benches(tmp_path: Path) -> None:
    run_dir = tmp_path / "2026-03-17T00-00-01Z"
    run_dir.mkdir()
    benches = {
        "benches": [
            {
                "name": "trait_overhead",
                "report": "criterion/trait_overhead/report/index.html",
            }
        ]
    }
    (run_dir / "benches.json").write_text(json.dumps(benches))
    index = build_index(tmp_path)
    assert index["runs"][0]["benches"][0]["name"] == "trait_overhead"


def test_parse_tests_jsonl_counts() -> None:
    sample = "\n".join(
        [
            json.dumps({"type": "test", "event": "ok", "name": "t1"}),
            json.dumps({"type": "test", "event": "failed", "name": "t2"}),
            json.dumps({"type": "test", "event": "ignored", "name": "t3"}),
        ]
    )
    stats = parse_tests_jsonl(sample)
    assert stats["total"] == 3
    assert stats["ok"] == 1
    assert stats["failed"] == 1
    assert stats["ignored"] == 1


def test_render_report(tmp_path: Path) -> None:
    run_dir = tmp_path / "2026-03-17T00-00-02Z"
    run_dir.mkdir()
    (run_dir / "meta.json").write_text(
        json.dumps(
            {
                "timestamp": run_dir.name,
                "git_sha": "deadbeef",
                "branch": "main",
            }
        )
    )
    (run_dir / "tests.jsonl").write_text(
        json.dumps({"type": "test", "event": "ok", "name": "t1"})
    )
    (run_dir / "benches.json").write_text(
        json.dumps({"benches": [{"name": "trait_overhead", "report": "x"}]})
    )
    render_report(tmp_path)
    report = (tmp_path / "report.html").read_text()
    assert "2026-03-17T00-00-02Z" in report
    assert "trait_overhead" in report


def run_self_test() -> None:
    import tempfile

    with tempfile.TemporaryDirectory() as td:
        tmp_path = Path(td)
        test_build_index(tmp_path)
        test_build_index_includes_benches(tmp_path)
        test_parse_tests_jsonl_counts()
        test_render_report(tmp_path)

    print("ok")


def build_index(reports_dir: Path) -> dict:
    runs = []
    if reports_dir.exists():
        for entry in sorted(reports_dir.iterdir(), reverse=True):
            if entry.is_dir():
                run = {"id": entry.name}
                benches_path = entry / "benches.json"
                if benches_path.exists():
                    run["benches"] = json.loads(benches_path.read_text()).get("benches", [])
                runs.append(run)
    return {"runs": runs}


def write_index(reports_dir: Path) -> None:
    index = build_index(reports_dir)
    (reports_dir / "index.json").write_text(json.dumps(index, indent=2))


def collect_benches(output_dir: Path) -> None:
    criterion_dir = Path("target/criterion")
    if not criterion_dir.exists():
        return
    dest = output_dir / "criterion"
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(criterion_dir, dest)
    benches = []
    for bench_dir in dest.iterdir():
        if not bench_dir.is_dir():
            continue
        report = bench_dir / "report" / "index.html"
        if report.exists():
            benches.append(
                {
                    "name": bench_dir.name,
                    "report": f"criterion/{bench_dir.name}/report/index.html",
                }
            )
    (output_dir / "benches.json").write_text(json.dumps({"benches": benches}, indent=2))


def collect_tests(output_dir: Path) -> None:
    result = subprocess.run(
        ["cargo", "test", "--", "--format", "json"],
        check=False,
        capture_output=True,
        text=True,
    )
    (output_dir / "tests.jsonl").write_text(result.stdout)
    if result.returncode != 0:
        raise RuntimeError("cargo test failed")


def git_output(args: list[str]) -> str:
    result = subprocess.run(["git", *args], check=False, capture_output=True, text=True)
    if result.returncode != 0:
        return "unknown"
    return result.stdout.strip()


def write_meta(output_dir: Path, error: str | None = None) -> None:
    meta = {
        "timestamp": output_dir.name,
        "git_sha": git_output(["rev-parse", "HEAD"]),
        "branch": git_output(["rev-parse", "--abbrev-ref", "HEAD"]),
    }
    if error:
        meta["error"] = error
    (output_dir / "meta.json").write_text(json.dumps(meta, indent=2))


def collect_run(reports_dir: Path) -> None:
    timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H-%M-%SZ")
    output_dir = reports_dir / timestamp
    output_dir.mkdir(parents=True, exist_ok=True)

    error = None
    try:
        collect_benches(output_dir)
        collect_tests(output_dir)
    except Exception as exc:
        error = str(exc)
    write_meta(output_dir, error=error)
    write_index(reports_dir)
    render_report(reports_dir)
    if error:
        raise SystemExit(error)


def parse_tests_jsonl(text: str) -> dict:
    results = {}
    for line in text.splitlines():
        if not line:
            continue
        try:
            payload = json.loads(line)
        except json.JSONDecodeError:
            continue
        if payload.get("type") != "test":
            continue
        name = payload.get("name", "unknown")
        event = payload.get("event")
        if event in {"ok", "failed", "ignored"}:
            results[name] = event

    ok = sum(1 for v in results.values() if v == "ok")
    failed = sum(1 for v in results.values() if v == "failed")
    ignored = sum(1 for v in results.values() if v == "ignored")
    failures = [k for k, v in results.items() if v == "failed"]
    return {"total": len(results), "ok": ok, "failed": failed, "ignored": ignored, "failures": failures}


def render_report(reports_dir: Path) -> None:
    index = build_index(reports_dir)
    runs_html = []
    for run in index["runs"]:
        run_dir = reports_dir / run["id"]
        meta = {}
        if (run_dir / "meta.json").exists():
            meta = json.loads((run_dir / "meta.json").read_text())
        tests_text = (run_dir / "tests.jsonl").read_text() if (run_dir / "tests.jsonl").exists() else ""
        stats = parse_tests_jsonl(tests_text)
        benches = []
        if (run_dir / "benches.json").exists():
            benches = json.loads((run_dir / "benches.json").read_text()).get("benches", [])

        benches_rows = "".join(
            f"<tr><td>{escape(b['name'])}</td><td><a href=\"{run['id']}/{escape(b['report'])}\">Report</a></td></tr>"
            for b in benches
        )
        benches_table = (
            f"<table><thead><tr><th>Benchmark</th><th>Report</th></tr></thead><tbody>{benches_rows}</tbody></table>"
            if benches_rows
            else "<div class=\"empty\">No benchmark reports found.</div>"
        )

        failures_list = (
            "<ul>" + "".join(f"<li>{escape(name)}</li>" for name in stats["failures"]) + "</ul>"
            if stats["failures"]
            else "<div class=\"empty\">No failures recorded.</div>"
        )

        runs_html.append(
            f"""
<section class="panel">
  <h2>{escape(run['id'])}</h2>
  <div class="meta">
    <div><strong>SHA:</strong> {escape(str(meta.get('git_sha', 'unknown')))}</div>
    <div><strong>Branch:</strong> {escape(str(meta.get('branch', 'unknown')))}</div>
    <div><strong>Timestamp:</strong> {escape(str(meta.get('timestamp', run['id'])))}</div>
  </div>
  <div class="grid">
    <div class="stat"><div>Total Tests</div><div class="value">{stats['total']}</div></div>
    <div class="stat"><div>Passed</div><div class="value">{stats['ok']}</div></div>
    <div class="stat"><div>Failed</div><div class="value">{stats['failed']}</div></div>
    <div class="stat"><div>Ignored</div><div class="value">{stats['ignored']}</div></div>
  </div>
  <h3>Bench Reports</h3>
  {benches_table}
  <h3>Test Failures</h3>
  {failures_list}
</section>
"""
        )

    fallback_html = '<div class="panel"><div class="empty">No runs found.</div></div>'
    runs_section = "".join(runs_html) if runs_html else fallback_html
    html = """<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Minibox Metrics Report</title>
  <style>
    :root {
      --bg: #0b0c10;
      --panel: #151820;
      --text: #e6e6e6;
      --muted: #9aa3b2;
      --accent: #67d3b4;
    }
    body {
      margin: 0;
      font-family: "IBM Plex Sans", "Segoe UI", sans-serif;
      background: radial-gradient(1200px 600px at 10% 0%, #182030, #0b0c10 55%);
      color: var(--text);
    }
    header {
      padding: 24px 32px;
      border-bottom: 1px solid #1f2430;
    }
    h1 {
      margin: 0 0 8px 0;
      font-size: 24px;
    }
    .sub {
      color: var(--muted);
      font-size: 14px;
    }
    main {
      padding: 24px 32px;
      display: grid;
      gap: 16px;
    }
    .panel {
      background: var(--panel);
      border: 1px solid #202636;
      border-radius: 10px;
      padding: 16px;
    }
    .meta {
      font-size: 13px;
      line-height: 1.6;
      margin-bottom: 12px;
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: 12px;
      margin-bottom: 12px;
    }
    .stat {
      background: #0f1218;
      border: 1px solid #1f2533;
      border-radius: 8px;
      padding: 12px;
    }
    .stat .value {
      font-size: 20px;
      margin-top: 6px;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      font-size: 13px;
    }
    th, td {
      text-align: left;
      padding: 8px 6px;
      border-bottom: 1px solid #202636;
    }
    a { color: var(--accent); text-decoration: none; }
    a:hover { text-decoration: underline; }
    .empty { color: var(--muted); font-style: italic; }
  </style>
</head>
<body>
  <header>
    <h1>Minibox Metrics Report</h1>
    <div class="sub">Static export with embedded runs</div>
  </header>
  <main>
    {RUNS}
  </main>
</body>
</html>"""
    html = html.replace("{RUNS}", runs_section)
    (reports_dir / "report.html").write_text(html)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--reports-dir", default="artifacts/reports")
    args = parser.parse_args()

    if args.self_test:
        run_self_test()
        return

    reports_dir = Path(args.reports_dir)
    reports_dir.mkdir(parents=True, exist_ok=True)
    collect_run(reports_dir)


if __name__ == "__main__":
    main()
