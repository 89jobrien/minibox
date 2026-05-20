package llm

import (
	"context"
	"fmt"
	"os"

	"google.golang.org/genai"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// GeminiProvider wraps the Google GenAI Go SDK.
type GeminiProvider struct {
	client *genai.Client
	model  string
}

// NewGeminiProvider creates a provider with an explicit API key and model.
func NewGeminiProvider(ctx context.Context, apiKey, model string) (*GeminiProvider, error) {
	client, err := genai.NewClient(ctx, &genai.ClientConfig{
		APIKey:  apiKey,
		Backend: genai.BackendGeminiAPI,
	})
	if err != nil {
		return nil, fmt.Errorf("gemini client: %w", err)
	}
	return &GeminiProvider{client: client, model: model}, nil
}

// NewGeminiFromEnv creates a provider reading GEMINI_API_KEY from env.
// Uses the model from GEMINI_MODEL env var, defaulting to gemini-2.5-flash-lite.
// Returns nil if the key is not set.
func NewGeminiFromEnv(ctx context.Context) *GeminiProvider {
	key := os.Getenv("GEMINI_API_KEY")
	if key == "" {
		return nil
	}
	model := os.Getenv("GEMINI_MODEL")
	if model == "" {
		model = "gemini-2.5-flash-lite"
	}
	p, err := NewGeminiProvider(ctx, key, model)
	if err != nil {
		fmt.Fprintf(os.Stderr, "warning: gemini provider init failed: %v\n", err)
		return nil
	}
	return p
}

func (p *GeminiProvider) Name() string {
	return fmt.Sprintf("gemini/%s", p.model)
}

func (p *GeminiProvider) Complete(ctx context.Context, req domain.CompletionRequest) (domain.CompletionResponse, error) {
	config := &genai.GenerateContentConfig{}
	if req.MaxTokens > 0 {
		maxTokens := int32(req.MaxTokens)
		config.MaxOutputTokens = maxTokens
	}
	if req.System != "" {
		config.SystemInstruction = &genai.Content{
			Parts: []*genai.Part{genai.NewPartFromText(req.System)},
		}
	}

	resp, err := p.client.Models.GenerateContent(ctx, p.model, genai.Text(req.Prompt), config)
	if err != nil {
		return domain.CompletionResponse{}, fmt.Errorf("gemini: %w", err)
	}

	var text string
	if resp != nil && len(resp.Candidates) > 0 && resp.Candidates[0].Content != nil {
		for _, part := range resp.Candidates[0].Content.Parts {
			if part.Text != "" {
				text += part.Text
			}
		}
	}

	return domain.CompletionResponse{
		Text:     text,
		Provider: p.Name(),
	}, nil
}
