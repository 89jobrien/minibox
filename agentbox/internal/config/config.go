package config

import "os"

// Config centralizes all LLM provider configuration.
type Config struct {
	OpenAIKey   string
	OpenAIModel string

	GeminiKey   string
	GeminiModel string
}

// LoadFromEnv reads provider configuration from environment variables,
// applying defaults for model names.
func LoadFromEnv() Config {
	cfg := Config{
		OpenAIKey:   os.Getenv("OPENAI_API_KEY"),
		OpenAIModel: os.Getenv("OPENAI_MODEL"),
		GeminiKey:   os.Getenv("GEMINI_API_KEY"),
		GeminiModel: os.Getenv("GEMINI_MODEL"),
	}
	if cfg.OpenAIModel == "" {
		cfg.OpenAIModel = "gpt-4.1-mini"
	}
	if cfg.GeminiModel == "" {
		cfg.GeminiModel = "gemini-2.5-flash-lite"
	}
	return cfg
}
