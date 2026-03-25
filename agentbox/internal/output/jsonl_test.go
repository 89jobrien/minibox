package output

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

func TestJSONLWriterFormat(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "agent-runs.jsonl")
	w := NewJSONLWriter(path)

	run := domain.AgentRun{
		RunID:  "2026-03-25T12:00:00",
		Script: "council",
		Args:   map[string]any{"base": "main"},
		Status: "running",
	}
	if err := w.WriteRun(context.Background(), run); err != nil {
		t.Fatalf("WriteRun: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile: %v", err)
	}

	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) != 1 {
		t.Fatalf("expected 1 line, got %d", len(lines))
	}

	var m map[string]any
	if err := json.Unmarshal([]byte(lines[0]), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if m["run_id"] != "2026-03-25T12:00:00" {
		t.Errorf("run_id = %v, want 2026-03-25T12:00:00", m["run_id"])
	}
	if m["script"] != "council" {
		t.Errorf("script = %v, want council", m["script"])
	}
}

func TestJSONLWriterAppends(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "agent-runs.jsonl")
	w := NewJSONLWriter(path)

	ctx := context.Background()
	w.WriteRun(ctx, domain.AgentRun{RunID: "1", Script: "a", Status: "running"})
	w.WriteRun(ctx, domain.AgentRun{RunID: "2", Script: "b", Status: "complete"})

	data, _ := os.ReadFile(path)
	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected 2 lines, got %d", len(lines))
	}
}

func TestReportWriter(t *testing.T) {
	dir := t.TempDir()
	w := NewReportWriter(dir)

	report := domain.AgentReport{
		SHA:     "abc1234",
		Script:  "council-core",
		Content: "## Findings\nAll good.",
		Meta:    map[string]string{"base": "main", "mode": "core"},
	}
	if err := w.WriteReport(context.Background(), report); err != nil {
		t.Fatalf("WriteReport: %v", err)
	}

	path := filepath.Join(dir, "abc1234-council-core.md")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile: %v", err)
	}

	content := string(data)
	if !strings.Contains(content, "# council-core") {
		t.Error("missing header")
	}
	if !strings.Contains(content, "abc1234") {
		t.Error("missing SHA in header")
	}
	if !strings.Contains(content, "## Findings") {
		t.Error("missing content body")
	}
}
