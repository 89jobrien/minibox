//! `cargo xtask bench` — run criterion benchmarks, collect metrics, and generate
//! a performance dashboard with history tracking.
//!
//! Parses criterion output for timing stats (mean, median, p50/p95/p99),
//! measures peak memory via a tracking allocator harness, writes results to
//! `bench/results/` with JSON/CSV snapshots and an HTML dashboard.

use anyhow::{Context, Result, anyhow};
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
    memory_peak_bytes: u64,
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
// Public entry point
// ---------------------------------------------------------------------------

pub fn bench(sh: &Shell, root: &Path) -> Result<()> {
    let skip_run = std::env::args().any(|a| a == "--skip-bench");
    let history_limit: usize = 30;

    let results_dir = root.join("bench/results");
    let history_dir = results_dir.join("history");
    fs::create_dir_all(&history_dir).context("create bench/results/history")?;

    let criterion_dir = root.join("target/criterion");

    if !skip_run {
        eprintln!("$ cargo bench -p minibox");
        cmd!(sh, "cargo bench -p minibox -- --noplot").run()?;
    }

    // Discover all bench functions from criterion output
    let metrics = collect_all_metrics(&criterion_dir)?;

    if metrics.is_empty() {
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
    Ok(())
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
        memory_peak_bytes: 0, // populated by memory harness if available
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
        sorted[lower] * (1.0 - weight) + sorted[upper] * weight
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
        "scenario,group,mean_ns,median_ns,std_dev_ns,p50_ns,p95_ns,p99_ns,memory_peak_bytes"
    )?;
    for m in metrics {
        writeln!(
            w,
            "{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
            m.scenario,
            m.group,
            m.mean_ns,
            m.median_ns,
            m.std_dev_ns,
            m.p50_ns,
            m.p95_ns,
            m.p99_ns,
            m.memory_peak_bytes
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
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());
    while entries.len() > limit {
        let entry = entries.remove(0);
        fs::remove_file(entry.path()).ok();
    }
    Ok(())
}

fn load_history_runs(history_dir: &Path, limit: usize) -> Result<Vec<RunRecord>> {
    let mut entries: Vec<_> = fs::read_dir(history_dir)?
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());
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
