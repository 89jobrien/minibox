//! ATIF v1.6 — Agent Trajectory Interchange Format types and writer.
//!
//! Implements the ATIF specification for logging complete LLM agent interaction
//! histories. Trajectories are written as YAML to
//! `~/.minibox/trajectories/<session_id>.yaml`.
//!
//! # Schema version
//!
//! This module targets **ATIF-v1.6**, which adds multimodal content support
//! (`ContentPart`, `ImageSource`) and the `has_multimodal_content()` helper.
//!
//! # Quick start
//!
//! ```ignore
//! use minibox_agent::trajectory::{Trajectory, TrajectoryWriter, AgentInfo, StepObject, Source};
//!
//! let mut traj = Trajectory::new("my-agent", "1.0.0", "session-uuid");
//! traj.push(StepObject::user(1, "What is 2+2?"));
//! traj.push(StepObject::agent(2, "4."));
//!
//! let writer = TrajectoryWriter::new()?;
//! writer.write(&traj)?;
//! ```

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Schema version
// ---------------------------------------------------------------------------

pub const SCHEMA_VERSION: &str = "ATIF-v1.6";

// ---------------------------------------------------------------------------
// Root trajectory
// ---------------------------------------------------------------------------

/// Root ATIF trajectory object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    /// ATIF schema version string (e.g. `"ATIF-v1.6"`).
    pub schema_version: String,

    /// Unique identifier for this agent session.
    pub session_id: String,

    /// Agent configuration.
    pub agent: AgentInfo,

    /// Complete interaction history.
    pub steps: Vec<StepObject>,

    /// Developer notes or format-discrepancy explanations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,

    /// Aggregate metrics for the entire trajectory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_metrics: Option<FinalMetrics>,

    /// Reference to a continuation trajectory file, if this one was split.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continued_trajectory_ref: Option<String>,

    /// Custom root-level metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl Trajectory {
    /// Create a new trajectory with the given agent name, version, and session ID.
    pub fn new(
        agent_name: impl Into<String>,
        agent_version: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            session_id: session_id.into(),
            agent: AgentInfo {
                name: agent_name.into(),
                version: agent_version.into(),
                model_name: None,
                tool_definitions: None,
                extra: None,
            },
            steps: Vec::new(),
            notes: None,
            final_metrics: None,
            continued_trajectory_ref: None,
            extra: None,
        }
    }

    /// Append a step.
    pub fn push(&mut self, step: StepObject) {
        self.steps.push(step);
    }

    /// Returns `true` if any step contains multimodal (image) content.
    pub fn has_multimodal_content(&self) -> bool {
        self.steps.iter().any(|s| {
            let msg_has_image = matches!(&s.message, MessageContent::Parts(parts)
                if parts.iter().any(|p| matches!(p, ContentPart::Image { .. })));

            let obs_has_image = s.observation.as_ref().map_or(false, |obs| {
                obs.results.iter().any(|r| {
                    r.content.as_ref().map_or(false, |c| {
                        matches!(c, MessageContent::Parts(parts)
                            if parts.iter().any(|p| matches!(p, ContentPart::Image { .. })))
                    })
                })
            });

            msg_has_image || obs_has_image
        })
    }
}

// ---------------------------------------------------------------------------
// Agent info
// ---------------------------------------------------------------------------

/// Agent configuration block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Agent system name (e.g. `"minibox-agent"`).
    pub name: String,

    /// Agent version (e.g. `"1.0.0"`).
    pub version: String,

    /// Default model for this trajectory. Step-level `model_name` overrides this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,

    /// Tool/function definitions available to the agent (OpenAI function schema).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_definitions: Option<Vec<serde_json::Value>>,

    /// Custom agent configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Final metrics
// ---------------------------------------------------------------------------

/// Aggregate statistics for the entire trajectory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FinalMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_prompt_tokens: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_completion_tokens: Option<u64>,

    /// Subset of `total_prompt_tokens` that were cache hits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cached_tokens: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,

    /// Total steps (may differ from `steps` array length — explain in `notes`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_steps: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl FinalMetrics {
    /// Accumulate per-step metrics into this summary.
    pub fn accumulate(&mut self, m: &StepMetrics) {
        if let Some(p) = m.prompt_tokens {
            *self.total_prompt_tokens.get_or_insert(0) += p;
        }
        if let Some(c) = m.completion_tokens {
            *self.total_completion_tokens.get_or_insert(0) += c;
        }
        if let Some(c) = m.cached_tokens {
            *self.total_cached_tokens.get_or_insert(0) += c;
        }
        if let Some(cost) = m.cost_usd {
            let t = self.total_cost_usd.get_or_insert(0.0);
            *t += cost;
        }
    }
}

// ---------------------------------------------------------------------------
// Step
// ---------------------------------------------------------------------------

/// The originator of a step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    System,
    User,
    Agent,
}

/// A single interaction turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepObject {
    /// 1-based ordinal index.
    pub step_id: u32,

    /// ISO 8601 timestamp (e.g. `"2025-10-16T14:30:00Z"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,

    /// Originator of this step.
    pub source: Source,

    /// Model used for this turn (agent steps only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,

    /// Qualitative or quantitative reasoning effort (agent steps only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,

    /// Dialogue message — text or multimodal content parts.
    pub message: MessageContent,

    /// Explicit internal reasoning (agent steps only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,

    /// Tool invocations (agent steps only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Environment feedback after actions, or system-initiated operation results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation: Option<Observation>,

    /// LLM operational metrics (agent steps only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<StepMetrics>,

    /// Custom step-level metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl StepObject {
    /// Construct a system-prompt step.
    pub fn system(step_id: u32, prompt: impl Into<String>) -> Self {
        Self::bare(step_id, Source::System, prompt.into())
    }

    /// Construct a user-message step.
    pub fn user(step_id: u32, message: impl Into<String>) -> Self {
        Self::bare(step_id, Source::User, message.into())
    }

    /// Construct an agent-response step (no tool calls).
    pub fn agent(step_id: u32, message: impl Into<String>) -> Self {
        Self::bare(step_id, Source::Agent, message.into())
    }

    fn bare(step_id: u32, source: Source, text: String) -> Self {
        Self {
            step_id,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
            source,
            model_name: None,
            reasoning_effort: None,
            message: MessageContent::Text(text),
            reasoning_content: None,
            tool_calls: None,
            observation: None,
            metrics: None,
            extra: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Reasoning effort
// ---------------------------------------------------------------------------

/// Qualitative or numeric reasoning effort assigned to an agent step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ReasoningEffort {
    /// Qualitative label: `"low"`, `"medium"`, `"high"`.
    Label(String),
    /// Numeric score.
    Score(f64),
}

// ---------------------------------------------------------------------------
// Message content (text or multimodal)
// ---------------------------------------------------------------------------

/// A step message or observation result — either plain text or a sequence of
/// content parts (text + images).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text message.
    Text(String),
    /// Multimodal content (v1.6+): array of text and/or image parts.
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    /// Construct a text-only value.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    /// Returns the text if this is a plain-text message.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s.as_str()),
            Self::Parts(_) => None,
        }
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        Self::Text(s.to_owned())
    }
}

// ---------------------------------------------------------------------------
// Content parts (v1.6+)
// ---------------------------------------------------------------------------

/// A single content part within a multimodal message or observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentPart {
    /// Plain text part.
    Text { text: String },
    /// Image part — references an image file stored alongside the trajectory.
    Image { source: ImageSource },
}

/// Image reference for a content part.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    /// MIME type: `"image/jpeg"`, `"image/png"`, `"image/gif"`, or `"image/webp"`.
    pub media_type: String,

    /// Relative path (e.g. `"images/step_1.png"`), absolute path, or URL.
    pub path: String,
}

// ---------------------------------------------------------------------------
// Tool calls
// ---------------------------------------------------------------------------

/// A structured tool invocation made by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID correlating with `ObservationResult::source_call_id`.
    pub tool_call_id: String,

    /// Tool or function name.
    pub function_name: String,

    /// Arguments passed to the function.
    pub arguments: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Observation
// ---------------------------------------------------------------------------

/// Environment feedback container — holds one result per tool call or action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Results from tool calls or system-initiated operations.
    pub results: Vec<ObservationResult>,
}

impl Observation {
    /// Build an observation with a single text result for `source_call_id`.
    pub fn single(source_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            results: vec![ObservationResult {
                source_call_id: Some(source_call_id.into()),
                content: Some(MessageContent::Text(content.into())),
                subagent_trajectory_ref: None,
            }],
        }
    }
}

/// A single result within an observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationResult {
    /// Correlates with `ToolCall::tool_call_id`. `None` for non-tool actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_call_id: Option<String>,

    /// Tool output — text or multimodal content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,

    /// Delegated subagent trajectory references.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_trajectory_ref: Option<Vec<SubagentTrajectoryRef>>,
}

// ---------------------------------------------------------------------------
// Subagent trajectory reference
// ---------------------------------------------------------------------------

/// Reference to a delegated subagent trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTrajectoryRef {
    /// Session ID of the delegated subagent trajectory.
    pub session_id: String,

    /// File path, S3 URL, or other locator for the full trajectory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trajectory_path: Option<String>,

    /// Custom metadata (summary, exit status, performance).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Per-step metrics
// ---------------------------------------------------------------------------

/// LLM operational and cost metrics for a single agent step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepMetrics {
    /// Total input tokens sent (cached + non-cached).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,

    /// Tokens generated in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,

    /// Subset of `prompt_tokens` served from cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,

    /// Monetary cost of this API call in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,

    /// Token IDs for prompt tokens (enables tokenization analysis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_token_ids: Option<Vec<u32>>,

    /// Token IDs for completion tokens (enables accurate RL training).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_token_ids: Option<Vec<u32>>,

    /// Log probabilities per completion token. Aligns with `completion_token_ids`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Vec<f64>>,

    /// Provider-specific extras (e.g. `cache_creation_input_tokens`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// TrajectoryWriter
// ---------------------------------------------------------------------------

/// Writes completed [`Trajectory`] objects to
/// `~/.minibox/trajectories/<session_id>.yaml`.
///
/// # Blocking I/O
///
/// All methods perform synchronous filesystem I/O. Wrap in
/// `tokio::task::spawn_blocking` when calling from async code.
pub struct TrajectoryWriter {
    dir: PathBuf,
}

impl TrajectoryWriter {
    /// Build with the default directory: `~/.minibox/trajectories/`.
    pub fn new() -> Result<Self> {
        let dir = dirs::home_dir()
            .context("cannot determine home directory; set HOME")?
            .join(".minibox")
            .join("trajectories");
        Self::with_dir(dir)
    }

    /// Build with an explicit directory (useful in tests).
    pub fn with_dir(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)
            .with_context(|| format!("create trajectory directory: {}", dir.display()))?;
        Ok(Self { dir })
    }

    /// Serialize `trajectory` to YAML and write to
    /// `{dir}/{session_id}.yaml`, replacing any existing file.
    pub fn write(&self, trajectory: &Trajectory) -> Result<PathBuf> {
        let path = self.path_for(&trajectory.session_id);
        let yaml = serde_yaml::to_string(trajectory).context("serialize trajectory to YAML")?;
        fs::write(&path, yaml).with_context(|| format!("write trajectory: {}", path.display()))?;
        Ok(path)
    }

    /// Return the path that would be used for `session_id` without writing.
    pub fn path_for(&self, session_id: &str) -> PathBuf {
        self.dir.join(format!("{session_id}.yaml"))
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn writer(tmp: &TempDir) -> TrajectoryWriter {
        TrajectoryWriter::with_dir(tmp.path()).expect("writer")
    }

    fn simple_trajectory() -> Trajectory {
        let mut t = Trajectory::new("minibox-agent", "0.1.0", "sess-001");
        t.push(StepObject::system(1, "You are a helpful assistant."));
        t.push(StepObject::user(2, "What is 2+2?"));
        t.push(StepObject::agent(3, "4."));
        t
    }

    #[test]
    fn write_and_file_exists() {
        let tmp = TempDir::new().expect("tmpdir");
        let traj = simple_trajectory();
        let path = writer(&tmp).write(&traj).expect("write");
        assert!(path.exists(), "trajectory file must exist after write");
    }

    #[test]
    fn written_yaml_roundtrips() {
        let tmp = TempDir::new().expect("tmpdir");
        let traj = simple_trajectory();
        let path = writer(&tmp).write(&traj).expect("write");

        let content = fs::read_to_string(&path).expect("read");
        let back: Trajectory = serde_yaml::from_str(&content).expect("parse YAML");

        assert_eq!(back.session_id, traj.session_id);
        assert_eq!(back.schema_version, SCHEMA_VERSION);
        assert_eq!(back.steps.len(), 3);
    }

    #[test]
    fn schema_version_is_atif_v1_6() {
        let traj = Trajectory::new("a", "1", "s");
        assert_eq!(traj.schema_version, "ATIF-v1.6");
    }

    #[test]
    fn has_multimodal_content_false_for_text_only() {
        let traj = simple_trajectory();
        assert!(!traj.has_multimodal_content());
    }

    #[test]
    fn has_multimodal_content_true_for_image_part() {
        let mut traj = Trajectory::new("a", "1", "s");
        let mut step = StepObject::user(1, "ignored");
        step.message = MessageContent::Parts(vec![
            ContentPart::Text {
                text: "look at this".into(),
            },
            ContentPart::Image {
                source: ImageSource {
                    media_type: "image/png".into(),
                    path: "images/step_1.png".into(),
                },
            },
        ]);
        traj.push(step);
        assert!(traj.has_multimodal_content());
    }

    #[test]
    fn final_metrics_accumulate() {
        let mut fm = FinalMetrics::default();
        let m1 = StepMetrics {
            prompt_tokens: Some(100),
            completion_tokens: Some(50),
            cached_tokens: Some(20),
            cost_usd: Some(0.001),
            ..Default::default()
        };
        let m2 = StepMetrics {
            prompt_tokens: Some(200),
            completion_tokens: Some(80),
            cached_tokens: None,
            cost_usd: Some(0.002),
            ..Default::default()
        };
        fm.accumulate(&m1);
        fm.accumulate(&m2);

        assert_eq!(fm.total_prompt_tokens, Some(300));
        assert_eq!(fm.total_completion_tokens, Some(130));
        assert_eq!(fm.total_cached_tokens, Some(20));
        assert!((fm.total_cost_usd.unwrap() - 0.003).abs() < 1e-9);
    }

    #[test]
    fn tool_call_observation_roundtrip() {
        let mut traj = Trajectory::new("a", "1", "tool-sess");
        let mut step = StepObject::agent(1, "I will search.");
        step.tool_calls = Some(vec![ToolCall {
            tool_call_id: "call_1".into(),
            function_name: "web_search".into(),
            arguments: serde_json::json!({"query": "rust serde"}),
        }]);
        step.observation = Some(Observation::single("call_1", "10 results found."));
        traj.push(step);

        let tmp = TempDir::new().expect("tmpdir");
        let path = writer(&tmp).write(&traj).expect("write");
        let content = fs::read_to_string(&path).expect("read");
        let back: Trajectory = serde_yaml::from_str(&content).expect("parse");

        let s = &back.steps[0];
        let tc = s.tool_calls.as_ref().unwrap();
        assert_eq!(tc[0].tool_call_id, "call_1");
        assert_eq!(tc[0].function_name, "web_search");

        let obs = s.observation.as_ref().unwrap();
        assert_eq!(obs.results[0].source_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn path_for_uses_session_id() {
        let tmp = TempDir::new().expect("tmpdir");
        let w = writer(&tmp);
        let p = w.path_for("my-session");
        assert!(p.ends_with("my-session.yaml"));
    }

    #[test]
    fn message_content_from_str() {
        let mc: MessageContent = "hello".into();
        assert_eq!(mc.as_text(), Some("hello"));
    }

    #[test]
    fn subagent_trajectory_ref_serializes() {
        let r = SubagentTrajectoryRef {
            session_id: "sub-001".into(),
            trajectory_path: Some("s3://bucket/sub-001.yaml".into()),
            extra: None,
        };
        let json = serde_json::to_string(&r).expect("serialize");
        assert!(json.contains("sub-001"));
        assert!(json.contains("s3://"));
    }
}
