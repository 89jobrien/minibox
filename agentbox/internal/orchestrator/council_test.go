package orchestrator

import (
	"context"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

type mockRunner struct {
	responses map[string]string
}

func (m *mockRunner) Run(_ context.Context, config domain.AgentConfig) (domain.AgentResult, error) {
	output := m.responses[config.Name]
	if output == "" {
		output = "Mock output for " + config.Name
	}
	return domain.AgentResult{Name: config.Name, Output: output}, nil
}

func TestCouncilCoreRoles(t *testing.T) {
	roles := CoreRoles()
	if len(roles) != 3 {
		t.Errorf("core roles = %d, want 3", len(roles))
	}
	names := map[string]bool{}
	for _, r := range roles {
		names[r.Key] = true
	}
	for _, expected := range []string{"strict-critic", "creative-explorer", "general-analyst"} {
		if !names[expected] {
			t.Errorf("missing role %q", expected)
		}
	}
}

func TestCouncilExtensiveRoles(t *testing.T) {
	roles := ExtensiveRoles()
	if len(roles) != 5 {
		t.Errorf("extensive roles = %d, want 5", len(roles))
	}
}

func TestCouncilRunCollectsAllRoles(t *testing.T) {
	runner := &mockRunner{responses: map[string]string{
		"strict-critic":     "Score: 0.7\nBad code",
		"creative-explorer": "Score: 0.9\nGreat ideas",
		"general-analyst":   "Score: 0.8\nBalanced view",
	}}

	council := &Council{runner: runner}
	results, err := council.RunRoles(context.Background(), CoreRoles(), "test context")
	if err != nil {
		t.Fatalf("RunRoles: %v", err)
	}
	if len(results) != 3 {
		t.Errorf("results = %d, want 3", len(results))
	}
}
