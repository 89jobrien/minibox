package agent

import (
	"context"
	"errors"
	"fmt"

	claudecode "github.com/severity1/claude-agent-sdk-go"
	"github.com/joe/minibox/agentbox/internal/domain"
)

// ClaudeSDKRunner runs agents via the Claude Agent SDK (CLI subprocess).
type ClaudeSDKRunner struct{}

// NewClaudeSDKRunner creates a new SDK-based agent runner.
func NewClaudeSDKRunner() *ClaudeSDKRunner {
	return &ClaudeSDKRunner{}
}

func configToQueryOptions(config domain.AgentConfig) []claudecode.Option {
	var opts []claudecode.Option
	if len(config.Tools) > 0 {
		opts = append(opts, claudecode.WithAllowedTools(config.Tools...))
	}
	if config.SystemPrompt != "" {
		opts = append(opts, claudecode.WithSystemPrompt(config.SystemPrompt))
	}
	return opts
}

func (r *ClaudeSDKRunner) Run(ctx context.Context, config domain.AgentConfig) (domain.AgentResult, error) {
	opts := configToQueryOptions(config)

	iter, err := claudecode.Query(ctx, config.Prompt, opts...)
	if err != nil {
		return domain.AgentResult{Name: config.Name, Error: err.Error()}, fmt.Errorf("sdk query: %w", err)
	}
	defer iter.Close()

	var parts []string
	for {
		msg, err := iter.Next(ctx)
		if errors.Is(err, claudecode.ErrNoMoreMessages) {
			break
		}
		if err != nil {
			return domain.AgentResult{Name: config.Name, Error: err.Error()}, fmt.Errorf("sdk next: %w", err)
		}
		if result, ok := msg.(*claudecode.ResultMessage); ok {
			if result.Result != nil {
				parts = append(parts, *result.Result)
			}
		}
	}

	output := ""
	for i, p := range parts {
		if i > 0 {
			output += "\n"
		}
		output += p
	}

	return domain.AgentResult{Name: config.Name, Output: output}, nil
}
