package domain

import (
	"encoding/json"
	"testing"
	"time"
)

func TestMessageRoundTrip(t *testing.T) {
	msg := Message{
		Source:        "council-critic",
		Timestamp:     time.Date(2026, 3, 25, 12, 0, 0, 0, time.UTC),
		Topic:         "result.council.abc123",
		SchemaVersion: 1,
		Payload:       json.RawMessage(`{"score":0.82}`),
	}

	data, err := json.Marshal(msg)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}

	var got Message
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}

	if got.Source != msg.Source {
		t.Errorf("source = %q, want %q", got.Source, msg.Source)
	}
	if got.Topic != msg.Topic {
		t.Errorf("topic = %q, want %q", got.Topic, msg.Topic)
	}
	if got.SchemaVersion != 1 {
		t.Errorf("schema_version = %d, want 1", got.SchemaVersion)
	}
}

func TestAgentRunJSONL(t *testing.T) {
	run := AgentRun{
		RunID:  "2026-03-25T12:00:00",
		Script: "council",
		Args:   map[string]any{"base": "main", "mode": "core"},
		Status: "running",
	}

	data, err := json.Marshal(run)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}

	// Verify it contains expected fields (JSONL compat with Python agent_log)
	var m map[string]any
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("unmarshal map: %v", err)
	}
	if m["run_id"] != "2026-03-25T12:00:00" {
		t.Errorf("run_id = %v, want 2026-03-25T12:00:00", m["run_id"])
	}
	if m["script"] != "council" {
		t.Errorf("script = %v, want council", m["script"])
	}
	if m["status"] != "running" {
		t.Errorf("status = %v, want running", m["status"])
	}
}
