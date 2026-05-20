package llm

import (
	"context"
	"fmt"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// LlmRunner adapts an LlmProvider into the AgentRunner interface.
// This allows direct API calls (OpenAI, Gemini) to be used wherever
// the Claude SDK runner is used.
type LlmRunner struct {
	provider domain.LlmProvider
}

// NewLlmRunner wraps an LlmProvider as an AgentRunner.
func NewLlmRunner(provider domain.LlmProvider) *LlmRunner {
	return &LlmRunner{provider: provider}
}

func (r *LlmRunner) Run(ctx context.Context, config domain.AgentConfig) (domain.AgentResult, error) {
	system := config.SystemPrompt
	if system == "" && config.Role != "" {
		system = fmt.Sprintf("You are %s.", config.Role)
	}

	resp, err := r.provider.Complete(ctx, domain.CompletionRequest{
		Prompt:    config.Prompt,
		System:    system,
		MaxTokens: 8192,
	})
	if err != nil {
		return domain.AgentResult{
			Name:  config.Name,
			Error: err.Error(),
		}, fmt.Errorf("%s: %w", r.provider.Name(), err)
	}

	return domain.AgentResult{
		Name:   config.Name,
		Output: resp.Text,
	}, nil
}

// ProviderName returns the underlying provider name.
func (r *LlmRunner) ProviderName() string {
	return r.provider.Name()
}
