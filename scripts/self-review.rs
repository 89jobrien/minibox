#!/usr/bin/env rust-script
//! Archive the latest session reflection self-review.
//!
//! Usage:
//!   ./scripts/self-review.rs [repo]
//!   ./scripts/self-review.rs --reflect .ctx/reflect-2026-05-25.md
//!
//! ```cargo
//! [dependencies]
//! anyhow = "1"
//! chrono = "0.4"
//! clap = { version = "4", features = ["derive"] }
//! ```

use anyhow::{bail, Context, Result};
use chrono::Local;
use clap::Parser;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

const PLACEHOLDER: &str = "(fill in next session)";
const PATTERNS_HEADING: &str = "## Patterns & Surprises";
const ARCHIVE_DIR: &str = ".ctx/logs/self-review";
const ACTIVE_CONTEXT: &str = ".ctx/memory-bank/activeContext.mbx.md";
const PROGRESS_CONTEXT: &str = ".ctx/memory-bank/progress.mbx.md";

#[derive(Parser)]
#[command(about = "Fill and archive the latest .ctx/reflect-*.md self-review")]
struct Args {
    /// Repository root. Defaults to the current directory.
    #[arg(default_value = ".")]
    repo: PathBuf,

    /// Specific reflect file to process instead of the latest .ctx/reflect-*.md.
    #[arg(long)]
    reflect: Option<PathBuf>,

    /// Print what would change without writing files.
    #[arg(long)]
    dry_run: bool,

    /// Run built-in parser and archiving checks.
    #[arg(long)]
    self_test: bool,
}

#[derive(Debug)]
struct ReflectUpdate {
    content: String,
    replacements: usize,
}

#[derive(Debug, Default)]
struct Evidence {
    shipped: Vec<String>,
    unfinished: Vec<String>,
    files_summary: Option<DiffSummary>,
}

#[derive(Debug, Clone, Copy)]
struct DiffSummary {
    files: usize,
    insertions: usize,
    deletions: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.self_test {
        run_self_tests()?;
        println!("self-review: self-test passed");
        return Ok(());
    }

    let repo = args.repo.canonicalize().unwrap_or(args.repo);
    let loaded = load_or_create_reflect(&repo, args.reflect.as_deref(), args.dry_run)?;
    let reflect_path = loaded.path;
    let original = loaded.content;
    let update = fill_placeholders(&original)?;

    if update.replacements > 0 && !args.dry_run {
        fs::write(&reflect_path, &update.content)
            .with_context(|| format!("write {}", reflect_path.display()))?;
    }

    let archive_path = archive_path(&repo, &reflect_path)?;
    if !args.dry_run {
        if let Some(parent) = archive_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&archive_path, &update.content)
            .with_context(|| format!("write {}", archive_path.display()))?;
    }

    let relative_archive = path_relative_to(&repo, &archive_path);
    if args.dry_run {
        println!("self-review: would write {}", relative_archive.display());
    } else {
        println!("self-review: wrote {}", relative_archive.display());
    }

    if update.replacements > 0 {
        println!(
            "self-review: filled {} placeholder section(s)",
            update.replacements
        );
    }
    if loaded.created {
        let relative_reflect = path_relative_to(&repo, &reflect_path);
        if args.dry_run {
            println!("self-review: would create {}", relative_reflect.display());
        } else {
            println!("self-review: created {}", relative_reflect.display());
        }
    }

    println!();
    println!("{}", patterns_section(&update.content)?.trim_end());
    Ok(())
}

#[derive(Debug)]
struct LoadedReflect {
    path: PathBuf,
    content: String,
    created: bool,
}

fn load_or_create_reflect(
    repo: &Path,
    reflect: Option<&Path>,
    dry_run: bool,
) -> Result<LoadedReflect> {
    let path = match reflect {
        Some(path) => resolve_reflect_arg(repo, path),
        None => {
            find_latest_reflect(&repo.join(".ctx"))?.unwrap_or_else(|| default_reflect_path(repo))
        }
    };

    if path.exists() {
        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        return Ok(LoadedReflect {
            path,
            content,
            created: false,
        });
    }

    let content = build_new_reflect(repo);
    if !dry_run {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&path, &content).with_context(|| format!("write {}", path.display()))?;
    }

    Ok(LoadedReflect {
        path,
        content,
        created: true,
    })
}

fn resolve_reflect_arg(repo: &Path, reflect: &Path) -> PathBuf {
    if reflect.is_absolute() {
        reflect.to_path_buf()
    } else {
        repo.join(reflect)
    }
}
fn find_latest_reflect(ctx_dir: &Path) -> Result<Option<PathBuf>> {
    let entries = match fs::read_dir(ctx_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", ctx_dir.display())),
    };
    let mut reflect_files = Vec::new();

    for entry in entries {
        let entry = entry.with_context(|| format!("read entry in {}", ctx_dir.display()))?;
        let path = entry.path();
        if !is_reflect_file(&path) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        reflect_files.push((modified, path));
    }

    reflect_files.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    Ok(reflect_files.pop().map(|(_, path)| path))
}

fn default_reflect_path(repo: &Path) -> PathBuf {
    let date = Local::now().format("%Y-%m-%d");
    repo.join(".ctx").join(format!("reflect-{date}.md"))
}

fn build_new_reflect(repo: &Path) -> String {
    let now = Local::now().format("%Y-%m-%d %H:%M");
    let seed = MemorySeed::from_repo(repo);
    let shipped = markdown_bullets(&seed.shipped, "Nothing recorded yet.");
    let unfinished = markdown_bullets(&seed.unfinished, "Nothing recorded yet.");

    format!(
        r#"# Session Reflection — {now}

## Shipped

{shipped}

## Unfinished

{unfinished}

## Memory-bank source

- {source}

## Patterns & Surprises

### Took longer than expected

- Nothing notable this session.

### Went smoothly

- Nothing notable this session.

### Discovered mid-session

- Nothing notable this session.

### Next session speedups

- Nothing notable this session.

## Open questions

- Nothing recorded yet.
"#,
        source = seed.source
    )
}

#[derive(Debug)]
struct MemorySeed {
    shipped: Vec<String>,
    unfinished: Vec<String>,
    source: String,
}

impl MemorySeed {
    fn from_repo(repo: &Path) -> Self {
        let active_path = repo.join(ACTIVE_CONTEXT);
        if let Ok(content) = fs::read_to_string(&active_path) {
            let shipped = bullets_after_marker(&content, "**Recently completed:**", 5);
            let unfinished = bullets_after_marker(&content, "**In progress:**", 5);
            if !shipped.is_empty() || !unfinished.is_empty() {
                return Self {
                    shipped,
                    unfinished,
                    source: format!(
                        "Seeded from `{ACTIVE_CONTEXT}` because no reflect file existed."
                    ),
                };
            }
        }

        let progress_path = repo.join(PROGRESS_CONTEXT);
        if let Ok(content) = fs::read_to_string(&progress_path) {
            let shipped = bullets_after_marker(&content, "## Recently completed", 5);
            let unfinished = bullets_after_marker(&content, "## In progress", 5);
            if !shipped.is_empty() || !unfinished.is_empty() {
                return Self {
                    shipped,
                    unfinished,
                    source: format!(
                        "Seeded from `{PROGRESS_CONTEXT}` because no reflect file existed."
                    ),
                };
            }
        }

        Self {
            shipped: Vec::new(),
            unfinished: Vec::new(),
            source: "No memory-bank seed was available when this reflect file was created."
                .to_owned(),
        }
    }
}

fn bullets_after_marker(content: &str, marker: &str, max: usize) -> Vec<String> {
    let mut in_section = false;
    let mut bullets = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == marker {
            in_section = true;
            continue;
        }

        if in_section && is_section_boundary(trimmed) {
            break;
        }

        if in_section {
            if let Some(bullet) = normalize_memory_bullet(trimmed) {
                bullets.push(bullet);
            }
        }

        if bullets.len() >= max {
            break;
        }
    }

    bullets
}

fn is_section_boundary(line: &str) -> bool {
    line.starts_with("## ")
        || (line.starts_with("**") && line.ends_with(":**"))
        || line == "**Decisions (recent):**"
        || line == "**Open questions:**"
}

fn normalize_memory_bullet(line: &str) -> Option<String> {
    let cleaned = line
        .strip_prefix("- [x] ")
        .or_else(|| line.strip_prefix("- [ ] "))
        .or_else(|| line.strip_prefix("- "))
        .map(str::trim)?;

    (!cleaned.is_empty()).then(|| cleaned.to_owned())
}

fn markdown_bullets(items: &[String], fallback: &str) -> String {
    if items.is_empty() {
        return format!("- {fallback}");
    }

    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_reflect_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    name.starts_with("reflect-") && name.ends_with(".md")
}

fn archive_path(repo: &Path, reflect_path: &Path) -> Result<PathBuf> {
    let Some(file_name) = reflect_path.file_name() else {
        bail!("reflect path has no filename: {}", reflect_path.display());
    };
    Ok(repo.join(ARCHIVE_DIR).join(file_name))
}

fn fill_placeholders(content: &str) -> Result<ReflectUpdate> {
    let evidence = Evidence::from_content(content);
    let replacements = [
        (
            "### Took longer than expected",
            bullets_took_longer(&evidence),
        ),
        ("### Went smoothly", bullets_went_smoothly(&evidence)),
        ("### Discovered mid-session", bullets_discovered(&evidence)),
        (
            "### Next session speedups",
            bullets_next_speedups(&evidence),
        ),
    ];

    let mut lines = content
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    let mut count = 0;

    for (heading, bullets) in replacements {
        if replace_placeholder_body(&mut lines, heading, &bullets)? {
            count += 1;
        }
    }

    let mut updated = lines.join("\n");
    if content.ends_with('\n') {
        updated.push('\n');
    }

    Ok(ReflectUpdate {
        content: updated,
        replacements: count,
    })
}

fn replace_placeholder_body(
    lines: &mut Vec<String>,
    heading: &str,
    bullets: &[String],
) -> Result<bool> {
    let Some(heading_index) = lines.iter().position(|line| line.trim() == heading) else {
        bail!("missing subsection: {heading}");
    };

    let body_start = heading_index + 1;
    let body_end = next_heading_index(lines, body_start).unwrap_or(lines.len());
    let has_placeholder = lines[body_start..body_end]
        .iter()
        .any(|line| line.contains(PLACEHOLDER));

    if !has_placeholder {
        return Ok(false);
    }

    let mut replacement = Vec::with_capacity(bullets.len() + 2);
    replacement.push(String::new());
    replacement.extend(bullets.iter().map(|bullet| format!("- {bullet}")));
    replacement.push(String::new());

    lines.splice(body_start..body_end, replacement);
    Ok(true)
}

fn next_heading_index(lines: &[String], start: usize) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, line)| {
            let trimmed = line.trim_start();
            trimmed.starts_with("### ") || trimmed.starts_with("## ")
        })
        .map(|(index, _)| index)
}

impl Evidence {
    fn from_content(content: &str) -> Self {
        Self {
            shipped: section_lines(content, "## Shipped"),
            unfinished: section_lines(content, "## Unfinished"),
            files_summary: parse_diff_summary(content),
        }
    }

    fn commit_lines(&self) -> Vec<&str> {
        self.shipped
            .iter()
            .map(String::as_str)
            .filter(|line| starts_with_short_sha(line.trim_start_matches("- ")))
            .collect()
    }

    fn example_commit(&self, needle: &str) -> Option<String> {
        self.commit_lines()
            .into_iter()
            .find(|line| line.contains(needle))
            .map(shorten_commit_line)
    }
}

fn section_lines(content: &str, heading: &str) -> Vec<String> {
    let Some(section) = section(content, heading) else {
        return Vec::new();
    };
    section
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .collect()
}

fn section<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    let start = content.find(heading)?;
    let after_heading = start + heading.len();
    let rest = &content[after_heading..];
    let end = rest
        .find("\n## ")
        .map(|offset| after_heading + offset)
        .unwrap_or(content.len());
    Some(content[after_heading..end].trim())
}

fn patterns_section(content: &str) -> Result<&str> {
    section_with_heading(content, PATTERNS_HEADING)
        .with_context(|| format!("missing {PATTERNS_HEADING} section"))
}

fn section_with_heading<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    let start = content.find(heading)?;
    let rest = &content[start..];
    let end = rest
        .find("\n## ")
        .filter(|offset| *offset != 0)
        .map(|offset| start + offset)
        .unwrap_or(content.len());
    Some(content[start..end].trim())
}

fn parse_diff_summary(content: &str) -> Option<DiffSummary> {
    content.lines().find_map(|line| {
        if !(line.contains("files changed") || line.contains("file changed")) {
            return None;
        }

        let numbers = line
            .split(|ch: char| !ch.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<usize>().ok())
            .collect::<Vec<_>>();

        if numbers.is_empty() {
            return None;
        }

        Some(DiffSummary {
            files: numbers.first().copied().unwrap_or(0),
            insertions: numbers.get(1).copied().unwrap_or(0),
            deletions: numbers.get(2).copied().unwrap_or(0),
        })
    })
}

fn bullets_took_longer(evidence: &Evidence) -> Vec<String> {
    let mut bullets = Vec::new();
    let fix_ci = evidence
        .commit_lines()
        .into_iter()
        .filter(|line| line.contains("fix(ci)") || line.contains("cargo fmt"))
        .map(shorten_commit_line)
        .collect::<Vec<_>>();

    if fix_ci.len() >= 2 {
        bullets.push(format!(
            "Repeated CI/fmt follow-up commits ({}) suggest gate cleanup took extra cycles.",
            join_examples(&fix_ci, 3)
        ));
    }

    if let Some(summary) = evidence.files_summary {
        if summary.files >= 50 || summary.insertions + summary.deletions >= 5_000 {
            bullets.push(format!(
                "Large diff stat ({} files, +{}, -{}) likely made review and reflection slower.",
                summary.files, summary.insertions, summary.deletions
            ));
        }
    }

    if bullets.is_empty() {
        bullets.push("Nothing notable this session.".to_owned());
    }
    bullets
}

fn bullets_went_smoothly(evidence: &Evidence) -> Vec<String> {
    let mut bullets = Vec::new();
    let merges = evidence
        .commit_lines()
        .into_iter()
        .filter(|line| line.contains("merge: issue"))
        .map(shorten_commit_line)
        .collect::<Vec<_>>();

    if !merges.is_empty() {
        bullets.push(format!(
            "Issue-oriented merge commits landed in a traceable sequence ({})",
            join_examples(&merges, 3)
        ));
    }

    if let Some(example) = evidence.example_commit("test:") {
        bullets.push(format!(
            "Test coverage work is explicit in the shipped list ({example})."
        ));
    }

    if let Some(example) = evidence.example_commit("docs:") {
        bullets.push(format!(
            "Documentation follow-through was captured alongside code changes ({example})."
        ));
    }

    if bullets.is_empty() && !evidence.shipped.is_empty() {
        bullets.push(format!(
            "Shipped section has {} recorded item(s) and no blocker is recorded in this section.",
            evidence.shipped.len()
        ));
    }

    if bullets.is_empty() {
        bullets.push("Nothing notable this session.".to_owned());
    }
    bullets.truncate(4);
    bullets
}

fn bullets_discovered(evidence: &Evidence) -> Vec<String> {
    let mut bullets = Vec::new();

    if let Some(example) = evidence.example_commit("fix(ci)") {
        bullets.push(format!(
            "CI drift surfaced during the session rather than ahead of time ({example})."
        ));
    }

    if let Some(example) = evidence.example_commit("docs:") {
        bullets.push(format!(
            "Docs needed alignment with implementation state ({example})."
        ));
    }

    if let Some(unfinished) = evidence.unfinished.first() {
        bullets.push(format!(
            "Unfinished work remained visible at handoff: {}",
            clean_bullet(unfinished)
        ));
    }

    if bullets.is_empty() {
        bullets.push("Nothing notable this session.".to_owned());
    }
    bullets.truncate(4);
    bullets
}

fn bullets_next_speedups(evidence: &Evidence) -> Vec<String> {
    let mut bullets = Vec::new();

    if evidence
        .commit_lines()
        .into_iter()
        .any(|line| line.contains("fix(ci)") || line.contains("cargo fmt"))
    {
        bullets.push(
            "Run the same fmt/clippy/CI gate locally before archiving to catch drift earlier."
                .to_owned(),
        );
    }

    if let Some(summary) = evidence.files_summary {
        if summary.files >= 50 {
            bullets.push(format!(
                "Split future reflection windows before they reach {} changed files.",
                summary.files
            ));
        }
    }

    if let Some(unfinished) = evidence.unfinished.first() {
        bullets.push(format!(
            "Start with the first unfinished item: {}",
            clean_bullet(unfinished)
        ));
    }

    if bullets.is_empty() {
        bullets.push("Nothing notable this session.".to_owned());
    }
    bullets.truncate(4);
    bullets
}

fn starts_with_short_sha(line: &str) -> bool {
    let Some(first) = line.split_whitespace().next() else {
        return false;
    };
    first.len() >= 7 && first.chars().take(7).all(|ch| ch.is_ascii_hexdigit())
}

fn shorten_commit_line(line: &str) -> String {
    let cleaned = clean_bullet(line);
    let mut parts = cleaned.splitn(2, ' ');
    let sha = parts.next().unwrap_or_default();
    let subject = parts.next().unwrap_or_default();
    if subject.is_empty() {
        sha.to_owned()
    } else {
        format!("`{sha}` {subject}")
    }
}

fn clean_bullet(line: &str) -> String {
    line.trim()
        .trim_start_matches("- ")
        .trim_start_matches("* ")
        .trim()
        .to_owned()
}

fn join_examples(examples: &[String], max: usize) -> String {
    examples
        .iter()
        .take(max)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

fn path_relative_to(base: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(base).unwrap_or(path).to_path_buf()
}

fn run_self_tests() -> Result<()> {
    fill_keeps_open_questions_placeholder()?;
    archive_writes_same_filename()?;
    already_filled_is_idempotent()?;
    missing_reflect_is_created_from_memory_bank()?;
    Ok(())
}

fn fill_keeps_open_questions_placeholder() -> Result<()> {
    let input = r#"# Session Reflection — test

## Shipped

- 31da360 fix(ci): handle multiple gates
- 67fb3d0 fix(ci): cargo fmt on tests
- fc514b3 docs: add mutation audit checklist

## Files changed

```
3 files changed, 20 insertions(+), 5 deletions(-)
```

## Unfinished

- Audit old stashes

## Patterns & Surprises

### Took longer than expected

- (fill in next session)

### Went smoothly

- (fill in next session)

### Discovered mid-session

- (fill in next session)

### Next session speedups

- (fill in next session)

## Open questions

- (fill in next session)
"#;

    let update = fill_placeholders(input)?;
    if update.replacements != 4 {
        bail!("expected 4 replacements, got {}", update.replacements);
    }
    let patterns = patterns_section(&update.content)?;
    if patterns.contains(PLACEHOLDER) {
        bail!("patterns section still contains placeholder");
    }
    let open_questions =
        section(&update.content, "## Open questions").context("missing open questions section")?;
    if !open_questions.contains(PLACEHOLDER) {
        bail!("open questions placeholder should remain untouched");
    }
    Ok(())
}

fn missing_reflect_is_created_from_memory_bank() -> Result<()> {
    let root = std::env::temp_dir().join(format!(
        "minibox-self-review-memory-test-{}",
        std::process::id()
    ));
    let memory_bank = root.join(".ctx/memory-bank");
    fs::create_dir_all(&memory_bank)?;
    fs::write(
        memory_bank.join("activeContext.mbx.md"),
        r#"# Active context

**In progress:**

- [ ] Finish self-review archiver

**Recently completed:**

- [x] Added rust-script self-review helper

**Open questions:**

- None
"#,
    )?;

    let loaded = load_or_create_reflect(&root, None, false)?;
    if !loaded.created {
        bail!("missing reflect file should be created");
    }
    if !loaded.path.exists() {
        bail!("created reflect file does not exist");
    }
    if !loaded
        .content
        .contains("Added rust-script self-review helper")
    {
        bail!("created reflect did not seed shipped work from memory-bank");
    }
    if !loaded.content.contains("Finish self-review archiver") {
        bail!("created reflect did not seed unfinished work from memory-bank");
    }

    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn archive_writes_same_filename() -> Result<()> {
    let root =
        std::env::temp_dir().join(format!("minibox-self-review-test-{}", std::process::id()));
    let ctx = root.join(".ctx");
    let reflect = ctx.join("reflect-2099-01-01.md");
    fs::create_dir_all(&ctx)?;
    fs::write(
        &reflect,
        "# Session Reflection\n\n## Patterns & Surprises\n\n### Took longer than expected\n\n- Nothing notable this session.\n\n### Went smoothly\n\n- Nothing notable this session.\n\n### Discovered mid-session\n\n- Nothing notable this session.\n\n### Next session speedups\n\n- Nothing notable this session.\n",
    )?;

    let archive = archive_path(&root, &reflect)?;
    let file_name = archive
        .file_name()
        .and_then(OsStr::to_str)
        .context("archive filename missing")?;
    if file_name != "reflect-2099-01-01.md" {
        bail!("archive did not keep same filename: {file_name}");
    }

    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn already_filled_is_idempotent() -> Result<()> {
    let input = r#"# Session Reflection — test

## Patterns & Surprises

### Took longer than expected

- Nothing notable this session.

### Went smoothly

- Already filled.

### Discovered mid-session

- Already filled.

### Next session speedups

- Already filled.
"#;
    let update = fill_placeholders(input)?;
    if update.replacements != 0 {
        bail!(
            "expected idempotent update, got {} replacements",
            update.replacements
        );
    }
    if update.content != input {
        bail!("idempotent update changed content");
    }
    Ok(())
}
