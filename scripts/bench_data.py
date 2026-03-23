"""Shared bench data library — reads and analyzes minibox benchmark results.

Data sources:
  bench/results/bench.jsonl  — append-only history (one JSON object per run)
  bench/results/latest.json  — most recent run snapshot

Used by: bench-agent.py (AI analysis), dashboard.py (TUI display)
"""

import json
import statistics
from dataclasses import dataclass, field
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent.parent / "bench"
RESULTS_DIR = BENCH_DIR / "results"
JSONL_PATH = RESULTS_DIR / "bench.jsonl"
LATEST_PATH = RESULTS_DIR / "latest.json"
SCHEMA_PATH = BENCH_DIR.parent / "crates" / "minibox-bench" / "schema.json"


@dataclass
class TestResult:
    name: str
    iterations: int
    durations_micros: list[int]
    stats: dict | None = None
    unit: str = "micros"

    @property
    def avg_us(self) -> float | None:
        if self.stats and self.stats.get("avg") is not None:
            return float(self.stats["avg"])
        if self.durations_micros:
            return statistics.mean(self.durations_micros)
        return None

    @property
    def p95_us(self) -> float | None:
        if self.stats and self.stats.get("p95") is not None:
            return float(self.stats["p95"])
        if len(self.durations_micros) >= 2:
            sorted_d = sorted(self.durations_micros)
            idx = int(len(sorted_d) * 0.95)
            return float(sorted_d[min(idx, len(sorted_d) - 1)])
        return self.avg_us

    @property
    def min_us(self) -> float | None:
        if self.stats and self.stats.get("min") is not None:
            return float(self.stats["min"])
        if self.durations_micros:
            return float(min(self.durations_micros))
        return None


@dataclass
class SuiteResult:
    name: str
    tests: list[TestResult]


@dataclass
class BenchRun:
    git_sha: str
    hostname: str
    timestamp: str
    minibox_version: str
    suites: list[SuiteResult]
    errors: list[str]

    @property
    def is_valid(self) -> bool:
        return any(t.iterations > 0 for s in self.suites for t in s.tests)

    @property
    def is_vps(self) -> bool:
        return self.hostname == "jobrien"

    def test_by_name(self, suite: str, test: str) -> TestResult | None:
        for s in self.suites:
            if s.name == suite:
                for t in s.tests:
                    if t.name == test:
                        return t
        return None


@dataclass
class Regression:
    suite: str
    test: str
    prev_avg_us: float
    curr_avg_us: float
    baseline_kind: str = "prev"

    @property
    def pct_change(self) -> float:
        if self.prev_avg_us == 0:
            return 0.0
        return ((self.curr_avg_us - self.prev_avg_us) / self.prev_avg_us) * 100

    @property
    def is_regression(self) -> bool:
        return self.pct_change > 10.0

    @property
    def is_improvement(self) -> bool:
        return self.pct_change < -10.0


def _parse_run(data: dict) -> BenchRun:
    meta = data.get("metadata", {})
    suites = []
    for s in data.get("suites", []):
        tests = []
        for t in s.get("tests", []):
            tests.append(TestResult(
                name=t["name"],
                iterations=t.get("iterations", 0),
                durations_micros=t.get("durations_micros", []),
                stats=t.get("stats"),
                unit=t.get("unit", "micros"),
            ))
        suites.append(SuiteResult(name=s["name"], tests=tests))
    errors = data.get("errors", [])
    if isinstance(errors, list) and errors and isinstance(errors[0], dict):
        errors = [e.get("message", str(e)) for e in errors]
    return BenchRun(
        git_sha=meta.get("git_sha", "unknown"),
        hostname=meta.get("hostname", "unknown"),
        timestamp=meta.get("timestamp", ""),
        minibox_version=meta.get("minibox_version", "unknown"),
        suites=suites,
        errors=errors,
    )


def load_latest() -> BenchRun | None:
    if not LATEST_PATH.exists():
        return None
    data = json.loads(LATEST_PATH.read_text())
    return _parse_run(data)


def load_history() -> list[BenchRun]:
    if not JSONL_PATH.exists():
        return []
    runs = []
    for line in JSONL_PATH.read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            runs.append(_parse_run(json.loads(line)))
        except (json.JSONDecodeError, KeyError):
            continue
    return runs


def valid_vps_runs() -> list[BenchRun]:
    return [r for r in load_history() if r.is_valid and r.is_vps]


def valid_runs() -> list[BenchRun]:
    return [r for r in load_history() if r.is_valid]


def compare_runs(prev: BenchRun, curr: BenchRun) -> list[Regression]:
    diffs = []
    for suite in curr.suites:
        for test in suite.tests:
            prev_test = prev.test_by_name(suite.name, test.name)
            if prev_test and prev_test.avg_us and test.avg_us:
                diffs.append(Regression(
                    suite=suite.name,
                    test=test.name,
                    prev_avg_us=prev_test.avg_us,
                    curr_avg_us=test.avg_us,
                    baseline_kind="prev",
                ))
    return diffs


def detect_regressions(threshold_pct: float = 10.0) -> list[Regression]:
    runs = valid_vps_runs()
    if len(runs) < 2:
        return []
    curr = runs[-1]
    prior_runs = runs[:-1]
    regressions = []

    for suite in curr.suites:
        for test in suite.tests:
            if not test.avg_us:
                continue

            prior_avgs = []
            for run in prior_runs:
                prior_test = run.test_by_name(suite.name, test.name)
                if prior_test and prior_test.avg_us:
                    prior_avgs.append(prior_test.avg_us)

            if not prior_avgs:
                continue

            # Small-sample history is noisy. Treat a regression as "worse than
            # the worst prior VPS sample by the threshold", not merely worse
            # than the immediately previous run.
            worst_prior_avg = max(prior_avgs)
            regression = Regression(
                suite=suite.name,
                test=test.name,
                prev_avg_us=worst_prior_avg,
                curr_avg_us=test.avg_us,
                baseline_kind="worst_prior",
            )
            if regression.pct_change > threshold_pct:
                regressions.append(regression)

    return regressions


def format_duration(us: float | None) -> str:
    if us is None:
        return "—"
    if us < 1:
        return f"{us * 1000:.0f}ns"
    if us < 1000:
        return f"{us:.1f}us"
    if us < 1_000_000:
        return f"{us / 1000:.1f}ms"
    return f"{us / 1_000_000:.2f}s"


def format_pct(pct: float) -> str:
    sign = "+" if pct > 0 else ""
    return f"{sign}{pct:.1f}%"


def result_file_count() -> int:
    if not RESULTS_DIR.exists():
        return 0
    return sum(1 for _ in RESULTS_DIR.iterdir())


def result_dir_size_mb() -> float:
    if not RESULTS_DIR.exists():
        return 0.0
    total = sum(f.stat().st_size for f in RESULTS_DIR.rglob("*") if f.is_file())
    return total / (1024 * 1024)


def summary_text() -> str:
    latest = load_latest()
    if not latest:
        return "No bench results available."

    lines = [f"Latest: {latest.git_sha[:8]} on {latest.hostname} ({latest.timestamp[:19]})"]
    for suite in latest.suites:
        for test in suite.tests:
            avg = format_duration(test.avg_us)
            p95 = format_duration(test.p95_us)
            lines.append(f"  {suite.name}/{test.name}: avg={avg} p95={p95} ({test.iterations} iter)")

    regressions = detect_regressions()
    if regressions:
        lines.append(f"\nRegressions ({len(regressions)}, vs worst prior VPS run):")
        for r in regressions:
            lines.append(f"  {r.suite}/{r.test}: {format_duration(r.prev_avg_us)} -> {format_duration(r.curr_avg_us)} ({format_pct(r.pct_change)})")

    history = valid_vps_runs()
    lines.append(f"\nHistory: {len(history)} valid VPS runs, {result_file_count()} files ({result_dir_size_mb():.1f} MB)")
    return "\n".join(lines)
