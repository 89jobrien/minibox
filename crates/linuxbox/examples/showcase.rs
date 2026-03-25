//! Image showcase: pull many images, run a probe in each container, show metrics.
//!
//! Pulls work on any platform. Container runs require miniboxd running (Linux).
//!
//! Usage:
//!   cargo run --release --example showcase -p linuxbox
//!   cargo run --release --example showcase -p linuxbox -- --run /path/to/minibox
//!   cargo run --release --example showcase -p linuxbox -- --run ./target/release/minibox --cleanup

use linuxbox::{
    adapters::DockerHubRegistry,
    domain::ImageRegistry,
    image::{ImageStore, reference::ImageRef},
};
use std::{
    io::Write,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

// ── ANSI ─────────────────────────────────────────────────────────────────────

const C: &str = "\x1b[36m"; // cyan
const B: &str = "\x1b[1m"; // bold
const G: &str = "\x1b[32m"; // green
const Y: &str = "\x1b[33m"; // yellow
const BL: &str = "\x1b[34m"; // blue
const D: &str = "\x1b[2m"; // dim
const R: &str = "\x1b[0m"; // reset

fn header(title: &str) {
    println!();
    println!("  {B}{C}╭─────────────────────────────────────────╮{R}");
    println!("  {B}{C}│  {title:<39}│{R}");
    println!("  {B}{C}╰─────────────────────────────────────────╯{R}");
    println!();
}

fn step(msg: &str) {
    println!("  {B}{BL}▸{R}  {B}{msg}{R}");
}

fn ok(msg: &str) {
    println!("  {G}✓{R}  {msg}");
}

fn fmt_duration(d: Duration) -> String {
    format!("{:.2}s", d.as_secs_f64())
}

fn human_size(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

fn dir_size(path: &std::path::Path) -> u64 {
    walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && !e.path_is_symlink())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

// ── Spinner ───────────────────────────────────────────────────────────────────

struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Spinner {
    fn start(label: &str) -> Self {
        let label = label.to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut i = 0;
            while !stop2.load(Ordering::Relaxed) {
                print!("\r    {C}{}{R}  {D}{label}{R}", frames[i]);
                std::io::stdout().flush().ok();
                i = (i + 1) % frames.len();
                std::thread::sleep(Duration::from_millis(80));
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            h.join().ok();
        }
        print!("\r{:<60}\r", "");
        std::io::stdout().flush().ok();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            h.join().ok();
        }
    }
}

// ── Image catalogue ───────────────────────────────────────────────────────────

struct Spec {
    name: &'static str, // "library/alpine"
    tag: &'static str,
    cmd: &'static [&'static str], // argv passed to container
}

const IMAGES: &[Spec] = &[
    Spec {
        name: "library/alpine",
        tag: "latest",
        cmd: &["/bin/sh", "-c", "echo Alpine $(cat /etc/alpine-release)"],
    },
    Spec {
        name: "library/alpine",
        tag: "3.19",
        cmd: &["/bin/sh", "-c", "echo Alpine $(cat /etc/alpine-release)"],
    },
    Spec {
        name: "library/alpine",
        tag: "3.18",
        cmd: &["/bin/sh", "-c", "echo Alpine $(cat /etc/alpine-release)"],
    },
    Spec {
        name: "library/alpine",
        tag: "3.17",
        cmd: &["/bin/sh", "-c", "echo Alpine $(cat /etc/alpine-release)"],
    },
    Spec {
        name: "library/alpine",
        tag: "3.16",
        cmd: &["/bin/sh", "-c", "echo Alpine $(cat /etc/alpine-release)"],
    },
    Spec {
        name: "library/alpine",
        tag: "3.15",
        cmd: &["/bin/sh", "-c", "echo Alpine $(cat /etc/alpine-release)"],
    },
    Spec {
        name: "library/alpine",
        tag: "3.14",
        cmd: &["/bin/sh", "-c", "echo Alpine $(cat /etc/alpine-release)"],
    },
    Spec {
        name: "library/busybox",
        tag: "latest",
        cmd: &["/bin/sh", "-c", "busybox | head -1"],
    },
    Spec {
        name: "library/busybox",
        tag: "1.35",
        cmd: &["/bin/sh", "-c", "busybox | head -1"],
    },
    Spec {
        name: "library/busybox",
        tag: "1.36",
        cmd: &["/bin/sh", "-c", "busybox | head -1"],
    },
    Spec {
        name: "library/debian",
        tag: "bookworm-slim",
        cmd: &["/bin/sh", "-c", "cat /etc/debian_version"],
    },
    Spec {
        name: "library/debian",
        tag: "bullseye-slim",
        cmd: &["/bin/sh", "-c", "cat /etc/debian_version"],
    },
    Spec {
        name: "library/nginx",
        tag: "alpine",
        cmd: &["/bin/sh", "-c", "nginx -v 2>&1"],
    },
    Spec {
        name: "library/redis",
        tag: "alpine",
        cmd: &["/bin/sh", "-c", "redis-server --version"],
    },
    Spec {
        name: "library/python",
        tag: "alpine",
        cmd: &["/bin/sh", "-c", "python3 --version"],
    },
];

// ── Result ────────────────────────────────────────────────────────────────────

struct Row {
    label: String,
    cached: bool,
    pull_ms: Duration,
    run_ms: Option<Duration>,
    output: String,
    layers: usize,
    size: u64,
}

// ── Args ──────────────────────────────────────────────────────────────────────

struct Args {
    cli_bin: Option<PathBuf>, // path to minibox CLI for running containers
    sudo_run: bool,           // prepend sudo when invoking the CLI
    cleanup: bool,
}

fn parse_args() -> Args {
    let mut args = std::env::args().skip(1);
    let mut cli_bin = None;
    let mut sudo_run = false;
    let mut cleanup = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run" => cli_bin = args.next().map(PathBuf::from),
            "--sudo-run" => sudo_run = true,
            "--cleanup" => cleanup = true,
            _ => {}
        }
    }
    Args {
        cli_bin,
        sudo_run,
        cleanup,
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args();

    let images_dir = {
        let base = if let Ok(d) = std::env::var("MINIBOX_DATA_DIR") {
            PathBuf::from(d)
        } else {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".mbx/cache"))
                .unwrap_or_else(|_| PathBuf::from("/tmp/minibox-demo"))
        };
        base.join("images")
    };

    header("minibox · image showcase");

    let mode = if args.cli_bin.is_some() {
        "pull + run"
    } else {
        "pull only"
    };
    println!("  {D}images  {}{R}", IMAGES.len());
    println!("  {D}mode    {mode}{R}");
    println!("  {D}store   {}{R}", images_dir.display());
    println!();

    let store = Arc::new(ImageStore::new(&images_dir)?);
    let registry = DockerHubRegistry::new(store.clone())?;

    // ── Pull phase ────────────────────────────────────────────────────────────

    step(&format!("pulling {} images", IMAGES.len()));
    println!();

    let mut rows: Vec<Row> = Vec::new();

    for spec in IMAGES {
        let short = spec.name.trim_start_matches("library/");
        let label = format!("{short}:{}", spec.tag);
        let pad_label = format!("{label:<28}");

        let cached = registry.has_image(spec.name, spec.tag).await;

        let spin = Spinner::start(&format!("{pad_label} pulling..."));
        let t0 = Instant::now();
        if !cached {
            let image_ref = ImageRef::parse(&format!("{}:{}", spec.name, spec.tag))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            registry.pull_image(&image_ref).await?;
        }
        let pull_ms = t0.elapsed();
        spin.stop();

        let layers = store.get_image_layers(spec.name, spec.tag)?;
        let size: u64 = layers.iter().map(|p| dir_size(p)).sum();
        let n_layers = layers.len();

        let status = if cached {
            format!("{D}cached{R}")
        } else {
            format!("{G}{}{R}", fmt_duration(pull_ms))
        };
        let size_str = human_size(size);

        println!("    {B}{pad_label}{R}  {status:<28}  {D}{n_layers}L  {size_str}{R}");

        rows.push(Row {
            label: label.clone(),
            cached,
            pull_ms,
            run_ms: None,
            output: String::new(),
            layers: n_layers,
            size,
        });
    }

    // ── Run phase ─────────────────────────────────────────────────────────────

    let mut daemon_unavailable = false;

    if let Some(ref cli) = args.cli_bin {
        println!();
        step(&format!("running {} containers", IMAGES.len()));
        println!();

        for (row, spec) in rows.iter_mut().zip(IMAGES.iter()) {
            let short = spec.name.trim_start_matches("library/");
            let image_ref = format!("{short}:{}", spec.tag);
            let pad_label = format!("{image_ref:<28}");

            if daemon_unavailable {
                row.output = "—".to_string();
                continue;
            }

            let spin = Spinner::start(&format!("{pad_label} running..."));
            let t0 = Instant::now();

            let mut cmd_args = vec!["run".to_string(), image_ref.clone(), "--".to_string()];
            cmd_args.extend(spec.cmd.iter().map(|s| s.to_string()));

            let out = if args.sudo_run {
                Command::new("sudo").arg(cli).args(&cmd_args).output()
            } else {
                Command::new(cli).args(&cmd_args).output()
            };

            let elapsed = t0.elapsed();
            spin.stop();

            let output = match out {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    if !stdout.is_empty() { stdout } else { stderr }
                }
                Err(e) => format!("error: {e}"),
            };

            // Detect daemon connection failure on first attempt and bail out cleanly
            if output.contains("connecting to daemon")
                || output.contains("No such file")
                || output.contains("Connection refused")
            {
                daemon_unavailable = true;
                println!("    {D}daemon not available — skipping container runs{R}");
                println!("    {D}start miniboxd and re-run with --run to see container output{R}");
                row.output = "—".to_string();
                continue;
            }

            let output_short = if output.len() > 32 {
                format!("{}…", &output[..31])
            } else {
                output.clone()
            };

            println!(
                "    {B}{pad_label}{R}  {G}{:<12}{R}  {D}{output_short}{R}",
                fmt_duration(elapsed)
            );

            row.run_ms = Some(elapsed);
            row.output = output;
        }
    }

    // ── Metrics table ─────────────────────────────────────────────────────────

    println!();
    step("metrics");
    println!();

    // Header
    let run_col = args.cli_bin.is_some() && !daemon_unavailable;
    if run_col {
        println!(
            "  {B}{D}{:<28}  {:<12}  {:<12}  {:<8}  {:<8}  {}{R}",
            "IMAGE", "PULL", "RUN", "LAYERS", "SIZE", "OUTPUT"
        );
        println!("  {D}{}{R}", "─".repeat(90));
    } else {
        println!(
            "  {B}{D}{:<28}  {:<12}  {:<8}  {}{R}",
            "IMAGE", "PULL", "LAYERS", "SIZE"
        );
        println!("  {D}{}{R}", "─".repeat(62));
    }

    let mut total_size: u64 = 0;
    let mut pulled = 0usize;
    let mut cached = 0usize;
    let mut total_pull = Duration::ZERO;
    let mut total_run = Duration::ZERO;

    for row in &rows {
        total_size += row.size;
        if row.cached {
            cached += 1;
        } else {
            pulled += 1;
            total_pull += row.pull_ms;
        }
        if let Some(r) = row.run_ms {
            total_run += r;
        }

        let pull_str = if row.cached {
            format!("{D}cached{R}")
        } else {
            format!("{Y}{}{R}", fmt_duration(row.pull_ms))
        };

        if run_col {
            let run_str = row
                .run_ms
                .map(|d| format!("{G}{}{R}", fmt_duration(d)))
                .unwrap_or_else(|| format!("{D}—{R}"));
            let out_str = if row.output.len() > 28 {
                format!("{}…", &row.output[..27])
            } else {
                row.output.clone()
            };
            println!(
                "  {:<28}  {pull_str:<20}  {run_str:<20}  {D}{:<8}{R}  {D}{:<8}{R}  {D}{out_str}{R}",
                row.label,
                row.layers,
                human_size(row.size)
            );
        } else {
            println!(
                "  {:<28}  {pull_str:<20}  {D}{:<8}{R}  {D}{}{R}",
                row.label,
                row.layers,
                human_size(row.size)
            );
        }
    }

    // Totals
    println!();
    println!("  {D}images   {}{R}", IMAGES.len());
    println!(
        "  {D}pulled   {} ({} cached)  total pull time {}{R}",
        pulled,
        cached,
        fmt_duration(total_pull)
    );
    if run_col {
        println!(
            "  {D}ran      {} containers  total run time {}{R}",
            rows.len(),
            fmt_duration(total_run)
        );
    }
    println!("  {D}disk     {}{R}", human_size(total_size));

    // ── Cleanup ───────────────────────────────────────────────────────────────

    if args.cleanup {
        println!();
        step("cleaning up image store");
        let mut removed = 0u32;
        for row in &rows {
            let short = row.label.split(':').next().unwrap_or(&row.label);
            let tag = row.label.split(':').nth(1).unwrap_or("latest");
            let name = if short.contains('/') {
                short.to_string()
            } else {
                format!("library/{short}")
            };
            let safe = name.replace('/', "_");
            let dir = images_dir.join(&safe).join(tag);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).ok();
                removed += 1;
            }
        }
        ok(&format!(
            "removed {removed} image directories  ({} freed)",
            human_size(total_size)
        ));
    }

    println!();
    ok("done");
    println!();

    Ok(())
}
