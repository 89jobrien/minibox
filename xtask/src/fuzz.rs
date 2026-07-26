//! `cargo xtask fuzz` — run libFuzzer fuzz targets against the protocol codec.
//!
//! Requires a nightly toolchain (`rustup toolchain install nightly`).
//!
//! Usage:
//!   cargo xtask fuzz                         # both targets, 60 s each
//!   cargo xtask fuzz --time 300              # 5 min each
//!   cargo xtask fuzz --target `fuzz_decode_request`
//!   cargo xtask fuzz --jobs 1               # sequential (default: parallel)

use anyhow::{Context, Result, bail};
use std::path::Path;
use xshell::{Shell, cmd};

const FUZZ_MANIFEST: &str = "crates/minibox/fuzz/Cargo.toml";
const DEFAULT_TARGETS: &[&str] = &[
    // Tier 1: protocol codec
    "fuzz_decode_request",
    "fuzz_decode_response",
    // Tier 2: layer extraction and path validation
    "fuzz_extract_layer",
    "fuzz_validate_tar_path",
    // Tier 3: manifest / image-ref parsing
    "fuzz_parse_manifest",
    "fuzz_parse_platform",
    "fuzz_parse_image_ref",
];
const DEFAULT_TIME_SECS: u64 = 60;

pub fn fuzz(sh: &Shell, root: &Path) -> Result<()> {
    let args: Vec<String> = std::env::args().skip(2).collect();

    // Parse --time <secs>
    let time_secs: u64 = args
        .windows(2)
        .find(|w| w[0] == "--time")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(DEFAULT_TIME_SECS);

    // Parse --target <name>
    let explicit_target: Option<String> = args
        .windows(2)
        .find(|w| w[0] == "--target")
        .map(|w| w[1].clone());

    let targets: Vec<&str> = match &explicit_target {
        Some(t) => {
            if !DEFAULT_TARGETS.contains(&t.as_str()) {
                bail!(
                    "unknown fuzz target {t:?}. Available: {}",
                    DEFAULT_TARGETS.join(", ")
                );
            }
            vec![t.as_str()]
        }
        None => DEFAULT_TARGETS.to_vec(),
    };

    // Verify nightly is available.
    cmd!(sh, "rustup run nightly cargo --version")
        .quiet()
        .run()
        .context("nightly toolchain not found; run: rustup toolchain install nightly")?;

    let manifest = root.join(FUZZ_MANIFEST);
    let time = time_secs.to_string();

    for target in targets {
        // corpus/  — fuzzer writes new interesting inputs here (gitignored)
        // seeds/   — human-authored seed inputs (committed)
        let corpus = root.join(format!("crates/minibox/fuzz/corpus/{target}"));
        let seeds = root.join(format!("crates/minibox/fuzz/seeds/{target}"));
        std::fs::create_dir_all(&corpus)
            .with_context(|| format!("create corpus dir for {target}"))?;

        eprintln!("fuzz: running {target} for {time_secs}s …");
        // libFuzzer accepts multiple corpus directories; new entries go to the first one.
        cmd!(
            sh,
            "cargo +nightly fuzz run {target}
                --manifest-path {manifest}
                {corpus}
                {seeds}
                -- -max_total_time={time}"
        )
        .run()
        .with_context(|| format!("fuzz target {target} crashed — check fuzz/artifacts/"))?;

        eprintln!("fuzz: {target} completed ({time_secs}s, no crashes)");
    }

    Ok(())
}
