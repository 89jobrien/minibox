#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! anyhow = "1"
//! clap = { version = "4", features = ["derive"] }
//! ```
//
//! Extract and ingest missing minibox docs via ollama:gpt-oss
//!
//! Usage: ./scripts/extract_and_ingest.rs [--docs-dir <path>] [--kgx-root <path>]

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(about = "Extract and ingest missing minibox docs via ollama:gpt-oss")]
struct Args {
    /// Path to docs directory
    #[arg(long, default_value = "/Users/joe/dev/minibox/docs/core")]
    docs_dir: PathBuf,

    /// Path to kgx root
    #[arg(long, default_value = "/Users/joe/dev/minibox/.kgx")]
    kgx_root: PathBuf,
}

// Helper: get just the filename from a full path
fn filename_from_path(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

// Helper: check if a doc name is in the ingested list
fn is_ingested(name: &str, ingested: &[&str]) -> bool {
    ingested.iter().any(|ing| ing == &name)
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Already ingested doc names (without .mbx.md extension)
    let ingested = [
        "active-context",
        "AGENTS",
        "ARCHITECTURE",
        "SECURITY_INVARIANTS",
        "TEST_INFRASTRUCTURE",
        "GOTCHAS",
        "STATE_MODEL",
    ];

    // Find all .mbx.md files in the docs directory
    let mut entries: Vec<String> = Vec::new();

    if args.docs_dir.exists() {
        if let Ok(read_dir) = fs::read_dir(&args.docs_dir) {
            for entry in read_dir {
                let entry = entry.context("reading directory entry")?;
                let path = entry.path();

                if path.is_file() {
                    let name = filename_from_path(&path);
                    if name.ends_with(".mbx.md") {
                        entries.push(name);
                    }
                }
            }
        }
        entries.sort();
    } else {
        bail!("Docs directory not found: {:?}", args.docs_dir);
    }

    // Filter to missing docs only
    let missing: Vec<String> = entries
        .iter()
        .filter(|name| !is_ingested(name, &ingested))
        .cloned()
        .collect();

    println!(
        "Found {} missing docs to ingest",
        missing.len()
    );

    // Extraction prompt template
    let extraction_prompt = "You are a knowledge extraction system. Read the provided document and extract entities and relations following these rules:

INGESTION RULES:
- Extract only entities explicitly present in the source text
- Provide supporting_text (quoted from the document) for every entity and relation
- Only add relations with confidence >= 0.6
- Normalize entity types to: security_invariant, code_function, code_path, test_category, data_structure, tool, feature_gate, gotcha, state_type, synchronization, protocol, etc.
- Return valid JSON with structure: {doc_id, title, source, raw_content, entities: [{name, type, supporting_text}], relations: [{source, target, type, confidence, supporting_text}]}

DOCUMENT TO EXTRACT FROM:";

    // Call ollama
    print "  ⏳ Extracting entities via ollama:gpt-oss...\n";

    let ollama_url = "http://localhost:11434/api/generate";

    for doc_name in &missing {
        let doc_path = args.docs_dir.join(doc_name);

        // Read first 2KB of content
        let _raw_content = if doc_path.exists() {
            let mut content = String::new();
            if let Ok(file) = fs::File::open(&doc_path) {
                let reader = BufReader::new(file);
                for line in reader.lines() {
                    if let Ok(l) = line {
                        if content.len() + l.len() + 1 > 2000 {
                            // Trim to ~2KB
                            content.push_str(&l[..std::cmp::min(l.len(), 2000 - content.len() - 1)]);
                            break;
                        }
                        content.push_str(&l);
                        content.push('\n');
                    }
                }
            }
            content
        } else {
            format!("File not found: {:?}", doc_path)
        };

        // Build extraction prompt
        let full_prompt = format!(
            "{}\n\n---\n\n{}",
            extraction_prompt, _raw_content
        );

        // Call ollama via curl
        let output = Command::new("curl")
            .args([
                "-s",
                ollama_url,
                "-d",
                &format!(
                    r#"{{"model":"gpt-oss","prompt":"{}","stream":false}}"#,
                    // Escape single quotes and special chars in prompt
                    full_prompt.replace('"', """)
                ),
            ])
            .output()
            .context("failed to call ollama")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("ollama request failed: {}", stderr.trim());
        }

        let response = String::from_utf8_lossy(&output.stdout);

        // Parse JSON from response (ollama may wrap it in markdown code blocks)
        let json_str = if response.contains("```json") {
            response
                .replace("```json", "")
                .replace("```", "")
                .trim()
                .to_string()
        } else if response.contains("{") {
            // Extract JSON object from response
            let start = response.find('{').unwrap_or(0);
            let end = response.find('}').map(|i| i + 1).unwrap_or(response.len());
            response[start..end].to_string()
        } else {
            "{}".to_string()
        };

        // Ensure valid JSON structure
        let payload = try {
            let parsed = serde_json::from_str::<serde_json::Value>(&json_str);
            serde_json::json!({
                "doc_id": doc_name.strip_suffix(".mbx.md").unwrap_or(doc_name),
                "title": doc_name,
                "source": doc_path.to_string_lossy().to_string(),
                "raw_content": _raw_content,
                "entities": parsed.get("entities").and_then(|v| v.as_array()).unwrap_or(&[]),
                "relations": parsed.get("relations").and_then(|v| v.as_array()).unwrap_or(&[]),
            })
        } catch {
            eprintln!("  ⚠️  Failed to parse ollama response, creating minimal payload");
            serde_json::json!({
                "doc_id": doc_name,
                "title": doc_name,
                "source": doc_path.to_string_lossy().to_string(),
                "raw_content": _raw_content,
                "entities": [],
                "relations": [],
            })
        };

        // Ingest via kgx
        print "  📥 Ingesting to kgx...\n";

        let kgx_output = Command::new("kgx")
            .args(["--root", args.kgx_root.to_str().unwrap_or(""), "ingest"])
            .stdin(Stdio::piped())
            .output()
            .context("failed to call kgx")?;

        if !kgx_output.status.success() {
            let stderr = String::from_utf8_lossy(&kgx_output.stderr);
            eprintln!("  ⚠️  kgx ingest failed: {}", stderr.trim());
        } else {
            let nodes = kgx_output
                .stdout
                .lines()
                .next()
                .and_then(|l| l.split_once(' ').and_then(|(n, _)| n.parse::<usize>().ok()))
                .unwrap_or(0);
            let edges = kgx_output
                .stdout
                .lines()
                .nth(1)
                .and_then(|l| l.split_once(' ').and_then(|(n, _)| n.parse::<usize>().ok()))
                .unwrap_or(0);
            println!("    Ingested: ({}) nodes, ({}) edges", nodes, edges);
        }
    }

    // Summary
    println!("\nSummary:");
    println!("   Total docs processed: {}", missing.len());
    println!("   Remaining for manual review: 0");
    println!("   kgx root: {:?}", args.kgx_root);

    Ok(())
}