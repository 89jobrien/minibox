package llm

import (
	"context"
	"fmt"
	"os"

	"github.com/anthropics/anthropic-sdk-go"
	"github.com/anthropics/anthropic-sdk-go/option"
	"github.com/joe/minibox/agentbox/internal/domain"
)

// AnthropicProvider wraps the official Anthropic Go SDK.
type AnthropicProvider struct {
	client anthropic.Client
	model  anthropic.Model
}

// NewAnthropicProvider creates a provider with an explicit API key.
func NewAnthropicProvider(apiKey string) *AnthropicProvider {
	return &AnthropicProvider{
		client: anthropic.NewClient(option.WithAPIKey(apiKey)),
		model:  anthropic.ModelClaudeSonnet4_6,
	}
}

// NewAnthropicFromEnv creates a provider reading ANTHROPIC_API_KEY from env.
// Returns nil if the key is not set.
func NewAnthropicFromEnv() *AnthropicProvider {
	key := os.Getenv("ANTHROPIC_API_KEY")
	if key == "" {
		return nil
	}
	return NewAnthropicProvider(key)
}

func (p *AnthropicProvider) Name() string {
	return fmt.Sprintf("anthropic/%s", p.model)
}

func (p *AnthropicProvider) Complete(ctx context.Context, req domain.CompletionRequest) (domain.CompletionResponse, error) {
	maxTokens := int64(req.MaxTokens)
	if maxTokens == 0 {
		maxTokens = 1024
	}

	params := anthropic.MessageNewParams{
		Model:     p.model,
		MaxTokens: maxTokens,
		Messages: []anthropic.MessageParam{
			anthropic.NewUserMessage(anthropic.NewTextBlock(req.Prompt)),
		},
	}
	if req.System != "" {
		params.System = []anthropic.TextBlockParam{
			{Text: req.System},
		}
	}

	msg, err := p.client.Messages.New(ctx, params)
	if err != nil {
		return domain.CompletionResponse{}, fmt.Errorf("anthropic: %w", err)
	}

	var text string
	for _, block := range msg.Content {
		if block.Type == "text" {
			text += block.Text
		}
	}

	return domain.CompletionResponse{
		Text:     text,
		Provider: p.Name(),
	}, nil
}
