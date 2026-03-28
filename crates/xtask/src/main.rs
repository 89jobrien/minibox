use anyhow::{Context, Result, bail};
use std::{
    env, fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use xshell::{Shell, cmd};

fn main() -> Result<()> {
    let task = env::args().nth(1);
    let sh = Shell::new()?;

    // Run from workspace root
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    sh.change_dir(root);

    match task.as_deref() {
        Some("pre-commit") => pre_commit(&sh),
        Some("prepush") => prepush(&sh),
        Some("test-unit") => test_unit(&sh),
        Some("test-property") => test_property(&sh),
        Some("test-integration") => test_integration(&sh),
        Some("test-e2e-suite") => test_e2e_suite(&sh),
        Some("test-sandbox") => test_sandbox(&sh),
        Some("clean-artifacts") => clean_artifacts(&sh),
        Some("nuke-test-state") => nuke_test_state(&sh),
        Some("bench") => {
            let extra: Vec<String> = env::args().skip(2).collect();
            bench(&sh, &extra)
        }
        Some("bench-vps") => {
            let extra: Vec<String> = env::args().skip(2).collect();
            bench_vps(&sh, &extra)
        }
        Some("bench-diff") => {
            let extra: Vec<String> = env::args().skip(2).collect();
            bench_diff(&extra)
        }
        Some("bench-report") => bench_report(),
        Some("flamegraph") => {
            let extra: Vec<String> = env::args().skip(2).collect();
            flamegraph(&sh, &extra)
        }
        Some(other) => bail!("unknown task: {other}"),
        None => {
            eprintln!("Available tasks:");
            eprintln!("  pre-commit       fmt-check + lint + build-release");
            eprintln!("  prepush          nextest + coverage");
            eprintln!("  test-unit        all unit + conformance tests");
            eprintln!("  test-property    property-based tests (proptest)");
            eprintln!("  test-integration cgroup + integration tests (Linux, root)");
            eprintln!("  test-e2e-suite   daemon+CLI e2e tests (Linux, root)");
            eprintln!("  test-sandbox     sandbox contract tests (Linux, root, Docker Hub)");
            eprintln!("  clean-artifacts  remove non-critical build outputs");
            eprintln!("  nuke-test-state  kill orphans, unmount overlays, clean cgroups");
            eprintln!("  bench            run benchmark binary (local, dry-run safe)");
            eprintln!(
                "  bench-vps        run benchmark on VPS, append to bench/results/bench.jsonl"
            );
            eprintln!("  bench-diff       diff two bench JSON files (default: HEAD vs previous)");
            eprintln!("  bench-report     generate HTML report from bench/results/bench.jsonl");
            eprintln!(
                "  flamegraph       profile bench binary with samply (macOS) or cargo-flamegraph (Linux)"
            );
            Ok(())
        }
    }
}

/// Pre-commit gate: fmt check → lint → release build (macOS-safe)
fn pre_commit(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo fmt --all --check")
        .run()
        .context("fmt-check failed")?;
    cmd!(
        sh,
        "cargo clippy -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -- -D warnings"
    )
    .run()
    .context("lint failed")?;
    cmd!(sh,
        "cargo build --release -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -p minibox-bench"
    ).run().context("build-release failed")?;
    eprintln!("pre-commit checks passed");
    Ok(())
}

/// Pre-push gate: nextest + coverage + ai-review
fn prepush(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo nextest run --release -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox"
    )
    .run()
    .context("nextest failed")?;
    cmd!(
        sh,
        "cargo llvm-cov nextest -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox --html"
    )
    .run()
    .context("coverage failed")?;
    eprintln!("coverage: target/llvm-cov/html/index.html");
    eprintln!("running ai-review...");
    if let Err(e) = cmd!(sh, "uv run scripts/ai-review.py --base main").run() {
        eprintln!("warning: ai-review failed (non-fatal): {e}");
    }
    Ok(())
}

/// All unit + conformance tests (any platform)
fn test_unit(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo test --release -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox --lib"
    )
    .run()
    .context("lib tests failed")?;
    cmd!(sh, "cargo test --release -p daemonbox --test handler_tests")
        .run()
        .context("handler_tests failed")?;
    cmd!(
        sh,
        "cargo test --release -p daemonbox --test conformance_tests"
    )
    .run()
    .context("conformance_tests failed")?;
    Ok(())
}

/// Property-based tests (proptest)
fn test_property(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo test --release -p linuxbox --test proptest_suite")
        .run()
        .context("linuxbox property tests failed")?;
    cmd!(
        sh,
        "cargo test --release -p daemonbox --test proptest_suite"
    )
    .run()
    .context("daemonbox property tests failed")?;
    Ok(())
}

/// Cgroup + integration tests (Linux, root required)
fn test_integration(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo test --release -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture"
    )
    .run()
    .context("cgroup tests failed")?;
    cmd!(
        sh,
        "cargo test --release -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture"
    )
    .run()
    .context("integration tests failed")?;
    Ok(())
}

/// Daemon+CLI e2e tests (Linux, root required)
fn test_e2e_suite(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo build --release")
        .run()
        .context("build failed")?;

    // Build test binary without running it, then find it in target/release/deps
    cmd!(
        sh,
        "cargo test -p miniboxd --test e2e_tests --release --no-run"
    )
    .run()
    .context("failed to build e2e test binary")?;

    let binary = find_test_binary("target/release/deps", "e2e_tests")
        .context("could not locate e2e test binary in target/release/deps")?;

    let bin_dir = env::current_dir()?.join("target/release");
    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {binary} --test-threads=1 --nocapture"
    )
    .run()
    .context("e2e tests failed")?;
    Ok(())
}

/// Sandbox contract tests (Linux, root, Docker Hub required)
fn test_sandbox(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo build --release")
        .run()
        .context("build failed")?;

    cmd!(
        sh,
        "cargo test -p miniboxd --test sandbox_tests --release --no-run"
    )
    .run()
    .context("failed to build sandbox test binary")?;

    let binary = find_test_binary("target/release/deps", "sandbox_tests")
        .context("could not locate sandbox test binary in target/release/deps")?;

    let bin_dir = env::current_dir()?.join("target/release");
    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {binary} --test-threads=1 --ignored --nocapture"
    )
    .run()
    .context("sandbox tests failed")?;
    Ok(())
}

/// Find the most recently modified test binary matching a name prefix (no `.d` extension)
fn find_test_binary(deps_dir: &str, prefix: &str) -> Option<std::path::PathBuf> {
    let dir = Path::new(deps_dir);
    let mut candidates: Vec<_> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            // Match `prefix-<hash>` pattern, no extension (i.e. not .d files)
            let is_file = e.file_type().is_ok_and(|t| t.is_file());
            name.starts_with(prefix) && !name.ends_with(".d") && is_file
        })
        .collect();
    // Pick the most recently modified candidate
    candidates.sort_by_key(|e| e.metadata().ok()?.modified().ok());
    candidates.last().map(|e| e.path())
}

/// Remove non-critical build outputs (preserves incremental cache and registry)
fn clean_artifacts(sh: &Shell) -> Result<()> {
    for dir in &["target/debug", "target/release"] {
        let p = Path::new(dir);
        if p.exists() {
            for entry in fs::read_dir(p).into_iter().flatten().flatten() {
                if entry.file_type().ok().is_some_and(|t| t.is_file()) {
                    fs::remove_file(entry.path()).ok();
                }
            }
        }
    }

    for dir in &["target/debug/deps", "target/release/deps"] {
        let p = Path::new(dir);
        if p.exists() {
            for entry in fs::read_dir(p).into_iter().flatten().flatten() {
                let path = entry.path();
                let keep = path.extension().is_some_and(|e| e == "d");
                if !keep && entry.file_type().ok().is_some_and(|t| t.is_file()) {
                    fs::remove_file(&path).ok();
                }
            }
        }
    }

    // Remove .dSYM bundles (macOS debug info directories)
    let _ = sh
        .cmd("find")
        .args([
            "target", "-type", "d", "-name", "*.dSYM", "-exec", "rm", "-rf", "{}", "+",
        ])
        .ignore_status()
        .run();

    eprintln!("artifacts cleaned");
    Ok(())
}

/// Kill orphan processes, unmount overlays, remove test cgroups, clean temp dirs
fn nuke_test_state(sh: &Shell) -> Result<()> {
    cmd!(sh, "pkill -f miniboxd.*minibox-test")
        .ignore_status()
        .run()?;
    cmd!(
        sh,
        "bash -c \"mount | grep minibox-test | awk '{print $3}' | xargs -r umount\""
    )
    .ignore_status()
    .run()?;
    cmd!(sh, "bash -c \"systemctl list-units --type=scope --no-legend 2>/dev/null | grep minibox-test | awk '{print $1}' | xargs -r systemctl stop\"")
        .ignore_status()
        .run()?;
    let _ = sh
        .cmd("find")
        .args([
            "/sys/fs/cgroup",
            "-name",
            "minibox-test-*",
            "-type",
            "d",
            "-exec",
            "rmdir",
            "{}",
            "+",
        ])
        .ignore_status()
        .run();
    cmd!(sh, "rm -rf /tmp/minibox-test-*")
        .ignore_status()
        .run()?;
    eprintln!("test state cleaned");
    Ok(())
}

/// Run benchmark binary (local, requires Linux + miniboxd running) and save results.
fn bench(sh: &Shell, extra_args: &[String]) -> Result<()> {
    let out_dir = "bench/results";
    fs::create_dir_all(out_dir).context("create bench/results")?;

    // Build the bench binary in release mode
    cmd!(sh, "cargo build --release -p minibox-bench")
        .run()
        .context("build minibox-bench")?;

    // Build argument list
    let mut args: Vec<&str> = vec!["--out-dir", out_dir];
    if extra_args.iter().any(|a| a == "--profile") {
        args.push("--profile");
    }

    // Capture stdout — the binary prints the JSON path as its last line.
    let output = cmd!(sh, "./target/release/minibox-bench {args...}")
        .read()
        .context("bench binary failed")?;

    let json_path = output
        .lines()
        .last()
        .filter(|l| l.ends_with(".json"))
        .ok_or_else(|| anyhow::anyhow!("bench binary did not print a .json path on stdout"))?;

    save_bench_results(sh, json_path)
}

/// Patch git_sha, append to bench.jsonl, and update latest.json.
fn save_bench_results(sh: &Shell, json_path: &str) -> Result<()> {
    let content = fs::read_to_string(json_path).context("read bench JSON")?;
    let mut json: serde_json::Value =
        serde_json::from_str(&content).context("invalid bench JSON")?;
    if let Ok(sha) = cmd!(sh, "git rev-parse HEAD").read() {
        let sha = sha.trim();
        if !sha.is_empty() {
            json["metadata"]["git_sha"] = serde_json::Value::String(sha.to_string());
        }
    }
    let line = serde_json::to_string(&json).context("re-serialise failed")?;
    let jsonl_path = "bench/results/bench.jsonl";
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(jsonl_path)
        .context("open bench.jsonl")?;
    use std::io::Write as _;
    writeln!(file, "{line}").context("append to bench.jsonl")?;
    eprintln!("✓ bench/results/bench.jsonl appended");

    let pretty = serde_json::to_string_pretty(&json).context("pretty-print failed")?;
    fs::write("bench/results/latest.json", pretty).context("write latest.json")?;
    eprintln!("✓ bench/results/latest.json updated");
    Ok(())
}

// ── Flamegraph / profiling ────────────────────────────────────────────────────

/// Profile the bench binary and open the result.
///
/// macOS  — uses `samply record` (no SIP changes needed); opens Firefox Profiler.
/// Linux  — uses `cargo flamegraph`; writes SVG to bench/profiles/.
///
/// Options (passed as extra args):
///   --suite <name>   bench suite to run (default: codec)
///   --open           open result automatically (default: true on macOS)
fn flamegraph(sh: &Shell, extra_args: &[String]) -> Result<()> {
    fs::create_dir_all("bench/profiles").context("create bench/profiles")?;

    let suite = extra_args
        .windows(2)
        .find(|w| w[0] == "--suite")
        .map(|w| w[1].as_str())
        .unwrap_or("codec");

    // Build release binary first
    cmd!(sh, "cargo build --release -p minibox-bench")
        .run()
        .context("build minibox-bench")?;

    let bin = "./target/release/minibox-bench";

    if cfg!(target_os = "macos") {
        // samply: records a profile and opens Firefox Profiler in the browser.
        // Install: cargo install samply
        which("samply").context("samply not found — install with: cargo install samply")?;

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let profile_path = format!("bench/profiles/samply-{suite}-{ts}.nsp");

        eprintln!("profiling with samply (suite={suite}) → {profile_path}");
        cmd!(
            sh,
            "samply record --save-only -o {profile_path} {bin} --suite {suite}"
        )
        .run()
        .context("samply record failed")?;

        eprintln!("saved: {profile_path}");
        eprintln!("opening in Firefox...");
        // BROWSER=firefox ensures Firefox Profiler opens, not the system default browser.
        let _env = sh.push_env("BROWSER", "firefox");
        cmd!(sh, "samply load {profile_path}")
            .run()
            .context("samply load failed")?;
    } else {
        // Linux: cargo flamegraph writes an SVG.
        which("cargo-flamegraph")
            .context("cargo-flamegraph not found — install with: cargo install flamegraph")?;
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let svg = format!("bench/profiles/flamegraph-{suite}-{ts}.svg");
        eprintln!("profiling with cargo-flamegraph (suite={suite}) → {svg}");
        cmd!(
            sh,
            "cargo flamegraph --bin minibox-bench -o {svg} -- --suite {suite}"
        )
        .run()
        .context("cargo flamegraph failed")?;
        eprintln!("saved: {svg}");

        // Open in browser if available
        let open_cmd = if which("xdg-open").is_ok() {
            "xdg-open"
        } else {
            "open"
        };
        let _ = Command::new(open_cmd).arg(&svg).status();
    }

    Ok(())
}

/// Return Ok if `name` is on PATH, Err otherwise.
fn which(name: &str) -> Result<()> {
    let status = Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("which failed")?;
    if status.success() {
        Ok(())
    } else {
        bail!("{name} not found on PATH")
    }
}

// ── VPS bench helpers ────────────────────────────────────────────────────────

/// Parse --commit / --push flags from extra args. Returns (commit, push).
fn parse_bench_vps_flags(args: &[String]) -> (bool, bool) {
    let commit = args.iter().any(|a| a == "--commit");
    let push = args.iter().any(|a| a == "--push");
    (commit, push)
}

const VPS_HOST: &str = "jobrien-vm";
const BENCH_BIN: &str = "/home/dev/minibox/target/release/minibox-bench";

/// Run a script on the remote as root via key-based SSH auth.
/// Two SSH calls:
/// 1. Upload script to a temp file (stdin pipe).
/// 2. Execute with `sudo -S bash <tmpfile>` (sudo password on stdin).
fn ssh_sudo_script(sudo_pass: &str, script: &str) -> Result<String> {
    let tmpfile = format!("/tmp/xtask-bench-{}.sh", std::process::id());

    // Step 1: upload script
    let write_cmd = format!("cat > '{tmpfile}' && chmod 700 '{tmpfile}'");
    let mut upload = Command::new("ssh")
        .arg(VPS_HOST)
        .arg(&write_cmd)
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn ssh for script upload")?;
    upload
        .stdin
        .take()
        .context("no stdin")?
        .write_all(script.as_bytes())
        .context("failed to write script")?;
    if !upload.wait().context("script upload wait")?.success() {
        bail!("failed to write script to remote");
    }

    // Step 2: run as root; pass sudo password via stdin so it never appears in ps output
    let run_cmd = format!("sudo -S bash '{tmpfile}'; RC=$?; rm -f '{tmpfile}'; exit $RC");
    let mut run = Command::new("ssh")
        .arg(VPS_HOST)
        .arg(&run_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("ssh sudo run failed")?;
    // Write password then close stdin so sudo reads exactly one line
    if let Some(mut stdin) = run.stdin.take() {
        writeln!(stdin, "{sudo_pass}").context("write sudo password to ssh stdin")?;
    }
    let out = run
        .wait_with_output()
        .context("ssh sudo wait_with_output failed")?;
    if !out.status.success() {
        bail!("remote script exited with status {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run minibox-bench on the VPS as root and append results to bench/results/bench.jsonl.
fn bench_vps(sh: &Shell, extra_args: &[String]) -> Result<()> {
    let vps_pass = cmd!(
        sh,
        "op item get jobrien-vm --account=my.1password.com --fields password --reveal"
    )
    .read()
    .context("op credential fetch failed")?;
    let vps_pass = vps_pass.trim().to_string();

    eprintln!("running bench on VPS (this takes ~1 min)...");
    let bench_script = format!(
        r#"set -euo pipefail
export PATH="/home/dev/.cargo/bin:/home/dev/.local/bin:$PATH"
BENCH_BIN="{BENCH_BIN}"
[ -f "$BENCH_BIN" ] || {{ echo "error: minibox-bench not found — run: mise run bench:setup" >&2; exit 1; }}
command -v minibox >/dev/null 2>&1 || {{ echo "error: minibox not in PATH" >&2; exit 1; }}
[ -S /run/minibox/miniboxd.sock ] || {{ echo "error: miniboxd not running" >&2; exit 1; }}
OUT_DIR="/tmp/bench-out-$$"
rm -rf "$OUT_DIR"
"$BENCH_BIN" --iters 5 --out-dir "$OUT_DIR"
JSON_FILE=$(ls -t "$OUT_DIR"/*.json | head -1)
cp "$JSON_FILE" /tmp/bench-latest.json
ls -t "$OUT_DIR"/*.txt | head -1 | xargs cat
rm -rf "$OUT_DIR"
"#
    );
    let bench_txt = ssh_sudo_script(&vps_pass, &bench_script)?;
    println!("{bench_txt}");

    eprintln!("fetching JSON result...");
    fs::create_dir_all("bench/results").context("failed to create bench/results")?;
    let tmp_path = format!("/tmp/bench-latest-{}.json", std::process::id());
    let scp_ok = Command::new("scp")
        .arg(format!("{VPS_HOST}:/tmp/bench-latest.json"))
        .arg(&tmp_path)
        .status()
        .context("scp failed")?
        .success();
    if scp_ok {
        save_bench_results(sh, &tmp_path)?;
        let _ = fs::remove_file(&tmp_path);

        let (do_commit, do_push) = parse_bench_vps_flags(extra_args);

        if do_commit || do_push {
            let sha_short = cmd!(sh, "git rev-parse --short HEAD")
                .read()
                .unwrap_or_default();
            let sha_short = sha_short.trim();
            cmd!(
                sh,
                "git add bench/results/bench.jsonl bench/results/latest.json"
            )
            .ignore_status()
            .run()?;
            let msg = format!("bench: vps results @ {sha_short}");
            cmd!(sh, "git commit -m {msg}").ignore_status().run()?;
            eprintln!("✓ bench results committed");
        }

        if do_push {
            cmd!(sh, "git push").run().context("git push failed")?;
            eprintln!("✓ bench results pushed");
        }
    } else {
        eprintln!("warning: scp failed — JSON not saved locally");
    }

    Ok(())
}

// ── bench-diff ────────────────────────────────────────────────────────────────

/// Compare two bench JSON files and print a delta table.
/// Usage: cargo xtask bench-diff [file-a] [file-b]
/// If only one arg, compares it against bench/results/latest.json.
/// If no args, compares the last two entries in bench/results/bench.jsonl.
fn bench_diff(args: &[String]) -> Result<()> {
    let (path_a, path_b) = match args.len() {
        0 => {
            // Find the last two entries with actual test results
            let jsonl =
                fs::read_to_string("bench/results/bench.jsonl").context("read bench.jsonl")?;
            let entries: Vec<serde_json::Value> = jsonl
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            let with_data: Vec<&serde_json::Value> = entries
                .iter()
                .filter(|e| {
                    e["suites"].as_array().is_some_and(|s| {
                        s.iter().any(|suite| {
                            suite["tests"]
                                .as_array()
                                .is_some_and(|tests| tests.iter().any(|t| !t["stats"].is_null()))
                        })
                    })
                })
                .collect();
            if with_data.len() < 2 {
                bail!(
                    "bench.jsonl has fewer than 2 entries with test results — run bench-vps at least twice"
                );
            }
            let n = with_data.len();
            print_diff(with_data[n - 2], with_data[n - 1]);
            return Ok(());
        }
        1 => (args[0].as_str(), "bench/results/latest.json"),
        _ => (args[0].as_str(), args[1].as_str()),
    };

    let a: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(path_a).with_context(|| format!("read {path_a}"))?,
    )
    .context("parse file-a")?;
    let b: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(path_b).with_context(|| format!("read {path_b}"))?,
    )
    .context("parse file-b")?;

    print_diff(&a, &b);
    Ok(())
}

fn print_diff(a: &serde_json::Value, b: &serde_json::Value) {
    let sha_a = a["metadata"]["git_sha"].as_str().unwrap_or("?");
    let sha_b = b["metadata"]["git_sha"].as_str().unwrap_or("?");
    let ts_a = a["metadata"]["timestamp"].as_str().unwrap_or("?");
    let ts_b = b["metadata"]["timestamp"].as_str().unwrap_or("?");

    println!("bench diff");
    println!(
        "  A  {} ({})",
        &sha_a[..sha_a.len().min(12)],
        &ts_a[..ts_a.len().min(19)]
    );
    println!(
        "  B  {} ({})",
        &sha_b[..sha_b.len().min(12)],
        &ts_b[..ts_b.len().min(19)]
    );
    println!();

    // Build lookup: (suite, test) -> Stats for each file
    use std::collections::HashMap;
    type Key = (String, String);
    #[derive(Debug)]
    struct S {
        avg: f64,
        p95: f64,
        unit: String,
    }

    fn extract(v: &serde_json::Value) -> HashMap<Key, S> {
        let mut m = HashMap::new();
        for suite in v["suites"].as_array().unwrap_or(&vec![]) {
            let sname = suite["name"].as_str().unwrap_or("").to_string();
            for test in suite["tests"].as_array().unwrap_or(&vec![]) {
                let tname = test["name"].as_str().unwrap_or("").to_string();
                let stats = &test["stats"];
                if stats.is_null() {
                    continue;
                }
                let avg = stats["avg"].as_f64().unwrap_or(0.0);
                let p95 = stats["p95"].as_f64().unwrap_or(0.0);
                let unit = test["unit"].as_str().unwrap_or("").to_string();
                m.insert((sname.clone(), tname), S { avg, p95, unit });
            }
        }
        m
    }

    let ma = extract(a);
    let mb = extract(b);

    // Collect all keys in order they appear in B
    let mut keys: Vec<Key> = Vec::new();
    for suite in b["suites"].as_array().unwrap_or(&vec![]) {
        let sname = suite["name"].as_str().unwrap_or("").to_string();
        for test in suite["tests"].as_array().unwrap_or(&vec![]) {
            let tname = test["name"].as_str().unwrap_or("").to_string();
            keys.push((sname.clone(), tname));
        }
    }
    // Also add any keys only in A
    for k in ma.keys() {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }

    if keys.is_empty() {
        println!("  (no test results to compare)");
        return;
    }

    // Print header
    println!(
        "{:<40} {:>10} {:>10} {:>10} {:>10} {:>8} {:>8}",
        "test", "avg-A", "avg-B", "p95-A", "p95-B", "Δavg%", "Δp95%"
    );
    println!("{}", "-".repeat(100));

    let mut regressions = 0usize;
    let mut improvements = 0usize;

    for (suite, test) in &keys {
        let sa = ma.get(&(suite.clone(), test.clone()));
        let sb = mb.get(&(suite.clone(), test.clone()));
        match (sa, sb) {
            (Some(a), Some(b)) => {
                let da = if a.avg > 0.0 {
                    (b.avg - a.avg) / a.avg * 100.0
                } else {
                    0.0
                };
                let dp = if a.p95 > 0.0 {
                    (b.p95 - a.p95) / a.p95 * 100.0
                } else {
                    0.0
                };
                let unit = if b.unit == "nanos" { "ns" } else { "µs" };
                let da_str = format!("{:+.1}%", da);
                let dp_str = format!("{:+.1}%", dp);
                let da_marker = if da > 5.0 {
                    regressions += 1;
                    "⚠"
                } else if da < -5.0 {
                    improvements += 1;
                    "✓"
                } else {
                    " "
                };
                println!(
                    "{:<40} {:>10} {:>10} {:>10} {:>10} {:>7}{} {:>8}",
                    format!("{}/{}", suite, test),
                    format!("{:.0}{}", a.avg, unit),
                    format!("{:.0}{}", b.avg, unit),
                    format!("{:.0}{}", a.p95, unit),
                    format!("{:.0}{}", b.p95, unit),
                    da_str,
                    da_marker,
                    dp_str,
                );
            }
            (None, Some(_)) => println!(
                "{:<40} {:>10} {:>10} {:>10} {:>10}   (new)",
                format!("{}/{}", suite, test),
                "-",
                "?",
                "-",
                "?"
            ),
            (Some(_), None) => println!(
                "{:<40} {:>10} {:>10} {:>10} {:>10}   (removed)",
                format!("{}/{}", suite, test),
                "?",
                "-",
                "?",
                "-"
            ),
            (None, None) => {}
        }
    }

    println!();
    println!(
        "  ⚠ regressions (>+5%): {}   ✓ improvements (<-5%): {}",
        regressions, improvements
    );
}

// ── bench-report ──────────────────────────────────────────────────────────────

/// Generate an HTML report from bench/results/bench.jsonl.
/// Output: bench/results/report.html
fn bench_report() -> Result<()> {
    let jsonl = fs::read_to_string("bench/results/bench.jsonl")
        .context("read bench/results/bench.jsonl")?;

    #[derive(Debug)]
    struct Row {
        sha: String,
        ts: String,
        suite: String,
        test: String,
        avg: f64,
        p95: f64,
        unit: String,
    }

    let mut rows: Vec<Row> = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line).context("parse bench.jsonl line")?;
        let sha = v["metadata"]["git_sha"].as_str().unwrap_or("").to_string();
        let ts = v["metadata"]["timestamp"]
            .as_str()
            .unwrap_or("")
            .to_string();
        for suite in v["suites"].as_array().unwrap_or(&vec![]) {
            let sname = suite["name"].as_str().unwrap_or("").to_string();
            for test in suite["tests"].as_array().unwrap_or(&vec![]) {
                let tname = test["name"].as_str().unwrap_or("").to_string();
                let stats = &test["stats"];
                if stats.is_null() {
                    continue;
                }
                let avg = stats["avg"].as_f64().unwrap_or(0.0);
                let p95 = stats["p95"].as_f64().unwrap_or(0.0);
                let unit = test["unit"].as_str().unwrap_or("").to_string();
                rows.push(Row {
                    sha: sha.clone(),
                    ts: ts.clone(),
                    suite: sname.clone(),
                    test: tname,
                    avg,
                    p95,
                    unit,
                });
            }
        }
    }

    if rows.is_empty() {
        bail!("no test results found in bench.jsonl — run codec/adapter benches first");
    }

    // Group by (suite, test)
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<(String, String), Vec<&Row>> = BTreeMap::new();
    for r in &rows {
        groups
            .entry((r.suite.clone(), r.test.clone()))
            .or_default()
            .push(r);
    }

    // Build chart datasets JSON
    let mut charts_js = String::new();
    let mut chart_ids = Vec::new();
    for ((suite, test), entries) in &groups {
        let id = format!("{}_{}", suite, test).replace(['-', '/'], "_");
        chart_ids.push((id.clone(), suite.clone(), test.clone()));
        let labels: Vec<String> = entries
            .iter()
            .map(|r| {
                let short = &r.sha[..r.sha.len().min(8)];
                format!("\"{}\"", short)
            })
            .collect();
        let avgs: Vec<String> = entries.iter().map(|r| format!("{:.1}", r.avg)).collect();
        let p95s: Vec<String> = entries.iter().map(|r| format!("{:.1}", r.p95)).collect();
        let unit = entries
            .last()
            .map(|r| if r.unit == "nanos" { "ns" } else { "µs" })
            .unwrap_or("µs");
        charts_js.push_str(&format!(
            r#"new Chart(document.getElementById('c_{id}'), {{
  type:'line', options:{{responsive:true,plugins:{{legend:{{position:'top'}},title:{{display:true,text:'{suite}/{test} ({unit})'}}}},scales:{{y:{{beginAtZero:false}}}}}},
  data:{{ labels:[{labels}], datasets:[
    {{label:'avg',data:[{avgs}],borderColor:'#4f88e3',backgroundColor:'#4f88e322',fill:true,tension:0.3}},
    {{label:'p95',data:[{p95s}],borderColor:'#e3834f',backgroundColor:'#e3834f22',fill:true,tension:0.3}}
  ]}}
}});
"#,
            id=id, suite=suite, test=test, unit=unit,
            labels=labels.join(","), avgs=avgs.join(","), p95s=p95s.join(",")
        ));
    }

    // Latest values table
    let mut latest_table = String::new();
    for ((suite, test), entries) in &groups {
        if let Some(r) = entries.last() {
            let unit = if r.unit == "nanos" { "ns" } else { "µs" };
            latest_table.push_str(&format!(
                "<tr><td>{suite}</td><td>{test}</td><td class='num'>{:.0}{unit}</td><td class='num'>{:.0}{unit}</td></tr>\n",
                r.avg, r.p95
            ));
        }
    }

    let canvas_tags: String = chart_ids
        .iter()
        .map(|(id, _, _)| format!("<div class='chart-wrap'><canvas id='c_{id}'></canvas></div>"))
        .collect::<Vec<_>>()
        .join("\n");

    let commit_count = rows
        .iter()
        .map(|r| r.sha.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let ts_latest = rows.iter().map(|r| r.ts.as_str()).max().unwrap_or("");

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>minibox bench report</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4/dist/chart.umd.min.js"></script>
<style>
  body {{ font-family: ui-monospace, monospace; background: #0e1117; color: #cdd6f4; margin: 0; padding: 2rem; }}
  h1 {{ color: #89b4fa; margin-bottom: .25rem; }}
  .meta {{ color: #6c7086; font-size: .85rem; margin-bottom: 2rem; }}
  table {{ border-collapse: collapse; width: 100%; margin-bottom: 3rem; }}
  th {{ text-align: left; padding: .4rem .8rem; border-bottom: 1px solid #313244; color: #89b4fa; }}
  td {{ padding: .35rem .8rem; border-bottom: 1px solid #1e1e2e; }}
  td.num {{ text-align: right; color: #a6e3a1; }}
  .charts {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(400px, 1fr)); gap: 1.5rem; }}
  .chart-wrap {{ background: #1e1e2e; border-radius: 8px; padding: 1rem; }}
</style>
</head>
<body>
<h1>minibox bench report</h1>
<p class="meta">{commit_count} commits · latest {ts_latest}</p>

<h2 style="color:#cba6f7">Latest values</h2>
<table>
<thead><tr><th>suite</th><th>test</th><th>avg</th><th>p95</th></tr></thead>
<tbody>
{latest_table}
</tbody>
</table>

<h2 style="color:#cba6f7">Trends</h2>
<div class="charts">
{canvas_tags}
</div>

<script>
{charts_js}
</script>
</body>
</html>
"#,
        commit_count = commit_count,
        ts_latest = &ts_latest[..ts_latest.len().min(19)],
        latest_table = latest_table,
        canvas_tags = canvas_tags,
        charts_js = charts_js,
    );

    let out = "bench/results/report.html";
    fs::write(out, &html).with_context(|| format!("write {out}"))?;
    eprintln!(
        "✓ {out}  ({} charts, {} data points)",
        chart_ids.len(),
        rows.len()
    );
    Ok(())
}

#[cfg(test)]
mod bench_vps_args_tests {
    use super::parse_bench_vps_flags;

    #[test]
    fn bench_vps_args_default_no_commit_no_push() {
        let args: Vec<String> = vec![];
        let (commit, push) = parse_bench_vps_flags(&args);
        assert!(!commit);
        assert!(!push);
    }

    #[test]
    fn bench_vps_args_explicit_flags() {
        let args = vec!["--commit".to_string(), "--push".to_string()];
        let (commit, push) = parse_bench_vps_flags(&args);
        assert!(commit);
        assert!(push);
    }

    #[test]
    fn bench_vps_args_commit_only() {
        let args = vec!["--commit".to_string()];
        let (commit, push) = parse_bench_vps_flags(&args);
        assert!(commit);
        assert!(!push);
    }
}
