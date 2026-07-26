package config

import (
	"os"
	"testing"
)

func TestLoadFromEnvDefaults(t *testing.T) {
	// Clear env to test defaults
	os.Unsetenv("OPENAI_API_KEY")
	os.Unsetenv("OPENAI_MODEL")
	os.Unsetenv("GEMINI_API_KEY")
	os.Unsetenv("GEMINI_MODEL")

	cfg := LoadFromEnv()
	if cfg.OpenAIKey != "" {
		t.Errorf("OpenAIKey = %q, want empty", cfg.OpenAIKey)
	}
	if cfg.OpenAIModel != "gpt-4.1-mini" {
		t.Errorf("OpenAIModel = %q, want gpt-4.1-mini", cfg.OpenAIModel)
	}
	if cfg.GeminiModel != "gemini-2.5-flash-lite" {
		t.Errorf("GeminiModel = %q, want gemini-2.5-flash-lite", cfg.GeminiModel)
	}
}

func TestLoadFromEnvCustomValues(t *testing.T) {
	os.Setenv("OPENAI_API_KEY", "sk-test")
	os.Setenv("OPENAI_MODEL", "gpt-5")
	os.Setenv("GEMINI_API_KEY", "gem-test")
	os.Setenv("GEMINI_MODEL", "gemini-pro")
	defer func() {
		os.Unsetenv("OPENAI_API_KEY")
		os.Unsetenv("OPENAI_MODEL")
		os.Unsetenv("GEMINI_API_KEY")
		os.Unsetenv("GEMINI_MODEL")
	}()

	cfg := LoadFromEnv()
	if cfg.OpenAIKey != "sk-test" {
		t.Errorf("OpenAIKey = %q, want sk-test", cfg.OpenAIKey)
	}
	if cfg.OpenAIModel != "gpt-5" {
		t.Errorf("OpenAIModel = %q, want gpt-5", cfg.OpenAIModel)
	}
	if cfg.GeminiKey != "gem-test" {
		t.Errorf("GeminiKey = %q, want gem-test", cfg.GeminiKey)
	}
	if cfg.GeminiModel != "gemini-pro" {
		t.Errorf("GeminiModel = %q, want gemini-pro", cfg.GeminiModel)
	}
}
