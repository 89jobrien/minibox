package orchestrator

import (
	"context"
	"encoding/json"
	"testing"
)

func TestParseAgentPlan(t *testing.T) {
	raw := `[
		{"name": "analyzer", "role": "Analyze code", "prompt": "Look at the code", "tools": ["Read", "Glob"]},
		{"name": "reviewer", "role": "Review tests", "prompt": "Check tests", "tools": ["Read"]}
	]`

	plan, err := parseAgentPlan(raw)
	if err != nil {
		t.Fatalf("parseAgentPlan: %v", err)
	}
	if len(plan) != 2 {
		t.Fatalf("plan length = %d, want 2", len(plan))
	}
	if plan[0].Name != "analyzer" {
		t.Errorf("plan[0].Name = %q, want analyzer", plan[0].Name)
	}
	if len(plan[1].Tools) != 1 || plan[1].Tools[0] != "Read" {
		t.Errorf("plan[1].Tools = %v, want [Read]", plan[1].Tools)
	}
}

func TestParseAgentPlanWithMarkdownFences(t *testing.T) {
	raw := "```json\n[{\"name\": \"test\", \"role\": \"Test\", \"prompt\": \"Do thing\", \"tools\": [\"Read\"]}]\n```"
	plan, err := parseAgentPlan(raw)
	if err != nil {
		t.Fatalf("parseAgentPlan with fences: %v", err)
	}
	if len(plan) != 1 {
		t.Fatalf("plan length = %d, want 1", len(plan))
	}
}

func TestParseAgentPlanInvalid(t *testing.T) {
	_, err := parseAgentPlan("not json at all")
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
}

func TestMetaAgentRunParallel(t *testing.T) {
	runner := &mockRunner{responses: map[string]string{
		"agent-a": "Result A",
		"agent-b": "Result B",
	}}

	meta := NewMetaAgent(runner)
	plan := []AgentSpec{
		{Name: "agent-a", Role: "A", Prompt: "Do A", Tools: []string{"Read"}},
		{Name: "agent-b", Role: "B", Prompt: "Do B", Tools: []string{"Read"}},
	}

	results, err := meta.RunParallel(context.Background(), plan)
	if err != nil {
		t.Fatalf("RunParallel: %v", err)
	}
	if len(results) != 2 {
		t.Fatalf("results = %d, want 2", len(results))
	}
	if results["agent-a"] != "Result A" {
		t.Errorf("agent-a = %q, want Result A", results["agent-a"])
	}
}

func TestMetaAgentSynthesisPrompt(t *testing.T) {
	outputs := map[string]string{"a": "found X", "b": "found Y"}
	prompt := MetaSynthesisPrompt("fix bugs", outputs)
	if prompt == "" {
		t.Fatal("expected non-empty synthesis prompt")
	}

	// Verify it contains expected sections
	for _, expected := range []string{"Summary", "Key Findings", "Recommended Actions", "Open Questions"} {
		if !contains(prompt, expected) {
			t.Errorf("missing section %q in synthesis prompt", expected)
		}
	}
}

// marshalJSON is a test helper
func marshalJSON(v any) string {
	data, _ := json.Marshal(v)
	return string(data)
}
