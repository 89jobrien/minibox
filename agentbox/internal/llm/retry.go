package llm

import (
	"context"
	"fmt"
	"math/rand/v2"
	"net/http"
	"time"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// RetryConfig controls retry behavior for LLM API calls.
type RetryConfig struct {
	MaxRetries  int
	BackoffBase time.Duration
	MaxDelay    time.Duration
}

// DefaultRetryConfig returns sensible defaults (3 retries, 1s base, 30s max).
func DefaultRetryConfig() RetryConfig {
	return RetryConfig{
		MaxRetries:  3,
		BackoffBase: time.Second,
		MaxDelay:    30 * time.Second,
	}
}

// HTTPStatusError represents an HTTP error with a status code,
// used to determine retryability.
type HTTPStatusError struct {
	StatusCode int
	Err        error
}

func (e *HTTPStatusError) Error() string {
	return fmt.Sprintf("HTTP %d: %s", e.StatusCode, e.Err.Error())
}

func (e *HTTPStatusError) Unwrap() error { return e.Err }

// IsRetryable returns true for rate limit (429) and server errors (5xx).
func IsRetryable(err error) bool {
	if err == nil {
		return false
	}
	var httpErr *HTTPStatusError
	if ok := errorAs(err, &httpErr); ok {
		return httpErr.StatusCode == http.StatusTooManyRequests ||
			httpErr.StatusCode >= 500
	}
	// Default: retry all errors (transient network failures, etc.)
	return true
}

// errorAs is a thin wrapper to allow testing; mirrors errors.As.
func errorAs(err error, target any) bool {
	type asInterface interface{ As(any) bool }
	// Use type assertion chain for HTTPStatusError
	if httpErr, ok := target.(**HTTPStatusError); ok {
		for e := err; e != nil; {
			if h, ok := e.(*HTTPStatusError); ok {
				*httpErr = h
				return true
			}
			if u, ok := e.(interface{ Unwrap() error }); ok {
				e = u.Unwrap()
			} else {
				break
			}
		}
	}
	return false
}

// RetryingProvider wraps an LlmProvider with exponential backoff retries
// and jitter for rate limit (429) and transient (5xx) error handling.
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
			if !IsRetryable(lastErr) {
				break
			}
			delay := p.backoffWithJitter(attempt)
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

// backoffWithJitter computes exponential backoff with full jitter.
func (p *RetryingProvider) backoffWithJitter(attempt int) time.Duration {
	base := p.config.BackoffBase * (1 << (attempt - 1))
	if base > p.config.MaxDelay {
		base = p.config.MaxDelay
	}
	// Full jitter: uniform random in [0, base]
	if base <= 0 {
		return 0
	}
	return time.Duration(rand.Int64N(int64(base)))
}
