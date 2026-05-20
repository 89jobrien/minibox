#!/usr/bin/env rust-script
//! Analyze harvested Claude Code session data via the Anthropic API.
//!
//! Usage: analyze.rs <sessions.json> [YYYY-MM-DD]
//!
//! Reads JSON produced by harvest.rs, builds a transcript, and calls
//! claude-haiku-4-5 to produce a structured digest (goals, tasks, hours).
//! Caches the result to ~/.claude/skills/whatidid/cache/YYYY-MM-DD.json.
//! Emits the digest JSON to stdout.
//!
//! Requires: OPENAI_API_KEY in environment (or via op run).
//!
//! ```cargo
//! [dependencies]
//! serde = { version = "1", features = ["derive"] }
//! serde_json = "1"
//! anyhow = "1"
//! chrono = "0.4"
//! ureq = { version = "2", features = ["json"] }
//! ```

use anyhow::{bail, Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

// ── Digest types (mirrors analysis.txt OUTPUT SCHEMA) ───────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct Digest {
    headline: String,
    primary_focus: String,
    day_narrative: String,
    goals: Vec<Goal>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Goal {
    title: String,
    label: String,
    summary: String,
    human_hours: f32,
    project: String,
    docs_referenced: Vec<String>,
    tasks: Vec<Task>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Task {
    title: String,
    what_got_done: String,
    domain_skills: Vec<String>,
    tech_skills: Vec<String>,
    task_type: String,
    professional_roles: Vec<String>,
    human_hours: f32,
}

fn main() -> Result<()> {
    let sessions_path = std::env::args()
        .nth(1)
        .context("usage: analyze.rs <sessions.json> [YYYY-MM-DD]")?;

    let date_str = std::env::args()
        .nth(2)
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string());

    // Cache check
    let cache_path = cache_path(&date_str)?;
    if cache_path.exists() {
        let cached = fs::read_to_string(&cache_path)
            .with_context(|| format!("read cache {}", cache_path.display()))?;
        print!("{cached}");
        return Ok(());
    }

    let sessions_raw = fs::read_to_string(&sessions_path)
        .with_context(|| format!("read {sessions_path}"))?;
    let sessions: Vec<Value> = serde_json::from_str(&sessions_raw)
        .context("parse sessions JSON")?;

    if sessions.is_empty() {
        bail!("no sessions found for {date_str}");
    }

    let transcript = build_transcript(&sessions);
    let analysis_prompt = load_analysis_prompt()?;
    let prompt = analysis_prompt.replace("{transcript}", &transcript);

    let api_key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY not set — run via: op run -- analyze.rs ...")?;

    let response = ureq::post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("content-type", "application/json")
        .send_json(json!({
            "model": "gpt-4o-mini",
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": prompt}]
        }))
        .context("OpenAI API call failed")?;

    let body: Value = response.into_json().context("parse API response")?;
    let content_raw = body["choices"][0]["message"]["content"]
        .as_str()
        .context("missing choices[0].message.content in response")?;

    // Strip markdown fences if present (gpt-4o-mini wraps despite prompt instruction)
    let content = content_raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    // Validate it's parseable JSON before caching
    let digest: Digest = serde_json::from_str(content)
        .with_context(|| format!("model returned invalid JSON:\n{content}"))?;

    let out = serde_json::to_string_pretty(&digest)?;
    fs::create_dir_all(cache_path.parent().unwrap())?;
    fs::write(&cache_path, &out)
        .with_context(|| format!("write cache {}", cache_path.display()))?;

    println!("{out}");
    Ok(())
}

fn build_transcript(sessions: &[Value]) -> String {
    let mut parts = Vec::new();

    for (i, session) in sessions.iter().enumerate() {
        let cwd = session["cwd"].as_str().unwrap_or("unknown");
        let branch = session["git_branch"].as_str().unwrap_or("");
        let project = session["project_slug"].as_str().unwrap_or("unknown");
        let started = session["started_at"].as_str().unwrap_or("");
        let tools = session["tool_calls"].as_u64().unwrap_or(0);
        let reads = session["read_calls"].as_u64().unwrap_or(0);
        let edits = session["edit_calls"].as_u64().unwrap_or(0);

        parts.push(format!(
            "SESSION {} — project={project}  cwd={cwd}  branch={branch}  started={started}\n\
             SIGNALS: tool_calls={tools}  read_calls={reads}  edit_calls={edits}",
            i + 1
        ));

        if let Some(messages) = session["messages"].as_array() {
            for msg in messages {
                let role = msg["role"].as_str().unwrap_or("?");
                let text = msg["text"].as_str().unwrap_or("").trim();
                if !text.is_empty() {
                    parts.push(format!("[{role}] {text}"));
                }
                if let Some(tools) = msg["tool_requests"].as_array() {
                    let names: Vec<&str> = tools
                        .iter()
                        .filter_map(|t| t.as_str())
                        .collect();
                    if !names.is_empty() {
                        parts.push(format!("  tools: {}", names.join(", ")));
                    }
                }
            }
        }
        parts.push(String::new());
    }

    parts.join("\n")
}

fn load_analysis_prompt() -> Result<String> {
    // Try skill-local path first, then fallback to relative
    let skill_dir = skill_dir()?;
    let prompt_path = skill_dir.join("prompts/analysis.whatidid.txt");
    fs::read_to_string(&prompt_path)
        .with_context(|| format!("read prompt {}", prompt_path.display()))
}

fn cache_path(date: &str) -> Result<PathBuf> {
    let skill_dir = skill_dir()?;
    Ok(skill_dir.join(format!("cache/{date}.json")))
}

fn skill_dir() -> Result<PathBuf> {
    // Resolve from this script's location at runtime
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home)
        .join("dev/minibox/.claude/skills/whatidid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_transcript_formats_sessions() {
        let sessions: Vec<Value> = vec![serde_json::json!({
            "cwd": "/dev/project",
            "git_branch": "main",
            "project_slug": "minibox",
            "started_at": "2026-05-20T10:00:00Z",
            "tool_calls": 5,
            "read_calls": 2,
            "edit_calls": 1,
            "messages": [
                {"role": "human", "text": "fix the bug", "tool_requests": []},
                {"role": "assistant", "text": "I'll look at it", "tool_requests": ["Read"]}
            ]
        })];
        let transcript = build_transcript(&sessions);
        assert!(transcript.contains("SESSION 1"));
        assert!(transcript.contains("project=minibox"));
        assert!(transcript.contains("[human] fix the bug"));
        assert!(transcript.contains("[assistant] I'll look at it"));
        assert!(transcript.contains("tools: Read"));
    }

    #[test]
    fn test_build_transcript_empty_sessions() {
        let sessions: Vec<Value> = vec![];
        let transcript = build_transcript(&sessions);
        assert!(transcript.is_empty());
    }

    #[test]
    fn test_cache_path_format() {
        let path = cache_path("2026-05-20").expect("cache_path should succeed");
        assert!(path.to_string_lossy().contains("cache/2026-05-20.json"));
    }

    #[test]
    fn test_digest_schema_roundtrip() {
        let json = r#"{
            "headline": "Test day",
            "primary_focus": "testing",
            "day_narrative": "Wrote tests all day",
            "goals": [{
                "title": "Testing",
                "label": "test",
                "summary": "Add tests",
                "human_hours": 2.0,
                "project": "minibox",
                "docs_referenced": [],
                "tasks": [{
                    "title": "Unit tests",
                    "what_got_done": "Added tests",
                    "domain_skills": ["testing"],
                    "tech_skills": ["rust"],
                    "task_type": "testing",
                    "professional_roles": ["developer"],
                    "human_hours": 2.0
                }]
            }]
        }"#;
        let digest: Digest = serde_json::from_str(json).expect("valid digest JSON");
        assert_eq!(digest.headline, "Test day");
        assert_eq!(digest.goals.len(), 1);
        assert_eq!(digest.goals[0].tasks.len(), 1);
    }
}
