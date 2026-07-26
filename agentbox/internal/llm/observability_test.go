package llm

import (
	"context"
	"testing"
	"time"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// mockLatencyProvider simulates an LLM provider that populates observability fields.
type mockLatencyProvider struct {
	name    string
	latency time.Duration
}

func (m *mockLatencyProvider) Name() string { return m.name }

func (m *mockLatencyProvider) Complete(_ context.Context, _ domain.CompletionRequest) (domain.CompletionResponse, error) {
	start := time.Now()
	time.Sleep(m.latency)
	return domain.CompletionResponse{
		Text:         "response",
		Provider:     m.name,
		LatencyMs:    time.Since(start).Milliseconds(),
		InputTokens:  10,
		OutputTokens: 20,
	}, nil
}

func TestObservabilityFieldsPopulated(t *testing.T) {
	p := &mockLatencyProvider{name: "test/mock", latency: 5 * time.Millisecond}
	resp, err := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "hello"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.LatencyMs <= 0 {
		t.Errorf("latency_ms = %d, want > 0", resp.LatencyMs)
	}
	if resp.InputTokens != 10 {
		t.Errorf("input_tokens = %d, want 10", resp.InputTokens)
	}
	if resp.OutputTokens != 20 {
		t.Errorf("output_tokens = %d, want 20", resp.OutputTokens)
	}
	if resp.Provider != "test/mock" {
		t.Errorf("provider = %q, want test/mock", resp.Provider)
	}
}

func TestObservabilityFieldsPassThroughChain(t *testing.T) {
	p := &mockLatencyProvider{name: "inner", latency: 5 * time.Millisecond}
	chain := NewFallbackChain(p)

	resp, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "hello"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.LatencyMs <= 0 {
		t.Errorf("latency_ms = %d, want > 0 after chain passthrough", resp.LatencyMs)
	}
	if resp.InputTokens != 10 {
		t.Errorf("input_tokens = %d, want 10", resp.InputTokens)
	}
	if resp.OutputTokens != 20 {
		t.Errorf("output_tokens = %d, want 20", resp.OutputTokens)
	}
}
