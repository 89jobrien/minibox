//! `cargo xtask bench` — run criterion benchmarks and save results.
//!
//! Runs `cargo bench -p minibox` and writes a summary to
//! `bench/results/latest.json` (and an append-only `bench.jsonl`).

use anyhow::{Context, Result};
use std::path::Path;
use xshell::{Shell, cmd};

pub fn bench(sh: &Shell, root: &Path) -> Result<()> {
    let results_dir = root.join("bench/results");
    std::fs::create_dir_all(&results_dir).context("create bench/results")?;

    eprintln!("$ cargo bench -p minibox");
    cmd!(sh, "cargo bench -p minibox").run()?;

    let commit = cmd!(sh, "git rev-parse --short HEAD").read()?;
    let timestamp = cmd!(sh, "date -u +%Y-%m-%dT%H:%M:%SZ").read()?;

    let summary = serde_json::json!({
        "commit": commit.trim(),
        "timestamp": timestamp.trim(),
        "benches": ["trait_overhead", "protocol_codec"],
        "status": "pass",
    });

    let latest = results_dir.join("latest.json");
    std::fs::write(&latest, serde_json::to_string_pretty(&summary)?)
        .context("write latest.json")?;

    let jsonl = results_dir.join("bench.jsonl");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&jsonl)
        .context("open bench.jsonl")?;
    writeln!(f, "{}", serde_json::to_string(&summary)?)?;

    eprintln!("Bench results written to {}", latest.display());
    Ok(())
}
