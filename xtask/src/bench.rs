//! `cargo xtask bench` — run criterion benchmarks, collect metrics, and generate
//! a performance dashboard with history tracking.
//!
//! Parses criterion output for timing stats (mean, median, p50/p95/p99),
//! writes results to `bench/results/` with JSON/CSV snapshots and an HTML
//! dashboard, and optionally compares against a tracked per-environment
//! baseline (`bench/baseline.<env>.json`).

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering as CmpOrd;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Write};
use std::path::Path;
use xshell::{Shell, cmd};

const DASHBOARD_TEMPLATE: &str = include_str!("perf_dashboard_template.html");

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct BenchMetrics {
    scenario: String,
    scenario_title: String,
    group: String,
    mean_ns: f64,
    median_ns: f64,
    std_dev_ns: f64,
    p50_ns: f64,
    p95_ns: f64,
    p99_ns: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunRecord {
    generated_at: String,
    git_rev: Option<String>,
    notes: Option<String>,
    metrics: Vec<BenchMetrics>,
}

#[derive(Debug, Serialize)]
struct HistoryFile {
    generated_at: String,
    history_limit: usize,
    runs: Vec<RunRecord>,
}

#[derive(Deserialize)]
struct EstimateFile {
    mean: EstimateEntry,
    median: EstimateEntry,
    std_dev: EstimateEntry,
}

#[derive(Deserialize)]
struct EstimateEntry {
    point_estimate: f64,
}

#[derive(Deserialize)]
struct SampleFile {
    times: Vec<f64>,
    iters: Vec<f64>,
}

// ---------------------------------------------------------------------------
// CLI options
// ---------------------------------------------------------------------------

/// Default regression threshold when `--threshold` is not given.
const DEFAULT_THRESHOLD_PCT: f64 = 15.0;

/// Options for `cargo xtask bench`, parsed from the argv tail.
#[derive(Debug)]
pub struct BenchOpts {
    pub skip_bench: bool,
    pub check: bool,
    pub save_baseline: bool,
    pub threshold_pct: Option<f64>,
    pub env: String,
}

/// Parse `cargo xtask bench` flags. Unrecognized flags are warned about and
/// ignored, consistent with the other xtask arg parsers.
pub fn parse_bench_args(rest: &[String]) -> BenchOpts {
    let mut opts = BenchOpts {
        skip_bench: false,
        check: false,
        save_baseline: false,
        threshold_pct: None,
        env: "local".to_string(),
    };
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--skip-bench" => opts.skip_bench = true,
            "--check" => opts.check = true,
            "--save-baseline" => opts.save_baseline = true,
            "--threshold" => match rest.get(i + 1).and_then(|v| v.parse::<f64>().ok()) {
                Some(v) => {
                    opts.threshold_pct = Some(v);
                    i += 1;
                }
                None => eprintln!("warning: --threshold expects a numeric value; ignoring"),
            },
            "--env" => match rest.get(i + 1) {
                Some(v) => {
                    opts.env.clone_from(v);
                    i += 1;
                }
                None => eprintln!("warning: --env expects a value; ignoring"),
            },
            other => eprintln!("warning: unrecognized bench flag {other:?}; ignoring"),
        }
        i += 1;
    }
    opts
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn bench(sh: &Shell, root: &Path, opts: &BenchOpts) -> Result<()> {
    let history_limit: usize = 30;

    let results_dir = root.join("bench/results");
    let history_dir = results_dir.join("history");
    fs::create_dir_all(&history_dir).context("create bench/results/history")?;

    let criterion_dir = root.join("target/criterion");

    if !opts.skip_bench {
        eprintln!("$ cargo bench -p minibox-bench");
        cmd!(sh, "cargo bench -p minibox-bench -- --noplot").run()?;
    }

    // Discover all bench functions from criterion output
    let metrics = collect_all_metrics(&criterion_dir)?;

    if metrics.is_empty() {
        if opts.skip_bench {
            eprintln!(
                "no criterion results found in {}; nothing to report. \
                 Run without --skip-bench to generate them.",
                criterion_dir.display()
            );
            return Ok(());
        }
        return Err(anyhow!(
            "no criterion results found in {}. Run without --skip-bench first.",
            criterion_dir.display()
        ));
    }

    let commit = cmd!(sh, "git rev-parse --short HEAD").read().ok();
    let timestamp = Utc::now();
    let timestamp_iso = timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let run_record = RunRecord {
        generated_at: timestamp_iso.clone(),
        git_rev: commit,
        notes: None,
        metrics,
    };

    // Write latest + history snapshot
    let filename_stamp = timestamp.format("%Y%m%dT%H%M%SZ").to_string();
    write_json(&results_dir.join("latest.json"), &run_record)?;
    write_csv(&results_dir.join("latest.csv"), &run_record.metrics)?;
    write_json(
        &history_dir.join(format!("{filename_stamp}.json")),
        &run_record,
    )?;

    // Append to bench.jsonl for backwards compat
    let jsonl_path = results_dir.join("bench.jsonl");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&jsonl_path)
        .context("open bench.jsonl")?;
    writeln!(f, "{}", serde_json::to_string(&run_record)?)?;

    // Prune + build history
    prune_history(&history_dir, history_limit)?;
    let history_runs = load_history_runs(&history_dir, history_limit)?;
    let timeline = HistoryFile {
        generated_at: timestamp_iso,
        history_limit,
        runs: history_runs,
    };
    write_json(&results_dir.join("history.json"), &timeline)?;

    // Render dashboard
    let dashboard_html = render_dashboard(&timeline)?;
    let index_path = results_dir.join("index.html");
    fs::write(&index_path, dashboard_html.as_bytes())
        .with_context(|| format!("write {}", index_path.display()))?;

    eprintln!(
        "Bench dashboard updated -> {}",
        results_dir.join("index.html").display()
    );

    let baseline_path = root.join(format!("bench/baseline.{}.json", opts.env));

    if opts.check {
        check_against_baseline(&run_record, &baseline_path, opts)?;
    }

    if opts.save_baseline {
        if let Some(parent) = baseline_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(results_dir.join("latest.json"), &baseline_path)
            .with_context(|| format!("copy latest.json to {}", baseline_path.display()))?;
        eprintln!("Baseline saved -> {}", baseline_path.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Baseline comparison
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct BaselineDelta {
    scenario: String,
    group: String,
    baseline_mean_ns: f64,
    current_mean_ns: f64,
    delta_pct: f64,
    regressed: bool,
}

/// Compare the latest run against a baseline run.
///
/// Per scenario, the effective threshold is
/// `t = threshold_pct.max(2.0 * baseline_cv_pct)` where the coefficient of
/// variation widens the band for noisy scenarios. A scenario regresses when
/// `current_mean_ns > baseline_mean_ns * (1 + t/100)`. A scenario present in
/// the baseline but missing from the latest run is a synthetic regression
/// (inventory collapse); a scenario only in the latest run is informational.
fn compare_to_baseline(
    latest: &RunRecord,
    baseline: &RunRecord,
    threshold_pct: f64,
) -> Vec<BaselineDelta> {
    let mut deltas = Vec::new();

    for base in &baseline.metrics {
        let current = latest
            .metrics
            .iter()
            .find(|m| m.group == base.group && m.scenario == base.scenario);
        match current {
            Some(cur) => {
                let cv_pct = (base.std_dev_ns / base.mean_ns) * 100.0;
                let t = threshold_pct.max(2.0 * cv_pct);
                let regressed = cur.mean_ns > base.mean_ns * (1.0 + t / 100.0);
                deltas.push(BaselineDelta {
                    scenario: base.scenario.clone(),
                    group: base.group.clone(),
                    baseline_mean_ns: base.mean_ns,
                    current_mean_ns: cur.mean_ns,
                    delta_pct: (cur.mean_ns - base.mean_ns) / base.mean_ns * 100.0,
                    regressed,
                });
            }
            None => {
                // Inventory collapse: the scenario disappeared from the run.
                deltas.push(BaselineDelta {
                    scenario: base.scenario.clone(),
                    group: base.group.clone(),
                    baseline_mean_ns: base.mean_ns,
                    current_mean_ns: 0.0,
                    delta_pct: -100.0,
                    regressed: true,
                });
            }
        }
    }

    for cur in &latest.metrics {
        let in_baseline = baseline
            .metrics
            .iter()
            .any(|m| m.group == cur.group && m.scenario == cur.scenario);
        if !in_baseline {
            deltas.push(BaselineDelta {
                scenario: cur.scenario.clone(),
                group: cur.group.clone(),
                baseline_mean_ns: 0.0,
                current_mean_ns: cur.mean_ns,
                delta_pct: 0.0,
                regressed: false,
            });
        }
    }

    deltas.sort_by(|a, b| a.group.cmp(&b.group).then(a.scenario.cmp(&b.scenario)));
    deltas
}

fn check_against_baseline(
    run_record: &RunRecord,
    baseline_path: &Path,
    opts: &BenchOpts,
) -> Result<()> {
    let data = match fs::read_to_string(baseline_path) {
        Ok(data) => data,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            eprintln!(
                "No baseline at {} — bootstrap run; save one with \
                 `cargo xtask bench --save-baseline --env {}`.",
                baseline_path.display(),
                opts.env
            );
            return Ok(());
        }
        Err(err) => {
            return Err(err).with_context(|| format!("read {}", baseline_path.display()));
        }
    };
    let baseline: RunRecord = serde_json::from_str(&data)
        .with_context(|| format!("parse {}", baseline_path.display()))?;

    let threshold = opts.threshold_pct.unwrap_or(DEFAULT_THRESHOLD_PCT);
    let deltas = compare_to_baseline(run_record, &baseline, threshold);
    print_delta_table(&deltas);

    let regressed: Vec<String> = deltas
        .iter()
        .filter(|d| d.regressed)
        .map(|d| format!("{}/{}", d.group, d.scenario))
        .collect();
    if regressed.is_empty() {
        eprintln!(
            "Baseline check passed vs {} (threshold {threshold}%).",
            baseline_path.display()
        );
        Ok(())
    } else {
        bail!(
            "bench regression vs {}: {}",
            baseline_path.display(),
            regressed.join(", ")
        );
    }
}

fn print_delta_table(deltas: &[BaselineDelta]) {
    let name_width = deltas
        .iter()
        .map(|d| d.group.len() + 1 + d.scenario.len())
        .chain(std::iter::once("scenario".len()))
        .max()
        .unwrap_or(8);
    eprintln!(
        "{:<name_width$}  {:>16}  {:>16}  {:>10}  status",
        "scenario", "baseline_ns", "current_ns", "delta_pct"
    );
    for d in deltas {
        let name = format!("{}/{}", d.group, d.scenario);
        let status = if d.regressed { "REGRESSED" } else { "ok" };
        eprintln!(
            "{name:<name_width$}  {:>16.2}  {:>16.2}  {:>+9.2}%  {status}",
            d.baseline_mean_ns, d.current_mean_ns, d.delta_pct
        );
    }
}

// ---------------------------------------------------------------------------
// Criterion result collection
// ---------------------------------------------------------------------------

fn collect_all_metrics(criterion_dir: &Path) -> Result<Vec<BenchMetrics>> {
    let mut metrics = Vec::new();

    if !criterion_dir.exists() {
        return Ok(metrics);
    }

    // Walk criterion output: each bench function has a directory with
    // estimates.json and sample.json (or raw.csv) under <group>/<bench>/new/
    for group_entry in fs::read_dir(criterion_dir)?.flatten() {
        let group_path = group_entry.path();
        if !group_path.is_dir() {
            continue;
        }
        let group_name = group_entry.file_name().to_string_lossy().to_string();
        if group_name == "report" {
            continue;
        }

        // Check if this is a flat bench (estimates.json directly in new/)
        let new_dir = group_path.join("new");
        if new_dir.join("estimates.json").exists() {
            if let Ok(m) = collect_single_metric(&group_name, &group_name, &new_dir) {
                metrics.push(m);
            }
            continue;
        }

        // Otherwise walk sub-benchmarks
        for bench_entry in fs::read_dir(&group_path)?.flatten() {
            let bench_path = bench_entry.path();
            if !bench_path.is_dir() {
                continue;
            }
            let bench_name = bench_entry.file_name().to_string_lossy().to_string();
            if bench_name == "report" {
                continue;
            }
            let case_new = bench_path.join("new");
            if case_new.join("estimates.json").exists()
                && let Ok(m) = collect_single_metric(&bench_name, &group_name, &case_new)
            {
                metrics.push(m);
            }
        }
    }

    metrics.sort_by(|a, b| a.group.cmp(&b.group).then(a.scenario.cmp(&b.scenario)));
    Ok(metrics)
}

fn collect_single_metric(name: &str, group: &str, new_dir: &Path) -> Result<BenchMetrics> {
    let estimate_path = new_dir.join("estimates.json");
    let data = fs::read_to_string(&estimate_path)
        .with_context(|| format!("read {}", estimate_path.display()))?;
    let estimates: EstimateFile = serde_json::from_str(&data)
        .with_context(|| format!("parse {}", estimate_path.display()))?;

    let mut samples = load_samples(new_dir)?;
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(CmpOrd::Equal));

    Ok(BenchMetrics {
        scenario: name.to_string(),
        scenario_title: name.replace('_', " "),
        group: group.to_string(),
        mean_ns: estimates.mean.point_estimate,
        median_ns: estimates.median.point_estimate,
        std_dev_ns: estimates.std_dev.point_estimate,
        p50_ns: quantile(&samples, 0.5),
        p95_ns: quantile(&samples, 0.95),
        p99_ns: quantile(&samples, 0.99),
    })
}

// ---------------------------------------------------------------------------
// Sample loading (supports both raw.csv and sample.json)
// ---------------------------------------------------------------------------

fn load_samples(case_dir: &Path) -> Result<Vec<f64>> {
    let raw_path = case_dir.join("raw.csv");
    match File::open(&raw_path) {
        Ok(file) => {
            load_samples_from_raw(file).with_context(|| format!("parse {}", raw_path.display()))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            load_samples_from_json(&case_dir.join("sample.json"))
        }
        Err(err) => Err(err).with_context(|| format!("open {}", raw_path.display())),
    }
}

fn load_samples_from_raw(raw_file: File) -> Result<Vec<f64>> {
    let mut reader = BufReader::new(raw_file);
    let mut line = String::new();
    let mut samples = Vec::new();
    while reader.read_line(&mut line)? != 0 {
        if line.starts_with("group") {
            line.clear();
            continue;
        }
        if let Ok(sample) = parse_sample_value(&line) {
            samples.push(sample);
        }
        line.clear();
    }
    if samples.is_empty() {
        Err(anyhow!("raw.csv contained no valid samples"))
    } else {
        Ok(samples)
    }
}

fn load_samples_from_json(path: &Path) -> Result<Vec<f64>> {
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let sample: SampleFile =
        serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
    if sample.times.len() != sample.iters.len() {
        return Err(anyhow!(
            "sample.json times ({}) and iters ({}) lengths differ",
            sample.times.len(),
            sample.iters.len()
        ));
    }
    let values: Vec<f64> = sample
        .times
        .iter()
        .zip(sample.iters.iter())
        .filter(|(_, iters)| **iters > 0.0)
        .map(|(time, iters)| time / iters)
        .collect();
    if values.is_empty() {
        Err(anyhow!("sample.json contained no valid samples"))
    } else {
        Ok(values)
    }
}

fn parse_sample_value(line: &str) -> Result<f64> {
    let parts: Vec<&str> = line.trim_end().split(',').collect();
    if parts.len() < 8 {
        return Err(anyhow!("row had {} columns, expected >= 8", parts.len()));
    }
    let raw_value: f64 = parts[5]
        .parse()
        .context("non-numeric sample_measured_value")?;
    let iterations: f64 = parts[7].parse().context("non-numeric iteration_count")?;
    if iterations > 0.0 {
        Ok(raw_value / iterations)
    } else {
        Ok(raw_value)
    }
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lower = pos.floor() as usize;
    let upper = pos.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let weight = pos - lower as f64;
        sorted[upper].mul_add(weight, sorted[lower] * (1.0 - weight))
    }
}

// ---------------------------------------------------------------------------
// Output writers
// ---------------------------------------------------------------------------

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(file), value)
        .with_context(|| format!("write {}", path.display()))
}

fn write_csv(path: &Path, metrics: &[BenchMetrics]) -> Result<()> {
    let mut w =
        BufWriter::new(File::create(path).with_context(|| format!("create {}", path.display()))?);
    writeln!(
        w,
        "scenario,group,mean_ns,median_ns,std_dev_ns,p50_ns,p95_ns,p99_ns"
    )?;
    for m in metrics {
        writeln!(
            w,
            "{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2}",
            m.scenario, m.group, m.mean_ns, m.median_ns, m.std_dev_ns, m.p50_ns, m.p95_ns, m.p99_ns
        )?;
    }
    w.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// History management
// ---------------------------------------------------------------------------

fn prune_history(history_dir: &Path, limit: usize) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(history_dir)?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    while entries.len() > limit {
        let entry = entries.remove(0);
        fs::remove_file(entry.path()).ok();
    }
    Ok(())
}

fn load_history_runs(history_dir: &Path, limit: usize) -> Result<Vec<RunRecord>> {
    let mut entries: Vec<_> = fs::read_dir(history_dir)?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries.reverse();
    entries.truncate(limit);

    let mut runs = Vec::new();
    for entry in entries {
        let path = entry.path();
        let data = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let record: RunRecord =
            serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))?;
        runs.push(record);
    }
    Ok(runs)
}

// ---------------------------------------------------------------------------
// Dashboard rendering
// ---------------------------------------------------------------------------

fn render_dashboard(history: &HistoryFile) -> Result<String> {
    let json = serde_json::to_string(history).context("serialize history for dashboard")?;
    Ok(DASHBOARD_TEMPLATE.replace("__DATA_PLACEHOLDER__", &json))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod bench_args_tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn defaults_when_no_flags() {
        let opts = parse_bench_args(&[]);
        assert!(!opts.skip_bench);
        assert!(!opts.check);
        assert!(!opts.save_baseline);
        assert_eq!(opts.threshold_pct, None);
        assert_eq!(opts.env, "local");
    }

    #[test]
    fn check_env_threshold_parse_together() {
        // `cargo xtask bench --check --env hosted --threshold 20`
        let opts = parse_bench_args(&args(&["--check", "--env", "hosted", "--threshold", "20"]));
        assert!(opts.check);
        assert_eq!(opts.env, "hosted");
        assert_eq!(opts.threshold_pct, Some(20.0));
        assert!(!opts.skip_bench);
        assert!(!opts.save_baseline);
    }

    #[test]
    fn skip_bench_and_save_baseline_parse() {
        let opts = parse_bench_args(&args(&["--skip-bench", "--save-baseline"]));
        assert!(opts.skip_bench);
        assert!(opts.save_baseline);
        assert!(!opts.check);
    }

    #[test]
    fn unrecognized_flags_are_ignored() {
        let opts = parse_bench_args(&args(&["--bogus", "--check"]));
        assert!(opts.check, "known flags after an unknown one still parse");
        assert_eq!(opts.env, "local");
    }

    #[test]
    fn threshold_without_value_is_ignored() {
        let opts = parse_bench_args(&args(&["--threshold"]));
        assert_eq!(opts.threshold_pct, None);
    }

    #[test]
    fn threshold_with_non_numeric_value_is_ignored() {
        let opts = parse_bench_args(&args(&["--threshold", "fast", "--check"]));
        assert_eq!(opts.threshold_pct, None);
        assert!(opts.check);
    }
}

#[cfg(test)]
mod baseline_compare_tests {
    use super::*;

    fn metric(group: &str, scenario: &str, mean_ns: f64, std_dev_ns: f64) -> BenchMetrics {
        BenchMetrics {
            scenario: scenario.to_string(),
            scenario_title: scenario.replace('_', " "),
            group: group.to_string(),
            mean_ns,
            median_ns: mean_ns,
            std_dev_ns,
            p50_ns: mean_ns,
            p95_ns: mean_ns,
            p99_ns: mean_ns,
        }
    }

    fn record(metrics: Vec<BenchMetrics>) -> RunRecord {
        RunRecord {
            generated_at: "2026-07-07T00:00:00Z".to_string(),
            git_rev: None,
            notes: None,
            metrics,
        }
    }

    #[allow(clippy::expect_used)]
    fn find<'a>(deltas: &'a [BaselineDelta], group: &str, scenario: &str) -> &'a BaselineDelta {
        deltas
            .iter()
            .find(|d| d.group == group && d.scenario == scenario)
            .expect("delta present")
    }

    #[test]
    fn within_threshold_is_not_regressed() {
        let baseline = record(vec![metric("pull", "one_layer", 1000.0, 10.0)]);
        let latest = record(vec![metric("pull", "one_layer", 1100.0, 10.0)]);
        // +10% < 15% default threshold
        let deltas = compare_to_baseline(&latest, &baseline, 15.0);
        let d = find(&deltas, "pull", "one_layer");
        assert!(!d.regressed);
        assert!((d.delta_pct - 10.0).abs() < 1e-9);
    }

    #[test]
    fn above_threshold_is_regressed() {
        let baseline = record(vec![metric("pull", "one_layer", 1000.0, 10.0)]);
        let latest = record(vec![metric("pull", "one_layer", 1200.0, 10.0)]);
        // +20% > 15% threshold; baseline CV is 1% so it does not widen the band
        let deltas = compare_to_baseline(&latest, &baseline, 15.0);
        let d = find(&deltas, "pull", "one_layer");
        assert!(d.regressed);
        assert!((d.delta_pct - 20.0).abs() < 1e-9);
    }

    #[test]
    fn noisy_baseline_widens_the_band() {
        // CV = 200/1000 = 20% -> effective threshold = max(15, 2*20) = 40%
        let baseline = record(vec![metric("pull", "noisy", 1000.0, 200.0)]);
        let latest = record(vec![metric("pull", "noisy", 1300.0, 200.0)]);
        let deltas = compare_to_baseline(&latest, &baseline, 15.0);
        assert!(
            !find(&deltas, "pull", "noisy").regressed,
            "+30% must pass under the CV-widened 40% band"
        );

        // But +50% exceeds even the widened band.
        let latest = record(vec![metric("pull", "noisy", 1500.0, 200.0)]);
        let deltas = compare_to_baseline(&latest, &baseline, 15.0);
        assert!(find(&deltas, "pull", "noisy").regressed);
    }

    #[test]
    fn explicit_threshold_overrides_default() {
        let baseline = record(vec![metric("pull", "one_layer", 1000.0, 1.0)]);
        let latest = record(vec![metric("pull", "one_layer", 1100.0, 1.0)]);
        // +10% regresses under a 5% threshold
        let deltas = compare_to_baseline(&latest, &baseline, 5.0);
        assert!(find(&deltas, "pull", "one_layer").regressed);
    }

    #[test]
    fn baseline_only_scenario_is_synthetic_regression() {
        let baseline = record(vec![metric("pull", "vanished", 1000.0, 10.0)]);
        let latest = record(vec![]);
        let deltas = compare_to_baseline(&latest, &baseline, 15.0);
        let d = find(&deltas, "pull", "vanished");
        assert!(d.regressed, "inventory collapse must fail the check");
        assert!((d.baseline_mean_ns - 1000.0).abs() < 1e-9);
        assert!((d.current_mean_ns - 0.0).abs() < 1e-9);
    }

    #[test]
    fn latest_only_scenario_is_informational() {
        let baseline = record(vec![]);
        let latest = record(vec![metric("pull", "brand_new", 1000.0, 10.0)]);
        let deltas = compare_to_baseline(&latest, &baseline, 15.0);
        let d = find(&deltas, "pull", "brand_new");
        assert!(!d.regressed, "new scenarios are informational");
        assert!((d.current_mean_ns - 1000.0).abs() < 1e-9);
        assert!((d.baseline_mean_ns - 0.0).abs() < 1e-9);
    }

    #[test]
    fn scenarios_match_on_group_and_scenario() {
        // Same scenario name in two groups must not cross-match.
        let baseline = record(vec![metric("pull", "same_name", 1000.0, 10.0)]);
        let latest = record(vec![metric("extract", "same_name", 5000.0, 10.0)]);
        let deltas = compare_to_baseline(&latest, &baseline, 15.0);
        assert!(find(&deltas, "pull", "same_name").regressed, "missing");
        assert!(!find(&deltas, "extract", "same_name").regressed, "new");
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn old_run_json_with_memory_field_still_parses() {
        // Pre-removal history files carry memory_peak_bytes; serde must ignore it.
        let json = r#"{
            "generated_at": "2026-07-01T00:00:00Z",
            "git_rev": "abc1234",
            "notes": null,
            "metrics": [{
                "scenario": "s",
                "scenario_title": "s",
                "group": "g",
                "mean_ns": 1.0,
                "median_ns": 1.0,
                "std_dev_ns": 0.1,
                "p50_ns": 1.0,
                "p95_ns": 1.0,
                "p99_ns": 1.0,
                "memory_peak_bytes": 0
            }]
        }"#;
        let record: RunRecord = serde_json::from_str(json).expect("old schema parses");
        assert_eq!(record.metrics.len(), 1);
    }
}
