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

#[derive(Debug, PartialEq)]
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
        let avg = if sorted.is_empty() { 0 } else { sum / sorted.len() as u64 };
        let p95_idx = if sorted.is_empty() {
            0
        } else {
            ((sorted.len() - 1) as f64 * 0.95).ceil() as usize
        };
        let p95 = *sorted.get(p95_idx).unwrap_or(&0);
        Self { min, avg, p95 }
    }
}

fn main() {
    println!("minibox-bench: not yet implemented");
}

fn print_help() {
    println!("minibox-bench\n\nFlags:\n  --iters <N>\n  --cold/--no-cold\n  --warm/--no-warm\n  --dry-run\n  --suite <name>\n  --out-dir <path>");
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
}
