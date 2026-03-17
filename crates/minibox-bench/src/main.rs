use serde::Serialize;

#[derive(Serialize, Default)]
struct BenchReport {
    metadata: Metadata,
    suites: Vec<SuiteResult>,
    errors: Vec<String>,
}

impl BenchReport {
    fn empty() -> Self {
        Self::default()
    }
}

#[derive(Serialize, Default)]
struct Metadata {
    timestamp: String,
    hostname: String,
    git_sha: String,
    minibox_version: String,
}

#[derive(Serialize, Default)]
struct SuiteResult {
    name: String,
    tests: Vec<TestResult>,
}

#[derive(Serialize, Default)]
struct TestResult {
    name: String,
    iterations: usize,
    durations_micros: Vec<u64>,
    stats: Option<Stats>,
}

fn write_json(report: &BenchReport, path: &str) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(report).unwrap();
    std::fs::write(path, json)
}

fn write_table(report: &BenchReport, path: &str) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("minibox benchmark results\n\n");
    out.push_str(&format!("suites: {}\n\n", report.suites.len()));
    out.push_str("suite\ttest\titers\tmin(ms)\tavg(ms)\tp95(ms)\n");
    for suite in &report.suites {
        for test in &suite.tests {
            let stats = test.stats.as_ref();
            let (min, avg, p95) = match stats {
                Some(s) => (
                    format!("{:.3}", s.min as f64 / 1000.0),
                    format!("{:.3}", s.avg as f64 / 1000.0),
                    format!("{:.3}", s.p95 as f64 / 1000.0),
                ),
                None => ("n/a".to_string(), "n/a".to_string(), "n/a".to_string()),
            };
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                suite.name, test.name, test.iterations, min, avg, p95
            ));
        }
    }
    std::fs::write(path, out)
}

#[derive(Debug)]
struct CmdResult {
    success: bool,
    stdout: String,
    stderr: String,
    duration_micros: u64,
}

fn run_cmd(path: &str, args: &[&str]) -> std::io::Result<CmdResult> {
    let start = std::time::Instant::now();
    let output = std::process::Command::new(path).args(args).output()?;
    let duration_micros = start.elapsed().as_micros() as u64;
    Ok(CmdResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        duration_micros,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct BenchConfig {
    iters: usize,
    cold: bool,
    warm: bool,
    dry_run: bool,
    suites: Vec<String>,
    out_dir: String,
}

impl BenchConfig {
    fn from_args(args: Vec<String>) -> Result<Self, String> {
        let mut iters = 20usize;
        let mut cold = true;
        let mut warm = true;
        let mut dry_run = false;
        let mut suites = Vec::new();
        let mut out_dir = "bench/results".to_string();

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--iters" => {
                    i += 1;
                    let val = args
                        .get(i)
                        .ok_or_else(|| "--iters requires a value".to_string())?;
                    iters = val.parse().map_err(|_| "invalid --iters".to_string())?;
                }
                "--cold" => cold = true,
                "--no-cold" => cold = false,
                "--warm" => warm = true,
                "--no-warm" => warm = false,
                "--dry-run" => dry_run = true,
                "--suite" => {
                    i += 1;
                    let val = args
                        .get(i)
                        .ok_or_else(|| "--suite requires a value".to_string())?;
                    suites.push(val.to_string());
                }
                "--out-dir" => {
                    i += 1;
                    let val = args
                        .get(i)
                        .ok_or_else(|| "--out-dir requires a value".to_string())?;
                    out_dir = val.to_string();
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
            i += 1;
        }

        Ok(Self {
            iters,
            cold,
            warm,
            dry_run,
            suites,
            out_dir,
        })
    }

    fn default() -> Self {
        Self {
            iters: 20,
            cold: true,
            warm: true,
            dry_run: false,
            suites: Vec::new(),
            out_dir: "bench/results".to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct Stats {
    min: u64,
    avg: u64,
    p95: u64,
}

impl Stats {
    fn from_samples(samples: &[u64]) -> Self {
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let min = *sorted.first().unwrap_or(&0);
        let sum: u64 = sorted.iter().sum();
        let avg = if sorted.is_empty() {
            0
        } else {
            sum / sorted.len() as u64
        };
        let p95_idx = if sorted.is_empty() {
            0
        } else {
            ((sorted.len() - 1) as f64 * 0.95).ceil() as usize
        };
        let p95 = *sorted.get(p95_idx).unwrap_or(&0);
        Self { min, avg, p95 }
    }
}

fn stats_for(samples: &[u64]) -> Option<Stats> {
    if samples.is_empty() {
        None
    } else {
        Some(Stats::from_samples(samples))
    }
}

fn suite_enabled(cfg: &BenchConfig, name: &str) -> bool {
    if !cfg.suites.is_empty() {
        return cfg.suites.iter().any(|suite| suite == name);
    }

    match name {
        "pull" | "e2e" => cfg.cold,
        "run" | "exec" => cfg.warm,
        _ => true,
    }
}

fn planned_suites(cfg: &BenchConfig) -> Vec<String> {
    let mut suites = Vec::new();
    for name in ["pull", "run", "exec", "e2e"] {
        if suite_enabled(cfg, name) {
            suites.push(name.to_string());
        }
    }
    suites
}

fn read_cmd_trim(path: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(path).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Some(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        None
    } else {
        Some(stderr)
    }
}

fn build_metadata(minibox_bin: &str) -> Metadata {
    Metadata {
        timestamp: chrono::Utc::now().to_rfc3339(),
        hostname: read_cmd_trim("hostname", &[]).unwrap_or_else(|| "unknown".to_string()),
        git_sha: read_cmd_trim("git", &["rev-parse", "HEAD"])
            .unwrap_or_else(|| "unknown".to_string()),
        minibox_version: read_cmd_trim(minibox_bin, &["--version"])
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

fn run_cmd_record(path: &str, args: &[&str], errors: &mut Vec<String>) -> Option<CmdResult> {
    match run_cmd(path, args) {
        Ok(result) => {
            if !result.success {
                let stderr = result.stderr.trim();
                let stdout = result.stdout.trim();
                let mut message = format!("command failed: {} {:?}", path, args);
                if !stderr.is_empty() {
                    message.push_str(&format!("\nstderr: {stderr}"));
                }
                if !stdout.is_empty() {
                    message.push_str(&format!("\nstdout: {stdout}"));
                }
                errors.push(message);
                return None;
            }
            Some(result)
        }
        Err(err) => {
            errors.push(format!("command error: {} {:?}: {err}", path, args));
            None
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cfg = match BenchConfig::from_args(args) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(2);
        }
    };

    let report = match run_benchmark(&cfg) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    };

    let timestamp = chrono::Utc::now().to_rfc3339();
    let json_path = format!("{}/{}.json", cfg.out_dir, timestamp);
    let table_path = format!("{}/{}.txt", cfg.out_dir, timestamp);
    if let Err(e) = std::fs::create_dir_all(&cfg.out_dir) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    if let Err(e) = write_json(&report, &json_path) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    if let Err(e) = write_table(&report, &table_path) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run_benchmark(cfg: &BenchConfig) -> Result<BenchReport, String> {
    run_suites(cfg, cfg.dry_run)
}

fn run_suites(cfg: &BenchConfig, dry_run: bool) -> Result<BenchReport, String> {
    let minibox_bin = std::env::var("MINIBOX_BIN").unwrap_or_else(|_| "minibox".to_string());
    let metadata = build_metadata(&minibox_bin);
    let selected_suites = planned_suites(cfg);
    let mut suites = Vec::new();
    let mut errors = Vec::new();

    if selected_suites.is_empty() {
        errors.push("no suites selected (use --suite or enable --cold/--warm)".to_string());
    }

    if dry_run {
        for suite in selected_suites {
            suites.push(SuiteResult {
                name: suite,
                tests: Vec::new(),
            });
        }
        return Ok(BenchReport {
            metadata,
            suites,
            errors,
        });
    }

    if suite_enabled(cfg, "pull") {
        let mut pull_suite = SuiteResult {
            name: "pull".to_string(),
            tests: Vec::new(),
        };
        if let Some(pull) = run_cmd_record(&minibox_bin, &["pull", "alpine"], &mut errors) {
            let durations = vec![pull.duration_micros];
            pull_suite.tests.push(TestResult {
                name: "pull_alpine".to_string(),
                iterations: durations.len(),
                durations_micros: durations.clone(),
                stats: stats_for(&durations),
            });
        }
        suites.push(pull_suite);
    }

    if suite_enabled(cfg, "run") {
        let mut run_suite = SuiteResult {
            name: "run".to_string(),
            tests: Vec::new(),
        };
        let mut durations = Vec::with_capacity(cfg.iters);
        for _ in 0..cfg.iters {
            if let Some(run) = run_cmd_record(
                &minibox_bin,
                &["run", "alpine", "--", "/bin/true"],
                &mut errors,
            ) {
                durations.push(run.duration_micros);
            }
        }
        run_suite.tests.push(TestResult {
            name: "run_true".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
        });
        suites.push(run_suite);
    }

    if suite_enabled(cfg, "exec") {
        let mut exec_suite = SuiteResult {
            name: "exec".to_string(),
            tests: Vec::new(),
        };
        let mut durations = Vec::with_capacity(cfg.iters);
        for _ in 0..cfg.iters {
            if let Some(exec) = run_cmd_record(
                &minibox_bin,
                &["run", "alpine", "--", "/bin/echo", "ok"],
                &mut errors,
            ) {
                durations.push(exec.duration_micros);
            }
        }
        exec_suite.tests.push(TestResult {
            name: "exec_echo".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
        });
        suites.push(exec_suite);
    }

    if suite_enabled(cfg, "e2e") {
        let mut e2e_suite = SuiteResult {
            name: "e2e".to_string(),
            tests: Vec::new(),
        };
        if let Some(pull) = run_cmd_record(&minibox_bin, &["pull", "alpine"], &mut errors) {
            let durations = vec![pull.duration_micros];
            e2e_suite.tests.push(TestResult {
                name: "pull_alpine".to_string(),
                iterations: durations.len(),
                durations_micros: durations.clone(),
                stats: stats_for(&durations),
            });
        }
        if let Some(run) = run_cmd_record(
            &minibox_bin,
            &["run", "alpine", "--", "/bin/true"],
            &mut errors,
        ) {
            let durations = vec![run.duration_micros];
            e2e_suite.tests.push(TestResult {
                name: "run_true".to_string(),
                iterations: durations.len(),
                durations_micros: durations.clone(),
                stats: stats_for(&durations),
            });
        }
        suites.push(e2e_suite);
    }

    Ok(BenchReport {
        metadata,
        suites,
        errors,
    })
}

fn print_help() {
    println!(
        "minibox-bench\n\nFlags:\n  --iters <N>\n  --cold/--no-cold\n  --warm/--no-warm\n  --dry-run\n  --suite <name>\n  --out-dir <path>"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_serializes() {
        let report = BenchReport::empty();
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"metadata\""));
    }

    #[test]
    fn stats_min_avg_p95() {
        let data = vec![10u64, 20, 30, 40, 50];
        let stats = Stats::from_samples(&data);
        assert_eq!(stats.min, 10);
        assert_eq!(stats.avg, 30);
        assert_eq!(stats.p95, 50);
    }

    #[test]
    fn default_iters_is_20() {
        let args = vec!["bench".to_string()];
        let cfg = BenchConfig::from_args(args).unwrap();
        assert_eq!(cfg.iters, 20);
    }

    #[test]
    fn command_runner_captures_exit_status() {
        let result = run_cmd("/bin/true", &[]).unwrap();
        assert!(result.success);
    }

    #[test]
    fn suite_has_results() {
        let cfg = BenchConfig::default();
        let report = run_suites(&cfg, true).unwrap();
        assert!(!report.suites.is_empty());
    }

    #[test]
    fn report_writes_json() {
        let report = BenchReport::empty();
        let path = "/tmp/bench-report.json";
        write_json(&report, path).unwrap();
        assert!(std::path::Path::new(path).exists());
    }

    #[test]
    fn dry_run_skips_execution() {
        let cfg = BenchConfig {
            dry_run: true,
            ..BenchConfig::default()
        };
        let report = run_benchmark(&cfg).unwrap();
        assert!(report.suites.is_empty() == false);
    }
}
