package llm

import (
	"context"
	"testing"

	"github.com/joe/minibox/agentbox/internal/config"
)

func TestNewGeminiFromConfigNilWhenKeyEmpty(t *testing.T) {
	p := NewGeminiFromConfig(context.Background(), config.Config{GeminiKey: "", GeminiModel: "gemini-2.5-flash"})
	if p != nil {
		t.Error("expected nil when API key is empty")
	}
}

func TestNewGeminiFromConfigNonNilWhenKeySet(t *testing.T) {
	p := NewGeminiFromConfig(context.Background(), config.Config{GeminiKey: "test-key", GeminiModel: "gemini-2.5-flash"})
	if p == nil {
		t.Fatal("expected non-nil provider")
	}
	if p.model != "gemini-2.5-flash" {
		t.Errorf("model = %q, want gemini-2.5-flash", p.model)
	}
}

func TestGeminiProviderName(t *testing.T) {
	p, err := NewGeminiProvider(context.Background(), "test-key", "gemini-2.5-pro")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if p.Name() != "gemini/gemini-2.5-pro" {
		t.Errorf("name = %q", p.Name())
	}
}
