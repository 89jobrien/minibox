use anyhow::{Context, Result, bail};
use std::{
    env, fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use xshell::{Shell, cmd};

fn main() -> Result<()> {
    let task = env::args().nth(1);
    let sh = Shell::new()?;

    // Run from workspace root
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    sh.change_dir(root);

    match task.as_deref() {
        Some("pre-commit") => pre_commit(&sh),
        Some("prepush") => prepush(&sh),
        Some("test-unit") => test_unit(&sh),
        Some("test-property") => test_property(&sh),
        Some("test-integration") => test_integration(&sh),
        Some("test-e2e-suite") => test_e2e_suite(&sh),
        Some("clean-artifacts") => clean_artifacts(&sh),
        Some("nuke-test-state") => nuke_test_state(&sh),
        Some("bench") => bench(&sh),
        Some("bench-vps") => bench_vps(&sh),
        Some(other) => bail!("unknown task: {other}"),
        None => {
            eprintln!("Available tasks:");
            eprintln!("  pre-commit       fmt-check + lint + build-release");
            eprintln!("  prepush          nextest + coverage");
            eprintln!("  test-unit        all unit + conformance tests");
            eprintln!("  test-property    property-based tests (proptest)");
            eprintln!("  test-integration cgroup + integration tests (Linux, root)");
            eprintln!("  test-e2e-suite   daemon+CLI e2e tests (Linux, root)");
            eprintln!("  clean-artifacts  remove non-critical build outputs");
            eprintln!("  nuke-test-state  kill orphans, unmount overlays, clean cgroups");
            eprintln!("  bench            run benchmark binary (local, dry-run safe)");
            eprintln!(
                "  bench-vps        run benchmark on VPS, append to bench/results/bench.jsonl"
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
        "cargo clippy -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -- -D warnings"
    )
    .run()
    .context("lint failed")?;
    cmd!(sh,
        "cargo build --release -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox -p minibox-bench"
    ).run().context("build-release failed")?;
    eprintln!("pre-commit checks passed");
    Ok(())
}

/// Pre-push gate: nextest + coverage
fn prepush(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo nextest run --release -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox"
    )
    .run()
    .context("nextest failed")?;
    cmd!(
        sh,
        "cargo llvm-cov nextest -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox --html"
    )
    .run()
    .context("coverage failed")?;
    eprintln!("coverage: target/llvm-cov/html/index.html");
    Ok(())
}

/// All unit + conformance tests (any platform)
fn test_unit(sh: &Shell) -> Result<()> {
    cmd!(
        sh,
        "cargo test --release -p minibox-lib -p minibox-macros -p minibox-cli -p daemonbox --lib"
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
    cmd!(
        sh,
        "cargo test --release -p minibox-lib --test proptest_suite"
    )
    .run()
    .context("minibox-lib property tests failed")?;
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
            let is_file = e.file_type().map_or(false, |t| t.is_file());
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
                if entry.file_type().ok().map_or(false, |t| t.is_file()) {
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
                let keep = path.extension().map_or(false, |e| e == "d");
                if !keep && entry.file_type().ok().map_or(false, |t| t.is_file()) {
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

/// Run benchmark binary (local, requires Linux + miniboxd running)
fn bench(sh: &Shell) -> Result<()> {
    cmd!(sh, "./target/release/minibox-bench --dry-run").run()?;
    cmd!(sh, "./target/release/minibox-bench").run()?;
    Ok(())
}

// ── VPS bench helpers ────────────────────────────────────────────────────────

const VPS_HOST: &str = "dev@100.105.75.7";
const BENCH_BIN: &str = "/home/dev/minibox/target/release/minibox-bench";

/// SSH options that bypass all key auth and force password-only.
/// Passed as individual args to avoid any shell quoting or template issues.
fn ssh_opts() -> Vec<&'static str> {
    vec![
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "IdentityAgent=none",
        "-o",
        "PubkeyAuthentication=no",
        "-o",
        "PreferredAuthentications=password",
        "-o",
        "StrictHostKeyChecking=no",
    ]
}

/// Run a script on the remote as root. Two SSH calls:
/// 1. Upload script to a temp file (script on stdin, no sudo password conflict).
/// 2. Execute with `sudo -S bash <tmpfile>` (password on stdin, script from file).
fn ssh_sudo_script(pass: &str, script: &str) -> Result<String> {
    let tmpfile = format!("/tmp/xtask-bench-{}.sh", std::process::id());

    // Step 1: upload script
    let write_cmd = format!("cat > '{tmpfile}' && chmod 700 '{tmpfile}'");
    let mut upload = Command::new("sshpass")
        .arg("-p")
        .arg(pass)
        .arg("ssh")
        .args(ssh_opts())
        .arg(VPS_HOST)
        .arg(&write_cmd)
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn sshpass for script upload")?;
    upload
        .stdin
        .take()
        .context("no stdin")?
        .write_all(script.as_bytes())
        .context("failed to write script")?;
    if !upload.wait().context("script upload wait")?.success() {
        bail!("failed to write script to remote");
    }

    // Step 2: run as root; clean up regardless of exit code
    let run_cmd = format!(
        "echo '{}' | sudo -S bash '{tmpfile}'; RC=$?; rm -f '{tmpfile}'; exit $RC",
        pass.replace('\'', "'\\''"),
    );
    let out = Command::new("sshpass")
        .arg("-p")
        .arg(pass)
        .arg("ssh")
        .args(ssh_opts())
        .arg(VPS_HOST)
        .arg(&run_cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .context("ssh sudo run failed")?;
    if !out.status.success() {
        bail!("remote script exited with status {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run minibox-bench on the VPS as root and append results to bench/results/bench.jsonl.
fn bench_vps(sh: &Shell) -> Result<()> {
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
    let scp_ok = Command::new("sshpass")
        .arg("-p")
        .arg(&vps_pass)
        .arg("scp")
        .args(ssh_opts())
        .arg(format!("{VPS_HOST}:/tmp/bench-latest.json"))
        .arg(&tmp_path)
        .status()
        .context("scp failed")?
        .success();
    if scp_ok {
        if let Ok(content) = fs::read_to_string(&tmp_path) {
            // Compact to a single line for JSONL; patch git_sha with local HEAD.
            let mut json: serde_json::Value =
                serde_json::from_str(&content).context("invalid JSON from VPS")?;
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

            // Commit and push bench results
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
            cmd!(sh, "git push").run().context("git push failed")?;
            eprintln!("✓ bench results committed and pushed");
        }
        let _ = fs::remove_file(&tmp_path);
    } else {
        eprintln!("warning: scp failed — JSON not saved locally");
    }

    Ok(())
}
