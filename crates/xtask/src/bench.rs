use crate::bench_types::BenchReport;
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};
use xshell::{Shell, cmd};

pub const VPS_HOST: &str = "jobrien-vm";
const BENCH_BIN: &str = "/home/dev/minibox/target/release/minibox-bench";

// ── Local bench ───────────────────────────────────────────────────────────────

/// Return the path of the most recently modified `.json` file in `dir`,
/// skipping `bench.jsonl` and `latest.json` which are managed separately.
fn newest_json_in_dir(dir: &str) -> Option<String> {
    let entries = fs::read_dir(dir).ok()?;
    entries
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !name.ends_with(".json") || name == "latest.json" {
                return None;
            }
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path().to_string_lossy().into_owned()))
        })
        .max_by_key(|(t, _)| *t)
        .map(|(_, path)| path)
}

/// Spawn `bin` with `args`, stream each line to stderr in real time, and return
/// the last line that ends with `.json` (the output path printed by minibox-bench).
fn stream_bench_last_json(bin: &str, args: &[&str]) -> Result<Option<String>> {
    let mut child = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn {bin}"))?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let reader = BufReader::new(stdout);

    let mut last_json: Option<String> = None;
    for line in reader.lines() {
        let line = line.context("read bench stdout")?;
        eprintln!("{line}");
        if line.ends_with(".json") {
            last_json = Some(line);
        }
    }

    let status = child.wait().context("wait bench")?;
    if !status.success() {
        bail!("{bin} exited with {status}");
    }
    Ok(last_json)
}

/// Run benchmark binary (local, requires Linux + miniboxd running) and save results.
pub fn bench(sh: &Shell, extra_args: &[String]) -> Result<()> {
    let out_dir = "bench/results";
    fs::create_dir_all(out_dir).context("create bench/results")?;

    cmd!(sh, "cargo build --release -p minibox-bench")
        .run()
        .context("build minibox-bench")?;

    let mut args: Vec<&str> = vec!["--out-dir", out_dir];
    if extra_args.iter().any(|a| a == "--profile") {
        args.push("--profile");
    }

    let json_path = stream_bench_last_json("./target/release/minibox-bench", &args)
        .context("bench binary failed")?
        .or_else(|| {
            eprintln!("warn: bench binary did not print a .json path on stdout; falling back to newest file in {out_dir}");
            newest_json_in_dir(out_dir)
        })
        .ok_or_else(|| anyhow::anyhow!("no .json output found in {out_dir}"))?;

    let micro_args: Vec<&str> = vec![
        "--out-dir",
        out_dir,
        "--suite",
        "codec",
        "--suite",
        "adapter",
        "--no-cold",
        "--no-warm",
    ];
    if let Ok(Some(micro_path)) =
        stream_bench_last_json("./target/release/minibox-bench", &micro_args)
    {
        merge_bench_suites(&json_path, &micro_path).ok();
    }

    save_bench_results(sh, &json_path)
}

/// Merge suites from `src_path` into `dst_path` in place.
///
/// Appends any suites in `src` that are not already present in `dst` by name.
pub fn merge_bench_suites(dst_path: &str, src_path: &str) -> Result<()> {
    let mut dst: BenchReport =
        serde_json::from_str(&fs::read_to_string(dst_path).context("read dst bench JSON")?)
            .context("parse dst bench JSON")?;

    let src: BenchReport =
        serde_json::from_str(&fs::read_to_string(src_path).context("read src bench JSON")?)
            .context("parse src bench JSON")?;

    let existing: HashSet<String> = dst.suites.iter().map(|s| s.name.clone()).collect();
    let new_suites: Vec<_> = src
        .suites
        .into_iter()
        .filter(|s| !existing.contains(&s.name))
        .collect();
    dst.suites.extend(new_suites);

    fs::write(
        dst_path,
        serde_json::to_string_pretty(&dst).context("re-serialise merged JSON")?,
    )
    .context("write merged bench JSON")
}

/// Drop raw sample vectors from all test results, keeping only aggregates.
///
/// `bench.jsonl` stores aggregates only; the timestamped `.json` file written
/// by minibox-bench retains the raw samples for offline analysis.
fn strip_raw_samples(report: &mut BenchReport) {
    for suite in &mut report.suites {
        for test in &mut suite.tests {
            test.durations_micros.clear();
            test.durations_nanos.clear();
        }
    }
}

/// Remove suites where every test has `iterations == 0`.
/// Returns the number of suites dropped.
fn strip_zero_iteration_suites(report: &mut BenchReport) -> usize {
    let before = report.suites.len();
    report
        .suites
        .retain(|suite| suite.tests.iter().any(|t| t.iterations > 0));
    before - report.suites.len()
}

/// Normalise a raw hostname to `"vps"` or `"local"`.
fn redact_hostname(hostname: &str) -> &'static str {
    if hostname.contains("vps")
        || hostname.contains("vm")
        || hostname.contains("runner")
        || hostname.contains("ci")
    {
        "vps"
    } else {
        "local"
    }
}

pub fn save_bench_results(sh: &Shell, json_path: &str) -> Result<()> {
    let mut report: BenchReport =
        serde_json::from_str(&fs::read_to_string(json_path).context("read bench JSON")?)
            .context("invalid bench JSON")?;

    if let Ok(sha) = cmd!(sh, "git rev-parse HEAD").read() {
        let sha = sha.trim();
        if !sha.is_empty() {
            report.metadata.git_sha = sha.to_string();
        }
    }

    report.metadata.hostname = redact_hostname(&report.metadata.hostname).to_string();

    let dropped = strip_zero_iteration_suites(&mut report);
    if dropped > 0 {
        eprintln!("bench: dropped {dropped} suite(s) with zero iterations (fixture failures)");
    }
    strip_raw_samples(&mut report);

    let line = serde_json::to_string(&report).context("serialise bench JSON")?;
    let jsonl_path = "bench/results/bench.jsonl";
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(jsonl_path)
        .context("open bench.jsonl")?;
    writeln!(file, "{line}").context("append to bench.jsonl")?;
    eprintln!("✓ bench/results/bench.jsonl appended");

    let pretty = serde_json::to_string_pretty(&report).context("pretty-print bench JSON")?;
    fs::write("bench/results/latest.json", pretty).context("write latest.json")?;
    eprintln!("✓ bench/results/latest.json updated");
    Ok(())
}

// ── VPS bench ─────────────────────────────────────────────────────────────────

/// Parse --commit / --push flags from extra args. Returns (commit, push).
pub fn parse_bench_vps_flags(args: &[String]) -> (bool, bool) {
    let commit = args.iter().any(|a| a == "--commit");
    let push = args.iter().any(|a| a == "--push");
    (commit, push)
}

/// Run a script on the remote as root via key-based SSH auth.
fn ssh_sudo_script(sudo_pass: &str, script: &str) -> Result<String> {
    let tmpfile = format!("/tmp/xtask-bench-{}.sh", std::process::id());

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

    let run_cmd = format!("sudo -S bash '{tmpfile}'; RC=$?; rm -f '{tmpfile}'; exit $RC");
    let mut run = Command::new("ssh")
        .arg(VPS_HOST)
        .arg(&run_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("ssh sudo run failed")?;
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
pub fn bench_vps(sh: &Shell, extra_args: &[String]) -> Result<()> {
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
MICRO_DIR="/tmp/bench-micro-$$"
rm -rf "$MICRO_DIR"
"$BENCH_BIN" --suite codec --suite adapter --no-cold --no-warm --out-dir "$MICRO_DIR" 2>/dev/null || true
MICRO_JSON=$(ls -t "$MICRO_DIR"/*.json 2>/dev/null | head -1)
if [ -n "$MICRO_JSON" ]; then
  python3 -c "
import json, sys
dst = json.load(open('$JSON_FILE'))
src = json.load(open('$MICRO_JSON'))
existing = {{s['name'] for s in dst.get('suites', [])}}
for s in src.get('suites', []):
    if s['name'] not in existing:
        dst['suites'].append(s)
json.dump(dst, open('$JSON_FILE', 'w'), indent=2)
" 2>/dev/null || true
fi
rm -rf "$MICRO_DIR"
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
            .run()
            .context("git add bench results")?;
            let msg = format!("bench: vps results @ {sha_short}");
            cmd!(sh, "git commit -m {msg}")
                .run()
                .context("git commit bench results")?;
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

// ── bench-sync ────────────────────────────────────────────────────────────────

/// Rsync VPS bench.jsonl and merge net-new entries into the local copy.
///
/// Deduplicates by `metadata.timestamp` — safe to run repeatedly.
pub fn bench_sync() -> Result<()> {
    let tmp = format!("/tmp/bench-sync-vps-{}.jsonl", std::process::id());

    eprintln!("fetching VPS bench.jsonl...");
    let status = Command::new("rsync")
        .args([
            "-az",
            "--compress",
            &format!("{VPS_HOST}:/home/dev/minibox/bench/results/bench.jsonl"),
            &tmp,
        ])
        .status()
        .context("rsync failed — is jobrien-vm reachable?")?;
    if !status.success() {
        bail!("rsync exited with status {status}");
    }

    let local_path = "bench/results/bench.jsonl";
    fs::create_dir_all("bench/results").context("create bench/results")?;

    let existing_timestamps: HashSet<String> = if fs::metadata(local_path).is_ok() {
        fs::read_to_string(local_path)
            .context("read local bench.jsonl")?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<BenchReport>(l).ok())
            .filter(|r| !r.metadata.timestamp.is_empty())
            .map(|r| r.metadata.timestamp)
            .collect()
    } else {
        HashSet::new()
    };

    let vps_content = fs::read_to_string(&tmp).context("read tmp vps bench.jsonl")?;
    let _ = fs::remove_file(&tmp);

    let mut new_lines: Vec<String> = Vec::new();
    let mut skipped_zero = 0usize;
    for l in vps_content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(mut report) = serde_json::from_str::<BenchReport>(l) else {
            continue;
        };
        if report.metadata.timestamp.is_empty()
            || existing_timestamps.contains(&report.metadata.timestamp)
        {
            continue;
        }
        let dropped = strip_zero_iteration_suites(&mut report);
        skipped_zero += dropped;
        report.metadata.hostname = redact_hostname(&report.metadata.hostname).to_string();
        strip_raw_samples(&mut report);
        new_lines.push(serde_json::to_string(&report).context("re-serialise VPS entry")?);
    }

    if new_lines.is_empty() {
        eprintln!(
            "bench-sync: already up to date ({} existing entries{})",
            existing_timestamps.len(),
            if skipped_zero > 0 {
                format!(", {skipped_zero} zero-iter suite(s) dropped")
            } else {
                String::new()
            }
        );
        return Ok(());
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(local_path)
        .context("open bench.jsonl for append")?;
    for entry in &new_lines {
        writeln!(file, "{entry}").context("append entry to bench.jsonl")?;
    }

    eprintln!(
        "✓ bench-sync: merged {} new VPS entries into bench/results/bench.jsonl{}",
        new_lines.len(),
        if skipped_zero > 0 {
            format!(" ({skipped_zero} zero-iter suite(s) stripped)")
        } else {
            String::new()
        }
    );
    Ok(())
}

// ── bench-diff ────────────────────────────────────────────────────────────────

/// Compare two bench JSON files and print a delta table.
pub fn bench_diff(args: &[String]) -> Result<()> {
    let parse_file = |path: &str| -> Result<BenchReport> {
        serde_json::from_str(&fs::read_to_string(path).with_context(|| format!("read {path}"))?)
            .with_context(|| format!("parse {path}"))
    };

    let (a, b) = match args.len() {
        0 => {
            let jsonl =
                fs::read_to_string("bench/results/bench.jsonl").context("read bench.jsonl")?;
            let entries: Vec<BenchReport> = jsonl
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            let with_data: Vec<&BenchReport> = entries
                .iter()
                .filter(|r| {
                    r.suites
                        .iter()
                        .any(|suite| suite.tests.iter().any(|t| t.stats.is_some()))
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
        1 => (
            parse_file(args[0].as_str())?,
            parse_file("bench/results/latest.json")?,
        ),
        _ => (parse_file(args[0].as_str())?, parse_file(args[1].as_str())?),
    };

    print_diff(&a, &b);
    Ok(())
}

pub fn print_diff(a: &BenchReport, b: &BenchReport) {
    let sha_a = &a.metadata.git_sha;
    let sha_b = &b.metadata.git_sha;
    let ts_a = &a.metadata.timestamp;
    let ts_b = &b.metadata.timestamp;

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

    type Key = (String, String);
    struct S {
        avg: f64,
        p95: f64,
        unit: String,
    }

    let extract = |report: &BenchReport| -> std::collections::HashMap<Key, S> {
        let mut m = std::collections::HashMap::new();
        for suite in &report.suites {
            for test in &suite.tests {
                let Some(stats) = &test.stats else { continue };
                m.insert(
                    (suite.name.clone(), test.name.clone()),
                    S {
                        avg: stats.avg as f64,
                        p95: stats.p95 as f64,
                        unit: test.unit.clone(),
                    },
                );
            }
        }
        m
    };

    let ma = extract(a);
    let mb = extract(b);

    // Build key list in b's suite/test order, then append any a-only keys.
    let mut keys: Vec<Key> = b
        .suites
        .iter()
        .flat_map(|suite| {
            suite
                .tests
                .iter()
                .map(|t| (suite.name.clone(), t.name.clone()))
        })
        .collect();
    for k in ma.keys() {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }

    if keys.is_empty() {
        println!("  (no test results to compare)");
        return;
    }

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
pub fn bench_report() -> Result<()> {
    let jsonl = fs::read_to_string("bench/results/bench.jsonl")
        .context("read bench/results/bench.jsonl")?;

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
        let report: BenchReport = serde_json::from_str(line).context("parse bench.jsonl line")?;
        let sha = report.metadata.git_sha.clone();
        let ts = report.metadata.timestamp.clone();
        for suite in &report.suites {
            for test in &suite.tests {
                let Some(stats) = &test.stats else { continue };
                rows.push(Row {
                    sha: sha.clone(),
                    ts: ts.clone(),
                    suite: suite.name.clone(),
                    test: test.name.clone(),
                    avg: stats.avg as f64,
                    p95: stats.p95 as f64,
                    unit: test.unit.clone(),
                });
            }
        }
    }

    if rows.is_empty() {
        bail!("no test results found in bench.jsonl — run codec/adapter benches first");
    }

    let mut groups: BTreeMap<(String, String), Vec<&Row>> = BTreeMap::new();
    for r in &rows {
        groups
            .entry((r.suite.clone(), r.test.clone()))
            .or_default()
            .push(r);
    }

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
            id = id,
            suite = suite,
            test = test,
            unit = unit,
            labels = labels.join(","),
            avgs = avgs.join(","),
            p95s = p95s.join(",")
        ));
    }

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
        .collect::<BTreeSet<_>>()
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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

    #[test]
    fn strip_zero_iterations_removes_empty_suites() {
        #[allow(unused_imports)]
        use crate::bench_types::Stats;
        use crate::bench_types::{BenchReport, SuiteResult, TestResult};
        let mut report = BenchReport::default();
        report.suites.push(SuiteResult {
            name: "has_data".to_string(),
            tests: vec![TestResult {
                name: "t1".to_string(),
                iterations: 5,
                ..Default::default()
            }],
        });
        report.suites.push(SuiteResult {
            name: "all_zero".to_string(),
            tests: vec![TestResult {
                name: "t2".to_string(),
                iterations: 0,
                ..Default::default()
            }],
        });
        let dropped = super::strip_zero_iteration_suites(&mut report);
        assert_eq!(dropped, 1);
        assert_eq!(report.suites.len(), 1);
        assert_eq!(report.suites[0].name, "has_data");
    }

    #[test]
    fn redact_hostname_classifies_correctly() {
        use super::redact_hostname;
        assert_eq!(redact_hostname("jobrien-vm"), "vps");
        assert_eq!(redact_hostname("github-runner-01"), "vps");
        assert_eq!(redact_hostname("macbook-pro"), "local");
        assert_eq!(redact_hostname("ci-build"), "vps");
    }

    #[test]
    fn merge_bench_suites_deduplicates() {
        use crate::bench_types::{BenchReport, SuiteResult};
        use std::io::Write;
        use tempfile::NamedTempFile;

        let make_report = |suite_names: &[&str]| -> String {
            let mut r = BenchReport::default();
            for name in suite_names {
                r.suites.push(SuiteResult {
                    name: name.to_string(),
                    tests: vec![],
                });
            }
            serde_json::to_string(&r).unwrap()
        };

        let mut dst = NamedTempFile::new().unwrap();
        dst.write_all(make_report(&["codec", "adapter"]).as_bytes())
            .unwrap();
        let dst_path = dst.path().to_str().unwrap().to_string();

        let mut src = NamedTempFile::new().unwrap();
        src.write_all(make_report(&["adapter", "pull"]).as_bytes())
            .unwrap();
        let src_path = src.path().to_str().unwrap().to_string();

        super::merge_bench_suites(&dst_path, &src_path).unwrap();

        let merged: BenchReport =
            serde_json::from_str(&std::fs::read_to_string(&dst_path).unwrap()).unwrap();
        let names: Vec<&str> = merged.suites.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["codec", "adapter", "pull"]);
    }
}
