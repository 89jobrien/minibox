package llm

import (
	"context"
	"errors"
	"fmt"
	"sync/atomic"
	"testing"
	"time"

	"github.com/joe/minibox/agentbox/internal/domain"
)

type countingProvider struct {
	calls atomic.Int32
	failN int
	resp  domain.CompletionResponse
}

func (p *countingProvider) Name() string { return "counting" }
func (p *countingProvider) Complete(_ context.Context, _ domain.CompletionRequest) (domain.CompletionResponse, error) {
	n := int(p.calls.Add(1))
	if n <= p.failN {
		return domain.CompletionResponse{}, errors.New("transient error")
	}
	return p.resp, nil
}

func TestRetryingProviderRetriesOnFailure(t *testing.T) {
	inner := &countingProvider{failN: 2, resp: domain.CompletionResponse{Text: "ok", Provider: "counting"}}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 3, BackoffBase: time.Millisecond, MaxDelay: time.Second})

	resp, err := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.Text != "ok" {
		t.Errorf("text = %q, want ok", resp.Text)
	}
	if inner.calls.Load() != 3 {
		t.Errorf("calls = %d, want 3", inner.calls.Load())
	}
}

func TestRetryingProviderExhaustsRetries(t *testing.T) {
	inner := &countingProvider{failN: 10, resp: domain.CompletionResponse{Text: "ok"}}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 2, BackoffBase: time.Millisecond, MaxDelay: time.Second})

	_, err := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error when retries exhausted")
	}
	if inner.calls.Load() != 3 { // 1 initial + 2 retries
		t.Errorf("calls = %d, want 3", inner.calls.Load())
	}
}

func TestRetryingProviderSucceedsImmediately(t *testing.T) {
	inner := &countingProvider{failN: 0, resp: domain.CompletionResponse{Text: "fast"}}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 3, BackoffBase: time.Millisecond, MaxDelay: time.Second})

	resp, _ := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if inner.calls.Load() != 1 {
		t.Errorf("calls = %d, want 1", inner.calls.Load())
	}
	if resp.Text != "fast" {
		t.Errorf("text = %q, want fast", resp.Text)
	}
}

type httpErrorProvider struct {
	calls      atomic.Int32
	statusCode int
}

func (p *httpErrorProvider) Name() string { return "http-error" }
func (p *httpErrorProvider) Complete(_ context.Context, _ domain.CompletionRequest) (domain.CompletionResponse, error) {
	p.calls.Add(1)
	return domain.CompletionResponse{}, &HTTPStatusError{
		StatusCode: p.statusCode,
		Err:        fmt.Errorf("api error"),
	}
}

func TestRetryingProviderRetries429(t *testing.T) {
	inner := &httpErrorProvider{statusCode: 429}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 2, BackoffBase: time.Millisecond, MaxDelay: time.Second})

	_, err := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error")
	}
	// Should have retried: 1 initial + 2 retries = 3
	if inner.calls.Load() != 3 {
		t.Errorf("calls = %d, want 3 (429 should be retried)", inner.calls.Load())
	}
}

func TestRetryingProviderRetries500(t *testing.T) {
	inner := &httpErrorProvider{statusCode: 500}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 1, BackoffBase: time.Millisecond, MaxDelay: time.Second})

	_, err := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error")
	}
	if inner.calls.Load() != 2 {
		t.Errorf("calls = %d, want 2 (500 should be retried)", inner.calls.Load())
	}
}

func TestRetryingProviderDoesNotRetry400(t *testing.T) {
	inner := &httpErrorProvider{statusCode: 400}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 3, BackoffBase: time.Millisecond, MaxDelay: time.Second})

	_, err := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error")
	}
	// 400 is not retryable, so only 1 call
	if inner.calls.Load() != 1 {
		t.Errorf("calls = %d, want 1 (400 should not be retried)", inner.calls.Load())
	}
}

func TestIsRetryable(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want bool
	}{
		{"nil", nil, false},
		{"plain error", errors.New("oops"), true},
		{"429", &HTTPStatusError{StatusCode: 429, Err: errors.New("rate limit")}, true},
		{"500", &HTTPStatusError{StatusCode: 500, Err: errors.New("server")}, true},
		{"502", &HTTPStatusError{StatusCode: 502, Err: errors.New("bad gateway")}, true},
		{"400", &HTTPStatusError{StatusCode: 400, Err: errors.New("bad request")}, false},
		{"401", &HTTPStatusError{StatusCode: 401, Err: errors.New("unauthorized")}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := IsRetryable(tt.err)
			if got != tt.want {
				t.Errorf("IsRetryable(%v) = %v, want %v", tt.err, got, tt.want)
			}
		})
	}
}

func TestBackoffWithJitterBounded(t *testing.T) {
	p := &RetryingProvider{
		config: RetryConfig{
			BackoffBase: 100 * time.Millisecond,
			MaxDelay:    500 * time.Millisecond,
		},
	}
	for attempt := 1; attempt <= 10; attempt++ {
		d := p.backoffWithJitter(attempt)
		if d < 0 {
			t.Errorf("attempt %d: negative delay %v", attempt, d)
		}
		if d > 500*time.Millisecond {
			t.Errorf("attempt %d: delay %v exceeds max 500ms", attempt, d)
		}
	}
}

func TestContextCancellation(t *testing.T) {
	inner := &countingProvider{failN: 100, resp: domain.CompletionResponse{}}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 10, BackoffBase: time.Second, MaxDelay: 10 * time.Second})

	ctx, cancel := context.WithCancel(context.Background())
	cancel() // cancel immediately

	_, err := p.Complete(ctx, domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error on cancelled context")
	}
}
