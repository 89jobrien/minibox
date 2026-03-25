package llm

import (
	"context"
	"fmt"
	"time"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// RetryConfig controls retry behavior.
type RetryConfig struct {
	MaxRetries  int
	BackoffBase time.Duration
}

// DefaultRetryConfig returns sensible defaults (2 retries, 1s base).
func DefaultRetryConfig() RetryConfig {
	return RetryConfig{
		MaxRetries:  2,
		BackoffBase: time.Second,
	}
}

// RetryingProvider wraps an LlmProvider with exponential backoff retries.
type RetryingProvider struct {
	inner  domain.LlmProvider
	config RetryConfig
}

// NewRetryingProvider wraps a provider with retry logic.
func NewRetryingProvider(inner domain.LlmProvider, config RetryConfig) *RetryingProvider {
	return &RetryingProvider{inner: inner, config: config}
}

func (p *RetryingProvider) Name() string { return p.inner.Name() }

func (p *RetryingProvider) Complete(ctx context.Context, req domain.CompletionRequest) (domain.CompletionResponse, error) {
	var lastErr error
	for attempt := 0; attempt <= p.config.MaxRetries; attempt++ {
		if attempt > 0 {
			delay := p.config.BackoffBase * (1 << (attempt - 1))
			if delay > 30*time.Second {
				delay = 30 * time.Second
			}
			select {
			case <-time.After(delay):
			case <-ctx.Done():
				return domain.CompletionResponse{}, fmt.Errorf("%s: %w", p.inner.Name(), ctx.Err())
			}
		}
		resp, err := p.inner.Complete(ctx, req)
		if err == nil {
			return resp, nil
		}
		lastErr = err
	}
	return domain.CompletionResponse{}, fmt.Errorf("%s: retries exhausted: %w", p.inner.Name(), lastErr)
}
