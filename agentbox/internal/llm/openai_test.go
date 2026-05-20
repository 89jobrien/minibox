package llm

import (
	"testing"

	"github.com/joe/minibox/agentbox/internal/config"
)

func TestNewOpenAIFromConfigNilWhenKeyEmpty(t *testing.T) {
	p := NewOpenAIFromConfig(config.Config{OpenAIKey: "", OpenAIModel: "gpt-4o"})
	if p != nil {
		t.Error("expected nil when API key is empty")
	}
}

func TestNewOpenAIFromConfigNonNilWhenKeySet(t *testing.T) {
	p := NewOpenAIFromConfig(config.Config{OpenAIKey: "sk-test", OpenAIModel: "gpt-4o"})
	if p == nil {
		t.Fatal("expected non-nil provider")
	}
	if p.model != "gpt-4o" {
		t.Errorf("model = %q, want gpt-4o", p.model)
	}
}

func TestOpenAIProviderName(t *testing.T) {
	p := NewOpenAIProvider("sk-test", "gpt-4o-mini")
	if p.Name() != "openai/gpt-4o-mini" {
		t.Errorf("name = %q", p.Name())
	}
}
