package domain

import "context"

// AgentRunner executes an agent with config and returns results.
type AgentRunner interface {
	Run(ctx context.Context, config AgentConfig) (AgentResult, error)
}

// LlmProvider sends a single LLM completion request.
type LlmProvider interface {
	Name() string
	Complete(ctx context.Context, req CompletionRequest) (CompletionResponse, error)
}

// CompletionRequest is the input to an LLM provider.
type CompletionRequest struct {
	Prompt    string `json:"prompt"`
	System    string `json:"system,omitempty"`
	MaxTokens int    `json:"max_tokens"`
}

// CompletionResponse is the output from an LLM provider.
type CompletionResponse struct {
	Text     string `json:"text"`
	Provider string `json:"provider"`
}

// MessageBroker provides pub/sub messaging.
type MessageBroker interface {
	Publish(ctx context.Context, topic string, msg Message) error
	Subscribe(ctx context.Context, topic string) (<-chan Message, error)
	Close() error
}

// ContextProvider gathers repository context.
type ContextProvider interface {
	GitLog(ctx context.Context, n int) ([]Commit, error)
	Diff(ctx context.Context, base string) (string, error)
	ProjectRules(ctx context.Context) (ProjectContext, error)
	BranchContext(ctx context.Context, base string) (string, error)
}

// ResultWriter persists agent results.
type ResultWriter interface {
	WriteRun(ctx context.Context, run AgentRun) error
	WriteReport(ctx context.Context, report AgentReport) error
}
