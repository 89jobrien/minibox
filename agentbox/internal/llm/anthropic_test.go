package llm

import (
	"context"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// mockProvider implements domain.LlmProvider for testing.
type mockProvider struct {
	name string
	resp domain.CompletionResponse
	err  error
}

func (m *mockProvider) Name() string { return m.name }
func (m *mockProvider) Complete(_ context.Context, _ domain.CompletionRequest) (domain.CompletionResponse, error) {
	return m.resp, m.err
}

func TestAnthropicProviderName(t *testing.T) {
	p := NewAnthropicProvider("test-key")
	if p.Name() != "anthropic/claude-sonnet-4-6" {
		t.Errorf("name = %q, want anthropic/claude-sonnet-4-6", p.Name())
	}
}
