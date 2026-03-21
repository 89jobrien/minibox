use minibox_lib::adapters::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};
use minibox_lib::domain::{
    ContainerHooks, ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageRegistry,
    ResourceConfig, ResourceLimiter,
};
use minibox_lib::protocol::{
    ContainerInfo, DaemonRequest, DaemonResponse, decode_request, decode_response, encode_request,
    encode_response,
};
use serde::Serialize;
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Serialize, Default)]
struct BenchReport {
    metadata: Metadata,
    suites: Vec<SuiteResult>,
    errors: Vec<String>,
}

impl BenchReport {
    #[allow(dead_code)]
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
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    durations_micros: Vec<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    durations_nanos: Vec<u64>,
    stats: Option<Stats>,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    unit: String,
}

fn write_json(report: &BenchReport, path: &str) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(report).unwrap();
    std::fs::write(path, json)
}

fn write_table(report: &BenchReport, path: &str) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("minibox benchmark results\n\n");
    out.push_str(&format!("suites: {}\n\n", report.suites.len()));
    out.push_str("suite\ttest\titers\tunit\tmin\tavg\tp95\n");
    for suite in &report.suites {
        for test in &suite.tests {
            let is_nanos = test.unit == "nanos";
            let (unit_label, min, avg, p95) = match test.stats.as_ref() {
                Some(s) if is_nanos => (
                    "ns",
                    format!("{}", s.min),
                    format!("{}", s.avg),
                    format!("{}", s.p95),
                ),
                Some(s) => (
                    "ms",
                    format!("{:.3}", s.min as f64 / 1000.0),
                    format!("{:.3}", s.avg as f64 / 1000.0),
                    format!("{:.3}", s.p95 as f64 / 1000.0),
                ),
                None => (
                    "n/a",
                    "n/a".to_string(),
                    "n/a".to_string(),
                    "n/a".to_string(),
                ),
            };
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                suite.name, test.name, test.iterations, unit_label, min, avg, p95
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

    #[allow(dead_code)]
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
        // codec and adapter only run when explicitly requested via --suite
        "codec" | "adapter" => false,
        _ => true,
    }
}

fn planned_suites(cfg: &BenchConfig) -> Vec<String> {
    let mut suites = Vec::new();
    for name in ["pull", "run", "exec", "e2e", "codec", "adapter"] {
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
                let mut message = format!("command failed: {path} {args:?}");
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
            errors.push(format!("command error: {path} {args:?}: {err}"));
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
    // Print JSON path to stdout so callers (e.g. xtask) can capture it without scanning the dir.
    println!("{json_path}");
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
                ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
            });
        }
        suites.push(e2e_suite);
    }

    if suite_enabled(cfg, "codec") {
        suites.push(bench_codec_suite(cfg));
    }

    if suite_enabled(cfg, "adapter") {
        suites.push(bench_adapter_suite(cfg));
    }

    Ok(BenchReport {
        metadata,
        suites,
        errors,
    })
}

// ── Microbenchmark suites (no daemon required) ───────────────────────────────

fn measure_nanos<F: FnMut()>(iters: usize, mut f: F) -> Vec<u64> {
    (0..iters)
        .map(|_| {
            let start = std::time::Instant::now();
            f();
            start.elapsed().as_nanos() as u64
        })
        .collect()
}

fn nano_test(name: &str, iters: usize, f: impl FnMut()) -> TestResult {
    let durations = measure_nanos(iters, f);
    let stats = stats_for(&durations);
    TestResult {
        name: name.to_string(),
        iterations: durations.len(),
        durations_nanos: durations,
        stats,
        unit: "nanos".to_string(),
        ..Default::default()
    }
}

fn bench_codec_suite(cfg: &BenchConfig) -> SuiteResult {
    let iters = cfg.iters.max(100);

    // ── requests ─────────────────────────────────────────────────────────────
    let small_run = DaemonRequest::Run {
        image: "alpine".to_string(),
        tag: None,
        command: vec!["/bin/sh".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: false,
    };
    let large_run = DaemonRequest::Run {
        image: "library/some-really-long-image-name-for-benchmarking".to_string(),
        tag: Some("2026.03.16-benchmarks".to_string()),
        command: (0..24)
            .map(|i| format!("arg-{}-{}", i, "x".repeat(16)))
            .collect(),
        memory_limit_bytes: Some(512 * 1024 * 1024),
        cpu_weight: Some(7500),
        ephemeral: false,
    };
    let small_pull = DaemonRequest::Pull {
        image: "alpine".to_string(),
        tag: None,
    };
    let large_pull = DaemonRequest::Pull {
        image: "library/some-really-long-image-name-for-benchmarking".to_string(),
        tag: Some("2026.03.16-benchmarks".to_string()),
    };
    let small_stop = DaemonRequest::Stop {
        id: "deadbeefdeadbeef".to_string(),
    };
    let large_stop = DaemonRequest::Stop {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    };
    let small_remove = DaemonRequest::Remove {
        id: "deadbeefdeadbeef".to_string(),
    };
    let large_remove = DaemonRequest::Remove {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    };
    let list_req = DaemonRequest::List;

    // ── responses ────────────────────────────────────────────────────────────
    fn make_container_info(i: usize) -> ContainerInfo {
        ContainerInfo {
            id: format!("{:016x}", i),
            image: format!("library/image-{}", i),
            command: format!("echo hello {}", i),
            state: if i % 2 == 0 { "running" } else { "stopped" }.to_string(),
            created_at: format!("2026-03-16T12:{:02}:00Z", i % 60),
            pid: Some(1000 + i as u32),
        }
    }
    let small_created = DaemonResponse::ContainerCreated {
        id: "deadbeefdeadbeef".to_string(),
    };
    let large_created = DaemonResponse::ContainerCreated {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    };
    let small_success = DaemonResponse::Success {
        message: "ok".to_string(),
    };
    let large_success = DaemonResponse::Success {
        message: "operation completed successfully with additional context".to_string(),
    };
    let small_error = DaemonResponse::Error {
        message: "error".to_string(),
    };
    let large_error = DaemonResponse::Error {
        message: "error: failed to perform operation due to invalid state".to_string(),
    };
    let small_list = DaemonResponse::ContainerList {
        containers: vec![make_container_info(0)],
    };
    let large_list = DaemonResponse::ContainerList {
        containers: (0..100).map(make_container_info).collect(),
    };

    // Pre-encode for decode benchmarks.
    let enc_small_run = encode_request(&small_run).unwrap();
    let enc_large_run = encode_request(&large_run).unwrap();
    let enc_small_pull = encode_request(&small_pull).unwrap();
    let enc_large_pull = encode_request(&large_pull).unwrap();
    let enc_small_stop = encode_request(&small_stop).unwrap();
    let enc_large_stop = encode_request(&large_stop).unwrap();
    let enc_small_remove = encode_request(&small_remove).unwrap();
    let enc_large_remove = encode_request(&large_remove).unwrap();
    let enc_list = encode_request(&list_req).unwrap();
    let enc_small_created = encode_response(&small_created).unwrap();
    let enc_large_created = encode_response(&large_created).unwrap();
    let enc_small_success = encode_response(&small_success).unwrap();
    let enc_large_success = encode_response(&large_success).unwrap();
    let enc_small_error = encode_response(&small_error).unwrap();
    let enc_large_error = encode_response(&large_error).unwrap();
    let enc_small_list = encode_response(&small_list).unwrap();
    let enc_large_list = encode_response(&large_list).unwrap();

    let mut tests = vec![
        nano_test("encode_run_small", iters, || {
            black_box(encode_request(black_box(&small_run)).unwrap());
        }),
        nano_test("decode_run_small", iters, || {
            black_box(decode_request(black_box(&enc_small_run)).unwrap());
        }),
        nano_test("encode_run_large", iters, || {
            black_box(encode_request(black_box(&large_run)).unwrap());
        }),
        nano_test("decode_run_large", iters, || {
            black_box(decode_request(black_box(&enc_large_run)).unwrap());
        }),
        nano_test("encode_pull_small", iters, || {
            black_box(encode_request(black_box(&small_pull)).unwrap());
        }),
        nano_test("decode_pull_small", iters, || {
            black_box(decode_request(black_box(&enc_small_pull)).unwrap());
        }),
        nano_test("encode_pull_large", iters, || {
            black_box(encode_request(black_box(&large_pull)).unwrap());
        }),
        nano_test("decode_pull_large", iters, || {
            black_box(decode_request(black_box(&enc_large_pull)).unwrap());
        }),
        nano_test("encode_stop_small", iters, || {
            black_box(encode_request(black_box(&small_stop)).unwrap());
        }),
        nano_test("decode_stop_small", iters, || {
            black_box(decode_request(black_box(&enc_small_stop)).unwrap());
        }),
        nano_test("encode_stop_large", iters, || {
            black_box(encode_request(black_box(&large_stop)).unwrap());
        }),
        nano_test("decode_stop_large", iters, || {
            black_box(decode_request(black_box(&enc_large_stop)).unwrap());
        }),
        nano_test("encode_remove_small", iters, || {
            black_box(encode_request(black_box(&small_remove)).unwrap());
        }),
        nano_test("decode_remove_small", iters, || {
            black_box(decode_request(black_box(&enc_small_remove)).unwrap());
        }),
        nano_test("encode_remove_large", iters, || {
            black_box(encode_request(black_box(&large_remove)).unwrap());
        }),
        nano_test("decode_remove_large", iters, || {
            black_box(decode_request(black_box(&enc_large_remove)).unwrap());
        }),
        nano_test("encode_list", iters, || {
            black_box(encode_request(black_box(&list_req)).unwrap());
        }),
        nano_test("decode_list", iters, || {
            black_box(decode_request(black_box(&enc_list)).unwrap());
        }),
        nano_test("encode_container_created_small", iters, || {
            black_box(encode_response(black_box(&small_created)).unwrap());
        }),
        nano_test("decode_container_created_small", iters, || {
            black_box(decode_response(black_box(&enc_small_created)).unwrap());
        }),
        nano_test("encode_container_created_large", iters, || {
            black_box(encode_response(black_box(&large_created)).unwrap());
        }),
        nano_test("decode_container_created_large", iters, || {
            black_box(decode_response(black_box(&enc_large_created)).unwrap());
        }),
        nano_test("encode_success_small", iters, || {
            black_box(encode_response(black_box(&small_success)).unwrap());
        }),
        nano_test("decode_success_small", iters, || {
            black_box(decode_response(black_box(&enc_small_success)).unwrap());
        }),
        nano_test("encode_success_large", iters, || {
            black_box(encode_response(black_box(&large_success)).unwrap());
        }),
        nano_test("decode_success_large", iters, || {
            black_box(decode_response(black_box(&enc_large_success)).unwrap());
        }),
        nano_test("encode_error_small", iters, || {
            black_box(encode_response(black_box(&small_error)).unwrap());
        }),
        nano_test("decode_error_small", iters, || {
            black_box(decode_response(black_box(&enc_small_error)).unwrap());
        }),
        nano_test("encode_error_large", iters, || {
            black_box(encode_response(black_box(&large_error)).unwrap());
        }),
        nano_test("decode_error_large", iters, || {
            black_box(decode_response(black_box(&enc_large_error)).unwrap());
        }),
        nano_test("encode_container_list_small", iters, || {
            black_box(encode_response(black_box(&small_list)).unwrap());
        }),
        nano_test("decode_container_list_small", iters, || {
            black_box(decode_response(black_box(&enc_small_list)).unwrap());
        }),
        nano_test("encode_container_list_large", iters, || {
            black_box(encode_response(black_box(&large_list)).unwrap());
        }),
        nano_test("decode_container_list_large", iters, || {
            black_box(decode_response(black_box(&enc_large_list)).unwrap());
        }),
    ];

    let invalid_req: &[u8] = b"{not-json\n";
    let invalid_resp: &[u8] = br#"{"type":"Unknown"}\n"#;
    tests.push(nano_test("decode_invalid_request", iters, || {
        black_box(decode_request(black_box(invalid_req)).is_err());
    }));
    tests.push(nano_test("decode_invalid_response", iters, || {
        black_box(decode_response(black_box(invalid_resp)).is_err());
    }));

    SuiteResult {
        name: "codec".to_string(),
        tests,
    }
}

fn bench_adapter_suite(cfg: &BenchConfig) -> SuiteResult {
    let iters = cfg.iters.max(100);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let layers = vec![PathBuf::from("/layer1")];
    let container_dir = PathBuf::from("/container");
    let resource_cfg = ResourceConfig::default();
    let spawn_cfg = ContainerSpawnConfig {
        rootfs: PathBuf::from("/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: PathBuf::from("/cgroup"),
        capture_output: false,
        hooks: ContainerHooks::default(),
    };

    let registry_concrete = MockRegistry::new().with_cached_image("alpine", "latest");
    let registry_trait: Arc<dyn ImageRegistry> =
        Arc::new(MockRegistry::new().with_cached_image("alpine", "latest"));
    let fs_concrete = MockFilesystem::new();
    let fs_trait: Arc<dyn FilesystemProvider> = Arc::new(MockFilesystem::new());
    let limiter_concrete = MockLimiter::new();
    let limiter_trait: Arc<dyn ResourceLimiter> = Arc::new(MockLimiter::new());
    let runtime_concrete = MockRuntime::new();
    let runtime_trait: Arc<dyn ContainerRuntime> = Arc::new(MockRuntime::new());
    let arc_for_clone: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());
    let arc_for_downcast: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());

    let tests = vec![
        nano_test("registry_direct_has_image", iters, || {
            black_box(registry_concrete.has_image_sync("alpine", "latest"));
        }),
        nano_test("registry_trait_object_has_image", iters, || {
            rt.block_on(async {
                black_box(registry_trait.has_image("alpine", "latest")).await;
            });
        }),
        nano_test("filesystem_direct_setup", iters, || {
            black_box(fs_concrete.setup_rootfs(&layers, &container_dir)).ok();
        }),
        nano_test("filesystem_trait_object_setup", iters, || {
            black_box(fs_trait.setup_rootfs(&layers, &container_dir)).ok();
        }),
        nano_test("limiter_direct_create", iters, || {
            black_box(limiter_concrete.create("container-123", &resource_cfg)).ok();
        }),
        nano_test("limiter_trait_object_create", iters, || {
            black_box(limiter_trait.create("container-123", &resource_cfg)).ok();
        }),
        nano_test("runtime_direct_spawn", iters, || {
            black_box(runtime_concrete.spawn_process_sync(&spawn_cfg)).ok();
        }),
        nano_test("runtime_trait_object_spawn", iters, || {
            rt.block_on(async {
                black_box(runtime_trait.spawn_process(&spawn_cfg).await).ok();
            });
        }),
        nano_test("arc_clone", iters, || {
            black_box(Arc::clone(&arc_for_clone));
        }),
        nano_test("downcast_to_concrete", iters, || {
            black_box(arc_for_downcast.as_any().downcast_ref::<MockRegistry>());
        }),
    ];

    SuiteResult {
        name: "adapter".to_string(),
        tests,
    }
}

fn print_help() {
    println!(
        "minibox-bench\n\nFlags:\n  --iters <N>\n  --cold/--no-cold\n  --warm/--no-warm\n  --dry-run\n  --suite <name>  (pull|run|exec|e2e|codec|adapter)\n  --out-dir <path>"
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
        assert!(!report.suites.is_empty());
    }

    #[test]
    fn nano_test_uses_durations_nanos_not_micros() {
        let result = nano_test("test", 5, || {
            std::hint::black_box(1 + 1);
        });
        assert!(
            !result.durations_nanos.is_empty(),
            "durations_nanos must be populated"
        );
        assert!(
            result.durations_micros.is_empty(),
            "durations_micros must be empty for nano tests"
        );
        assert_eq!(result.unit, "nanos");
    }
}
