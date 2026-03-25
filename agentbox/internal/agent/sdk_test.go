package agent

import (
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

func TestAgentConfigToQueryOptions(t *testing.T) {
	config := domain.AgentConfig{
		Name:   "test-agent",
		Prompt: "Analyze this code",
		Tools:  []string{"Read", "Glob", "Grep"},
	}

	opts := configToQueryOptions(config)
	if len(opts) == 0 {
		t.Error("expected non-empty query options")
	}
}
