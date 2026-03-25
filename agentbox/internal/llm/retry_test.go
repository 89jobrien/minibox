package llm

import (
	"context"
	"errors"
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
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 3, BackoffBase: time.Millisecond})

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
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 2, BackoffBase: time.Millisecond})

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
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 3, BackoffBase: time.Millisecond})

	resp, _ := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if inner.calls.Load() != 1 {
		t.Errorf("calls = %d, want 1", inner.calls.Load())
	}
	if resp.Text != "fast" {
		t.Errorf("text = %q, want fast", resp.Text)
	}
}
