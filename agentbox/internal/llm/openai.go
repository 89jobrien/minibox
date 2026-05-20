package llm

import (
	"context"
	"fmt"
	"os"

	"github.com/openai/openai-go"
	"github.com/openai/openai-go/option"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// OpenAIProvider wraps the official OpenAI Go SDK.
type OpenAIProvider struct {
	client openai.Client
	model  string
}

// NewOpenAIProvider creates a provider with an explicit API key and model.
func NewOpenAIProvider(apiKey, model string) *OpenAIProvider {
	return &OpenAIProvider{
		client: openai.NewClient(option.WithAPIKey(apiKey)),
		model:  model,
	}
}

// NewOpenAIFromEnv creates a provider reading OPENAI_API_KEY from env.
// Uses the model from OPENAI_MODEL env var, defaulting to gpt-4.1-mini.
// Returns nil if the key is not set.
func NewOpenAIFromEnv() *OpenAIProvider {
	key := os.Getenv("OPENAI_API_KEY")
	if key == "" {
		return nil
	}
	model := os.Getenv("OPENAI_MODEL")
	if model == "" {
		model = "gpt-4.1-mini"
	}
	return NewOpenAIProvider(key, model)
}

func (p *OpenAIProvider) Name() string {
	return fmt.Sprintf("openai/%s", p.model)
}

func (p *OpenAIProvider) Complete(ctx context.Context, req domain.CompletionRequest) (domain.CompletionResponse, error) {
	maxTokens := int64(req.MaxTokens)
	if maxTokens == 0 {
		maxTokens = 4096
	}

	messages := []openai.ChatCompletionMessageParamUnion{
		openai.UserMessage(req.Prompt),
	}

	params := openai.ChatCompletionNewParams{
		Model:     p.model,
		Messages:  messages,
		MaxTokens: openai.Int(maxTokens),
	}
	if req.System != "" {
		params.Messages = append(
			[]openai.ChatCompletionMessageParamUnion{openai.SystemMessage(req.System)},
			params.Messages...,
		)
	}

	resp, err := p.client.Chat.Completions.New(ctx, params)
	if err != nil {
		return domain.CompletionResponse{}, fmt.Errorf("openai: %w", err)
	}

	var text string
	if len(resp.Choices) > 0 {
		text = resp.Choices[0].Message.Content
	}

	return domain.CompletionResponse{
		Text:     text,
		Provider: p.Name(),
	}, nil
}
