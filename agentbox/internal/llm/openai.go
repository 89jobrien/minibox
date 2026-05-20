package llm

import (
	"context"
	"fmt"

	"github.com/openai/openai-go"
	"github.com/openai/openai-go/option"

	"github.com/joe/minibox/agentbox/internal/config"
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

// NewOpenAIFromConfig creates a provider from a centralized Config.
// Returns nil if the API key is not set.
func NewOpenAIFromConfig(cfg config.Config) *OpenAIProvider {
	if cfg.OpenAIKey == "" {
		return nil
	}
	return NewOpenAIProvider(cfg.OpenAIKey, cfg.OpenAIModel)
}

// NewOpenAIFromEnv creates a provider reading config from environment.
// Returns nil if the key is not set.
func NewOpenAIFromEnv() *OpenAIProvider {
	return NewOpenAIFromConfig(config.LoadFromEnv())
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
