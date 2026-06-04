use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

pub fn run(root: &Path, base: &str, mode: &str, no_synthesis: bool, prod: bool) -> Result<()> {
    let agentbox_bin = root.join("agentbox/bin/agentbox");

    if !agentbox_bin.exists() {
        eprintln!("building agentbox...");
        let status = Command::new("go")
            .args([
                "build",
                "-C",
                "agentbox",
                "-o",
                "bin/agentbox",
                "./cmd/agentbox",
            ])
            .current_dir(root)
            .status()
            .context("failed to run go build")?;
        if !status.success() {
            bail!("go build failed with {status}");
        }
    }

    let dotenv_key = {
        let output = Command::new("op")
            .args([
                "read",
                "--account",
                "my.1password.com",
                "op://byxmw65w7idxsk3i6qbohfiuty/nihl7o2bojy53zy4aqtr7txyqi/password",
            ])
            .output()
            .context("failed to run op read")?;
        if !output.status.success() {
            bail!(
                "op read failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8(output.stdout)
            .context("op read output not UTF-8")?
            .trim()
            .to_string()
    };

    let openai_model = if prod { "gpt-5.3" } else { "gpt-4o" };
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let env_file = format!("--env-file={home}/dev/.env");

    let mut args = vec![
        "run",
        &env_file,
        "--",
        agentbox_bin.to_str().unwrap_or("agentbox"),
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
