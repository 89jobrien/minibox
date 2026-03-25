package domain

import (
	"encoding/json"
	"time"
)

// Message is the pub/sub envelope for all inter-agent communication.
type Message struct {
	Source        string          `json:"source"`
	Timestamp     time.Time       `json:"timestamp"`
	Topic         string          `json:"topic"`
	SchemaVersion int             `json:"schema_version"`
	Payload       json.RawMessage `json:"payload"`
}

// AgentConfig is the input to an AgentRunner.
type AgentConfig struct {
	Name         string   `json:"name"`
	Role         string   `json:"role"`
	Prompt       string   `json:"prompt"`
	SystemPrompt string   `json:"system_prompt,omitempty"`
	Tools        []string `json:"tools"`
}

// AgentResult is the output from an AgentRunner.
type AgentResult struct {
	Name   string `json:"name"`
	Output string `json:"output"`
	Error  string `json:"error,omitempty"`
}

// AgentRun is the telemetry record written to agent-runs.jsonl.
type AgentRun struct {
	RunID     string         `json:"run_id"`
	Script    string         `json:"script"`
	Args      map[string]any `json:"args"`
	Status    string         `json:"status"`
	DurationS float64        `json:"duration_s,omitempty"`
	Output    string         `json:"output,omitempty"`
}

// AgentReport is the markdown report written to ai-logs/.
type AgentReport struct {
	SHA     string
	Script  string
	Content string
	Meta    map[string]string
}

// Commit is a parsed git log entry.
type Commit struct {
	SHA     string `json:"sha"`
	Message string `json:"message"`
}

// ProjectContext is discovered project metadata.
type ProjectContext struct {
	Rules     string `json:"rules"`
	Branch    string `json:"branch"`
	GitLog    string `json:"git_log"`
	GitStatus string `json:"git_status"`
	GitStat   string `json:"git_stat"`
	Structure string `json:"structure"`
}
