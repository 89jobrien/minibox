package tools

import (
	"context"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

type mockRunner struct {
	lastPrompt string
}

func (m *mockRunner) Run(_ context.Context, config domain.AgentConfig) (domain.AgentResult, error) {
	m.lastPrompt = config.Prompt
	return domain.AgentResult{Name: config.Name, Output: "feat(agentbox): add commit message tool"}, nil
}

func TestCommitMsgPromptContainsRules(t *testing.T) {
	runner := &mockRunner{}
	cm := NewCommitMsg(runner)

	ctx := CommitMsgContext{
		Branch:     "feature-x",
		StagedDiff: "diff --git a/foo.go\n+new line",
		StagedStat: "1 file changed, 1 insertion",
		RecentLog:  "abc123 feat: prior change",
	}

	_, err := cm.Generate(context.Background(), ctx)
	if err != nil {
		t.Fatalf("Generate: %v", err)
	}

	for _, expected := range []string{"conventional commits", "feat, fix, docs", "≤72 chars"} {
		if !contains(runner.lastPrompt, expected) {
			t.Errorf("prompt missing %q", expected)
		}
	}
}

func contains(s, sub string) bool {
	for i := 0; i <= len(s)-len(sub); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
