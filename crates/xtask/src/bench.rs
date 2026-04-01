use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    io::Write,
    process::{Command, Stdio},
};
use xshell::{Shell, cmd};

pub const VPS_HOST: &str = "jobrien-vm";
const BENCH_BIN: &str = "/home/dev/minibox/target/release/minibox-bench";

// ── Local bench ───────────────────────────────────────────────────────────────

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

    let output = cmd!(sh, "./target/release/minibox-bench {args...}")
        .read()
        .context("bench binary failed")?;

    let json_path = output
        .lines()
        .last()
        .filter(|l| l.ends_with(".json"))
        .ok_or_else(|| anyhow::anyhow!("bench binary did not print a .json path on stdout"))?
        .to_string();

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
    if let Ok(micro_out) = cmd!(sh, "./target/release/minibox-bench {micro_args...}").read() {
        if let Some(micro_path) = micro_out.lines().last().filter(|l| l.ends_with(".json")) {
            merge_bench_suites(&json_path, micro_path).ok();
        }
    }

    save_bench_results(sh, &json_path)
}

/// Merge suites from `src_path` into `dst_path` in place.
///
/// Appends any suites in `src` that are not already present in `dst` by name.
/// Used to combine codec/adapter microbench results with the main cold/warm run.
pub fn merge_bench_suites(dst_path: &str, src_path: &str) -> Result<()> {
    let dst_content = fs::read_to_string(dst_path).context("read dst bench JSON")?;
    let src_content = fs::read_to_string(src_path).context("read src bench JSON")?;
    let mut dst: serde_json::Value =
        serde_json::from_str(&dst_content).context("parse dst bench JSON")?;
    let src: serde_json::Value =
        serde_json::from_str(&src_content).context("parse src bench JSON")?;

    let dst_suites = dst["suites"].as_array_mut().context("dst missing suites")?;
    let existing: HashSet<String> = dst_suites
        .iter()
        .filter_map(|s| s["name"].as_str().map(|n| n.to_string()))
        .collect();

    if let Some(src_suites) = src["suites"].as_array() {
        for suite in src_suites {
            if let Some(name) = suite["name"].as_str() {
                if !existing.contains(name) {
                    dst_suites.push(suite.clone());
                }
            }
        }
    }

    let merged = serde_json::to_string_pretty(&dst).context("re-serialise merged JSON")?;
    fs::write(dst_path, merged).context("write merged bench JSON")?;
    Ok(())
}

/// Patch git_sha, append to bench.jsonl, and update latest.json.
/// Remove suites where every test has `iterations == 0` (fixture setup failed or
/// daemon was unreachable). These produce no signal and pollute baselines.
/// Returns the number of suites dropped.
fn strip_zero_iteration_suites(json: &mut serde_json::Value) -> usize {
    let Some(suites) = json["suites"].as_array_mut() else {
        return 0;
    };
    let before = suites.len();
    suites.retain(|suite| {
        suite["tests"].as_array().is_some_and(|tests| {
            tests
                .iter()
                .any(|t| t["iterations"].as_u64().unwrap_or(0) > 0)
        })
    });
    before - suites.len()
}

pub fn save_bench_results(sh: &Shell, json_path: &str) -> Result<()> {
    let content = fs::read_to_string(json_path).context("read bench JSON")?;
    let mut json: serde_json::Value =
        serde_json::from_str(&content).context("invalid bench JSON")?;
    if let Ok(sha) = cmd!(sh, "git rev-parse HEAD").read() {
        let sha = sha.trim();
        if !sha.is_empty() {
            json["metadata"]["git_sha"] = serde_json::Value::String(sha.to_string());
        }
    }
    let dropped = strip_zero_iteration_suites(&mut json);
    if dropped > 0 {
        eprintln!("bench: dropped {dropped} suite(s) with zero iterations (fixture failures)");
    }
    let line = serde_json::to_string(&json).context("re-serialise failed")?;
    let jsonl_path = "bench/results/bench.jsonl";
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(jsonl_path)
        .context("open bench.jsonl")?;
    writeln!(file, "{line}").context("append to bench.jsonl")?;
    eprintln!("✓ bench/results/bench.jsonl appended");

    let pretty = serde_json::to_string_pretty(&json).context("pretty-print failed")?;
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

    // Collect existing timestamps so we can skip duplicates.
    let existing_timestamps: HashSet<String> = if fs::metadata(local_path).is_ok() {
        fs::read_to_string(local_path)
            .context("read local bench.jsonl")?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v["metadata"]["timestamp"].as_str().map(|s| s.to_string()))
            .collect()
    } else {
        HashSet::new()
    };

    let vps_content = fs::read_to_string(&tmp).context("read tmp vps bench.jsonl")?;
    let _ = fs::remove_file(&tmp);

    // Parse new VPS entries, dedup by timestamp, and drop zero-iteration suites.
    let mut new_lines: Vec<String> = Vec::new();
    let mut skipped_zero = 0usize;
    for l in vps_content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(mut v) = serde_json::from_str::<serde_json::Value>(l) else {
            continue;
        };
        let ts = v["metadata"]["timestamp"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if ts.is_empty() || existing_timestamps.contains(&ts) {
            continue;
        }
        let dropped = strip_zero_iteration_suites(&mut v);
        skipped_zero += dropped;
        new_lines.push(serde_json::to_string(&v).context("re-serialise VPS entry")?);
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
    let (path_a, path_b) = match args.len() {
        0 => {
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

pub fn print_diff(a: &serde_json::Value, b: &serde_json::Value) {
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

    let mut keys: Vec<Key> = Vec::new();
    for suite in b["suites"].as_array().unwrap_or(&vec![]) {
        let sname = suite["name"].as_str().unwrap_or("").to_string();
        for test in suite["tests"].as_array().unwrap_or(&vec![]) {
            let tname = test["name"].as_str().unwrap_or("").to_string();
            keys.push((sname.clone(), tname));
        }
    }
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
}
