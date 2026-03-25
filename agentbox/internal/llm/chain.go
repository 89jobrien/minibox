package llm

import (
	"context"
	"fmt"
	"strings"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// FallbackChain tries providers in order, returning the first success.
type FallbackChain struct {
	providers []domain.LlmProvider
}

// NewFallbackChain creates a chain from the given providers. Nil providers are skipped.
func NewFallbackChain(providers ...domain.LlmProvider) *FallbackChain {
	var valid []domain.LlmProvider
	for _, p := range providers {
		if p != nil {
			valid = append(valid, p)
		}
	}
	return &FallbackChain{providers: valid}
}

func (c *FallbackChain) Name() string {
	names := make([]string, len(c.providers))
	for i, p := range c.providers {
		names[i] = p.Name()
	}
	return fmt.Sprintf("chain[%s]", strings.Join(names, ","))
}

func (c *FallbackChain) Complete(ctx context.Context, req domain.CompletionRequest) (domain.CompletionResponse, error) {
	if len(c.providers) == 0 {
		return domain.CompletionResponse{}, fmt.Errorf("no providers configured")
	}

	var errs []string
	for _, p := range c.providers {
		resp, err := p.Complete(ctx, req)
		if err == nil {
			return resp, nil
		}
		errs = append(errs, fmt.Sprintf("%s: %v", p.Name(), err))
	}
	return domain.CompletionResponse{}, fmt.Errorf("all providers failed: %s", strings.Join(errs, "; "))
}
