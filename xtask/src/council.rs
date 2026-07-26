use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

use crate::dotenv;
use crate::xconfig::XConfig;

pub fn run(root: &Path, base: &str, mode: &str, no_synthesis: bool, prod: bool) -> Result<()> {
    let cfg = XConfig::load(root)?;

    let agentbox_bin = root.join(&cfg.binaries.agentbox_dir);

    if !agentbox_bin.exists() {
        eprintln!("building agentbox...");
        let status = Command::new("go")
            .args([
                "build",
                "-C",
                "agentbox",
                "-o",
                "bin/agentbox",
                &cfg.binaries.agentbox_pkg,
            ])
            .current_dir(root)
            .status()
            .context("failed to run go build")?;
        if !status.success() {
            bail!("go build failed with {status}");
        }
    }

    let dotenv_key = dotenv::op_read(&std::env::var("DOTENV_KEY_OP_REF").context(
        "DOTENV_KEY_OP_REF env var not set — set it to the op:// ref for the dotenvx private key",
    )?)?;

    let openai_model = if prod {
        &cfg.models.prod
    } else {
        &cfg.models.default
    };
    let env_file_path = cfg.dotenv.resolved_env_file();
    let env_file_flag = format!("--env-file={env_file_path}");

    let agentbox_str = agentbox_bin.to_string_lossy();
    let mut args = vec![
        "run",
        &env_file_flag,
        "--",
        &agentbox_str,
        "council",
        "--base",
        base,
        "--mode",
        mode,
    ];
    if no_synthesis {
        args.push("--no-synthesis");
    }

    let status = Command::new("dotenvx")
        .args(&args)
        .env("OPENAI_MODEL", openai_model)
        .env("DOTENV_PRIVATE_KEY", &dotenv_key)
        .current_dir(root)
        .status()
        .context("failed to run dotenvx")?;

    if !status.success() {
        bail!("council exited with {status}");
    }
    Ok(())
}
