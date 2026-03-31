use anyhow::{Context, Result, bail};
use std::{
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use xshell::{Shell, cmd};

/// Profile the bench binary and open the result.
///
/// macOS  — uses `samply record` (no SIP changes needed); opens Firefox Profiler.
/// Linux  — uses `cargo flamegraph`; writes SVG to bench/profiles/.
///
/// Options (passed as extra args):
///   --suite <name>   bench suite to run (default: codec)
pub fn flamegraph(sh: &Shell, extra_args: &[String]) -> Result<()> {
    std::fs::create_dir_all("bench/profiles").context("create bench/profiles")?;

    let suite = extra_args
        .windows(2)
        .find(|w| w[0] == "--suite")
        .map(|w| w[1].as_str())
        .unwrap_or("codec");

    cmd!(sh, "cargo build --release -p minibox-bench")
        .run()
        .context("build minibox-bench")?;

    let bin = sh.current_dir().join("target/release/minibox-bench");
    let bin_path = bin.to_string_lossy().to_string();

    if cfg!(target_os = "macos") {
        which("samply").context("samply not found — install with: cargo install samply")?;

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let profile_path = format!("bench/profiles/samply-{suite}-{ts}.nsp");

        eprintln!("profiling with samply (suite={suite}) → {profile_path}");
        cmd!(
            sh,
            "samply record --save-only -o {profile_path} {bin_path} --suite {suite}"
        )
        .run()
        .context("samply record failed")?;

        eprintln!("saved: {profile_path}");
        eprintln!("opening in Firefox...");
        let _env = sh.push_env("BROWSER", "firefox");
        cmd!(sh, "samply load {profile_path}")
            .run()
            .context("samply load failed")?;
    } else {
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
pub fn which(name: &str) -> Result<()> {
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
