//! Observations — structured capture of tool inputs and outputs.
//!
//! [`ObservationManager`] accumulates [`Observation`]s during the agentic
//! loop so they can be inspected, logged, or forwarded to an external sink.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::tools::{ToolInput, ToolOutput};

/// A single captured tool invocation (input + output pair).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// ISO-8601 timestamp when the observation was captured.
    pub timestamp: String,
    /// Session the observation belongs to.
    pub session_id: String,
    /// Zero-based turn index.
    pub turn: usize,
    /// The tool input that was sent.
    pub input: ToolInput,
    /// The tool output that was returned.
    pub output: ToolOutput,
}

impl Observation {
    /// Capture a new observation with the current UTC timestamp.
    pub fn capture(
        session_id: impl Into<String>,
        turn: usize,
        input: ToolInput,
        output: ToolOutput,
    ) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            session_id: session_id.into(),
            turn,
            input,
            output,
        }
    }
}

/// Accumulates observations for the current session.
#[derive(Default)]
pub struct ObservationManager {
    observations: Vec<Observation>,
}

impl ObservationManager {
    /// Create an empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an observation.
    pub fn record(&mut self, obs: Observation) {
        self.observations.push(obs);
    }

    /// Return all recorded observations.
    pub fn all(&self) -> &[Observation] {
        &self.observations
    }

    /// Return observations for a specific turn.
    pub fn for_turn(&self, turn: usize) -> Vec<&Observation> {
        self.observations
            .iter()
            .filter(|o| o.turn == turn)
            .collect()
    }

    /// Number of recorded observations.
    pub fn len(&self) -> usize {
        self.observations.len()
    }

    /// `true` if no observations have been recorded.
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    /// Consume the manager and return the inner observations.
    pub fn into_observations(self) -> Vec<Observation> {
        self.observations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_obs(turn: usize) -> Observation {
        let input = ToolInput {
            name: "bash".into(),
            args: serde_json::json!({"command": "echo hi"}),
        };
        let output = ToolOutput::ok("bash", "hi");
        Observation::capture("sess-1", turn, input, output)
    }

    #[test]
    fn observation_manager_captures_tool_input_and_output() {
        let mut mgr = ObservationManager::new();
        mgr.record(make_obs(0));

        assert_eq!(mgr.len(), 1);
        let obs = &mgr.all()[0];
        assert_eq!(obs.input.name, "bash");
        assert_eq!(obs.output.content, "hi");
        assert!(!obs.output.is_error);
    }

    #[test]
    fn for_turn_filters_correctly() {
        let mut mgr = ObservationManager::new();
        mgr.record(make_obs(0));
        mgr.record(make_obs(0));
        mgr.record(make_obs(1));

        assert_eq!(mgr.for_turn(0).len(), 2);
        assert_eq!(mgr.for_turn(1).len(), 1);
        assert_eq!(mgr.for_turn(2).len(), 0);
    }

    #[test]
    fn observation_serializes_to_json() {
        let obs = make_obs(0);
        let json = serde_json::to_string(&obs).expect("serialize");
        let back: Observation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.input.name, obs.input.name);
        assert_eq!(back.output.content, obs.output.content);
    }
}
