package llm

import (
	"context"
	"errors"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

type stubProvider struct {
	name string
	resp domain.CompletionResponse
	err  error
	got  domain.CompletionRequest
}

func (p *stubProvider) Name() string { return p.name }
func (p *stubProvider) Complete(_ context.Context, req domain.CompletionRequest) (domain.CompletionResponse, error) {
	p.got = req
	return p.resp, p.err
}

func TestLlmRunnerSuccess(t *testing.T) {
	p := &stubProvider{
		name: "test/model",
		resp: domain.CompletionResponse{Text: "hello", Provider: "test/model"},
	}
	r := NewLlmRunner(p)

	res, err := r.Run(context.Background(), domain.AgentConfig{
		Name:   "agent1",
		Prompt: "say hi",
	})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if res.Name != "agent1" {
		t.Errorf("name = %q, want agent1", res.Name)
	}
	if res.Output != "hello" {
		t.Errorf("output = %q, want hello", res.Output)
	}
	if res.Error != "" {
		t.Errorf("error = %q, want empty", res.Error)
	}
}

func TestLlmRunnerError(t *testing.T) {
	p := &stubProvider{
		name: "test/model",
		err:  errors.New("api down"),
	}
	r := NewLlmRunner(p)

	res, err := r.Run(context.Background(), domain.AgentConfig{
		Name:   "agent1",
		Prompt: "say hi",
	})
	if err == nil {
		t.Fatal("expected error")
	}
	if res.Name != "agent1" {
		t.Errorf("name = %q, want agent1", res.Name)
	}
	if res.Error != "api down" {
		t.Errorf("error field = %q, want 'api down'", res.Error)
	}
}

func TestLlmRunnerRoleFallback(t *testing.T) {
	p := &stubProvider{
		name: "test/model",
		resp: domain.CompletionResponse{Text: "ok"},
	}
	r := NewLlmRunner(p)

	_, _ = r.Run(context.Background(), domain.AgentConfig{
		Name:   "agent1",
		Role:   "a code reviewer",
		Prompt: "review this",
	})
	if p.got.System != "You are a code reviewer." {
		t.Errorf("system = %q, want role-based fallback", p.got.System)
	}
}

func TestLlmRunnerExplicitSystemPrompt(t *testing.T) {
	p := &stubProvider{
		name: "test/model",
		resp: domain.CompletionResponse{Text: "ok"},
	}
	r := NewLlmRunner(p)

	_, _ = r.Run(context.Background(), domain.AgentConfig{
		Name:         "agent1",
		Role:         "ignored",
		SystemPrompt: "custom system",
		Prompt:       "do stuff",
	})
	if p.got.System != "custom system" {
		t.Errorf("system = %q, want 'custom system'", p.got.System)
	}
}

func TestLlmRunnerProviderName(t *testing.T) {
	p := &stubProvider{name: "openai/gpt-4o"}
	r := NewLlmRunner(p)
	if r.ProviderName() != "openai/gpt-4o" {
		t.Errorf("provider name = %q", r.ProviderName())
	}
}
