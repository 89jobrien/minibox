package llm

import (
	"context"
	"errors"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

func TestFallbackChainFirstSuccess(t *testing.T) {
	chain := NewFallbackChain(
		&mockProvider{name: "primary", resp: domain.CompletionResponse{Text: "hello", Provider: "primary"}},
		&mockProvider{name: "fallback", resp: domain.CompletionResponse{Text: "world", Provider: "fallback"}},
	)

	resp, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.Provider != "primary" {
		t.Errorf("provider = %q, want primary", resp.Provider)
	}
	if resp.Text != "hello" {
		t.Errorf("text = %q, want hello", resp.Text)
	}
}

func TestFallbackChainFallsThrough(t *testing.T) {
	chain := NewFallbackChain(
		&mockProvider{name: "broken", err: errors.New("rate limited")},
		&mockProvider{name: "backup", resp: domain.CompletionResponse{Text: "ok", Provider: "backup"}},
	)

	resp, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.Provider != "backup" {
		t.Errorf("provider = %q, want backup", resp.Provider)
	}
}

func TestFallbackChainAllFail(t *testing.T) {
	chain := NewFallbackChain(
		&mockProvider{name: "a", err: errors.New("fail a")},
		&mockProvider{name: "b", err: errors.New("fail b")},
	)

	_, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error when all providers fail")
	}
}

func TestFallbackChainEmpty(t *testing.T) {
	chain := NewFallbackChain()
	_, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error with empty chain")
	}
}

func TestFallbackChainSkipsNilProviders(t *testing.T) {
	chain := NewFallbackChain(
		nil,
		&mockProvider{name: "real", resp: domain.CompletionResponse{Text: "ok", Provider: "real"}},
	)
	resp, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.Provider != "real" {
		t.Errorf("provider = %q, want real", resp.Provider)
	}
}
