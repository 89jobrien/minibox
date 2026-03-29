//! Pull an OCI image from Docker Hub without the daemon.
//!
//! Works on any platform (macOS, Linux, Windows). Stores images in
//! `~/.mbx/cache/images` by default, or `$MINIBOX_DATA_DIR/images`.
//!
//! Usage:
//!   cargo run --release --example pull -p mbx
//!   cargo run --release --example pull -p mbx -- nginx:1.25
//!   cargo run --release --example pull -p mbx -- ubuntu:22.04

use mbx::{adapters::DockerHubRegistry, domain::ImageRegistry, image::ImageStore};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

// ── ANSI helpers ──────────────────────────────────────────────────────────────

const C: &str = "\x1b[36m"; // cyan
const B: &str = "\x1b[1m"; // bold
const G: &str = "\x1b[32m"; // green
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

// ── Spinner ───────────────────────────────────────────────────────────────────

struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Spinner {
    fn start(msg: &str) -> Self {
        let msg = msg.to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);

        let handle = std::thread::spawn(move || {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut i = 0;
            while !stop2.load(Ordering::Relaxed) {
                print!("\r  {C}{}{R}  {msg}", frames[i]);
                std::io::stdout().flush().ok();
                i = (i + 1) % frames.len();
                std::thread::sleep(Duration::from_millis(80));
            }
            print!("\r{:<50}\r", ""); // clear line
            std::io::stdout().flush().ok();
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn finish(mut self, done_msg: &str) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            h.join().ok();
        }
        ok(done_msg);
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

// ── Image ref parsing ─────────────────────────────────────────────────────────

fn parse_image_ref(arg: &str) -> (String, String) {
    let (name, tag) = arg.split_once(':').unwrap_or((arg, "latest"));
    let name = if name.contains('/') {
        name.to_string()
    } else {
        format!("library/{name}")
    };
    (name, tag.to_string())
}

// ── Storage path ──────────────────────────────────────────────────────────────

fn images_dir() -> PathBuf {
    let base = if let Ok(d) = std::env::var("MINIBOX_DATA_DIR") {
        PathBuf::from(d)
    } else {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".mbx/cache"))
            .unwrap_or_else(|_| PathBuf::from("/tmp/minibox-demo"))
    };
    base.join("images")
}

// ── Dir size ──────────────────────────────────────────────────────────────────

fn dir_size(path: &std::path::Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

fn human_size(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "alpine".into());
    let (name, tag) = parse_image_ref(&arg);
    let images_dir = images_dir();

    header("minibox · image pull");

    println!("  {B}{BL}▸{R}  {B}image{R}   {name}:{tag}");
    println!("  {D}         store   {}{R}", images_dir.display());
    println!();

    let store = Arc::new(ImageStore::new(&images_dir)?);
    let registry = DockerHubRegistry::new(store.clone())?;
    let image_ref = mbx::ImageRef::parse(&format!("{name}:{tag}"))?;

    if registry.has_image(&name, &tag).await {
        ok("already cached — skipping pull");
    } else {
        step("fetching manifest + layers");
        let spinner = Spinner::start("pulling from Docker Hub...");
        registry.pull_image(&image_ref).await?;
        spinner.finish("layers extracted");
    }

    println!();

    let layers = store.get_image_layers(&name, &tag)?;
    let total: u64 = layers.iter().map(|p| dir_size(p)).sum();

    println!(
        "  {B}layers{R}  {D}({} layer{}, {} on disk){R}",
        layers.len(),
        if layers.len() == 1 { "" } else { "s" },
        human_size(total)
    );
    for (i, path) in layers.iter().enumerate() {
        let size = dir_size(path);
        let digest = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        let short = &digest[digest.len().saturating_sub(16)..];
        println!("  {D}  [{i}] …{short}  {}{R}", human_size(size));
    }
    println!();
    ok("done");
    println!();

    Ok(())
}
