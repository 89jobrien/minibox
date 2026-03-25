# Agentbox Tier A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the agentbox Go module with orchestration agents (council, meta-agent) and one tool agent (commit-msg), using the Claude Agent SDK and native Go LLM client.

**Architecture:** Hexagonal — domain interfaces define ports (LlmProvider, MessageBroker, AgentRunner, ContextProvider, ResultWriter), adapters implement them, composition root wires everything. Agents use Claude Agent SDK (CLI subprocess) for agentic workflows and a native Go LLM client (wrapping official Anthropic SDK) for simple completions. Pub/sub is Go channels.

**Tech Stack:** Go 1.22+, `github.com/anthropics/anthropic-sdk-go`, `github.com/severity1/claude-agent-sdk-go`, standard library

---

## File Map

### Foundation (Tasks 1–4)

| File | Responsibility |
|------|---------------|
| `agentbox/go.mod` | Go module definition |
| `agentbox/internal/domain/types.go` | Domain types: Message, AgentConfig, AgentResult, AgentRun, AgentReport, Commit, ProjectContext |
| `agentbox/internal/domain/interfaces.go` | Port interfaces: LlmProvider, MessageBroker, AgentRunner, ContextProvider, ResultWriter |
| `agentbox/internal/domain/types_test.go` | Tests for domain type serialization |

### LLM Layer (Tasks 5–7)

| File | Responsibility |
|------|---------------|
| `agentbox/internal/llm/provider.go` | CompletionRequest/CompletionResponse types, LlmProvider re-export |
| `agentbox/internal/llm/anthropic.go` | AnthropicProvider wrapping official SDK |
| `agentbox/internal/llm/anthropic_test.go` | Unit test with mock HTTP |
| `agentbox/internal/llm/chain.go` | FallbackChain: sequential provider fallback |
| `agentbox/internal/llm/chain_test.go` | Tests with mock providers |
| `agentbox/internal/llm/retry.go` | RetryingProvider: exponential backoff |
| `agentbox/internal/llm/retry_test.go` | Tests for retry logic |

### Pub/Sub (Task 8)

| File | Responsibility |
|------|---------------|
| `agentbox/internal/pubsub/broker.go` | ChannelBroker: in-process Go channel broker |
| `agentbox/internal/pubsub/message.go` | Message envelope type + JSONL serialization |
| `agentbox/internal/pubsub/broker_test.go` | Tests for pub/sub routing, dynamic topics, close behavior |

### Context & Output (Tasks 9–10)

| File | Responsibility |
|------|---------------|
| `agentbox/internal/context/git.go` | GitContextProvider: git log, diff, branch, project rules |
| `agentbox/internal/context/git_test.go` | Tests with fixture data |
| `agentbox/internal/output/jsonl.go` | JSONL writer for ~/.mbx/agent-runs.jsonl |
| `agentbox/internal/output/report.go` | Markdown report writer for ~/.mbx/ai-logs/ |
| `agentbox/internal/output/dual.go` | DualWriter: pub/sub + file output |
| `agentbox/internal/output/jsonl_test.go` | Tests for JSONL format compatibility |

### Agent SDK Wrapper (Task 11)

| File | Responsibility |
|------|---------------|
| `agentbox/internal/agent/sdk.go` | ClaudeSDKRunner wrapping severity1/claude-agent-sdk-go |
| `agentbox/internal/agent/sdk_test.go` | Tests with mock iterator |

### Orchestration Agents (Tasks 12–13)

| File | Responsibility |
|------|---------------|
| `agentbox/internal/orchestrator/council.go` | Council agent: multi-role analysis + synthesis |
| `agentbox/internal/orchestrator/council_test.go` | Tests with mock AgentRunner |
| `agentbox/internal/orchestrator/meta.go` | Meta-agent: design + spawn + synthesize |
| `agentbox/internal/orchestrator/meta_test.go` | Tests with mock AgentRunner + MessageBroker |

### Tool Agent (Task 14)

| File | Responsibility |
|------|---------------|
| `agentbox/internal/tools/commitmsg.go` | Commit message generator |
| `agentbox/internal/tools/commitmsg_test.go` | Tests with mock context/runner |

### Binaries & Integration (Tasks 15–16)

| File | Responsibility |
|------|---------------|
| `agentbox/cmd/agentbox/main.go` | Orchestration binary: subcommand dispatch |
| `agentbox/cmd/mbx-commit-msg/main.go` | Standalone commit-msg binary |

---

## Task 1: Go Module Initialization

**Files:**
- Create: `agentbox/go.mod`
- Create: `agentbox/internal/domain/types.go`
- Create: `agentbox/internal/domain/interfaces.go`

- [ ] **Step 1: Initialize Go module**

```bash
cd /Users/joe/dev/minibox
mkdir -p agentbox
cd agentbox
go mod init github.com/joe/minibox/agentbox
```

- [ ] **Step 2: Create domain types**

Create `agentbox/internal/domain/types.go`:

```go
package domain

import (
	"encoding/json"
	"time"
)

// Message is the pub/sub envelope for all inter-agent communication.
type Message struct {
	Source        string          `json:"source"`
	Timestamp    time.Time       `json:"timestamp"`
	Topic        string          `json:"topic"`
	SchemaVersion int            `json:"schema_version"`
	Payload      json.RawMessage `json:"payload"`
}

// AgentConfig is the input to an AgentRunner.
type AgentConfig struct {
	Name         string   `json:"name"`
	Role         string   `json:"role"`
	Prompt       string   `json:"prompt"`
	SystemPrompt string   `json:"system_prompt,omitempty"`
	Tools        []string `json:"tools"`
}

// AgentResult is the output from an AgentRunner.
type AgentResult struct {
	Name   string `json:"name"`
	Output string `json:"output"`
	Error  string `json:"error,omitempty"`
}

// AgentRun is the telemetry record written to agent-runs.jsonl.
type AgentRun struct {
	RunID     string         `json:"run_id"`
	Script    string         `json:"script"`
	Args      map[string]any `json:"args"`
	Status    string         `json:"status"`
	DurationS float64       `json:"duration_s,omitempty"`
	Output    string         `json:"output,omitempty"`
}

// AgentReport is the markdown report written to ai-logs/.
type AgentReport struct {
	SHA     string
	Script  string
	Content string
	Meta    map[string]string
}

// Commit is a parsed git log entry.
type Commit struct {
	SHA     string `json:"sha"`
	Message string `json:"message"`
}

// ProjectContext is discovered project metadata.
type ProjectContext struct {
	Rules     string `json:"rules"`
	Branch    string `json:"branch"`
	GitLog    string `json:"git_log"`
	GitStatus string `json:"git_status"`
	GitStat   string `json:"git_stat"`
	Structure string `json:"structure"`
}
```

- [ ] **Step 3: Create domain interfaces**

Create `agentbox/internal/domain/interfaces.go`:

```go
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
```

- [ ] **Step 4: Verify module compiles**

```bash
cd /Users/joe/dev/minibox/agentbox
go build ./...
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
cd /Users/joe/dev/minibox
git add agentbox/
git commit -m "feat(agentbox): initialize Go module with domain types and interfaces"
```

---

## Task 2: Domain Type Tests

**Files:**
- Create: `agentbox/internal/domain/types_test.go`

- [ ] **Step 1: Write tests for Message serialization**

```go
package domain

import (
	"encoding/json"
	"testing"
	"time"
)

func TestMessageRoundTrip(t *testing.T) {
	msg := Message{
		Source:        "council-critic",
		Timestamp:     time.Date(2026, 3, 25, 12, 0, 0, 0, time.UTC),
		Topic:         "result.council.abc123",
		SchemaVersion: 1,
		Payload:       json.RawMessage(`{"score":0.82}`),
	}

	data, err := json.Marshal(msg)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}

	var got Message
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}

	if got.Source != msg.Source {
		t.Errorf("source = %q, want %q", got.Source, msg.Source)
	}
	if got.Topic != msg.Topic {
		t.Errorf("topic = %q, want %q", got.Topic, msg.Topic)
	}
	if got.SchemaVersion != 1 {
		t.Errorf("schema_version = %d, want 1", got.SchemaVersion)
	}
}

func TestAgentRunJSONL(t *testing.T) {
	run := AgentRun{
		RunID:  "2026-03-25T12:00:00",
		Script: "council",
		Args:   map[string]any{"base": "main", "mode": "core"},
		Status: "running",
	}

	data, err := json.Marshal(run)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}

	// Verify it contains expected fields (JSONL compat with Python agent_log)
	var m map[string]any
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("unmarshal map: %v", err)
	}
	if m["run_id"] != "2026-03-25T12:00:00" {
		t.Errorf("run_id = %v, want 2026-03-25T12:00:00", m["run_id"])
	}
	if m["script"] != "council" {
		t.Errorf("script = %v, want council", m["script"])
	}
	if m["status"] != "running" {
		t.Errorf("status = %v, want running", m["status"])
	}
}
```

- [ ] **Step 2: Run tests**

```bash
cd /Users/joe/dev/minibox/agentbox
go test ./internal/domain/ -v
```

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add agentbox/internal/domain/types_test.go
git commit -m "test(agentbox): add domain type serialization tests"
```

---

## Task 3: Pub/Sub Core

**Files:**
- Create: `agentbox/internal/pubsub/message.go`
- Create: `agentbox/internal/pubsub/broker.go`
- Create: `agentbox/internal/pubsub/broker_test.go`

- [ ] **Step 1: Write broker tests**

```go
package pubsub

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/joe/minibox/agentbox/internal/domain"
)

func TestChannelBrokerPublishSubscribe(t *testing.T) {
	b := NewChannelBroker()
	defer b.Close()

	ctx := context.Background()
	ch, err := b.Subscribe(ctx, "result.council.test1")
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}

	msg := domain.Message{
		Source:        "test",
		Timestamp:     time.Now(),
		Topic:         "result.council.test1",
		SchemaVersion: 1,
		Payload:       json.RawMessage(`{"score":0.9}`),
	}

	if err := b.Publish(ctx, "result.council.test1", msg); err != nil {
		t.Fatalf("publish: %v", err)
	}

	select {
	case got := <-ch:
		if got.Source != "test" {
			t.Errorf("source = %q, want %q", got.Source, "test")
		}
	case <-time.After(time.Second):
		t.Fatal("timeout waiting for message")
	}
}

func TestChannelBrokerDynamicTopics(t *testing.T) {
	b := NewChannelBroker()
	defer b.Close()

	ctx := context.Background()

	// Publishing to a topic with no subscribers should not error
	msg := domain.Message{Source: "test", Timestamp: time.Now(), Topic: "nobody.listening", SchemaVersion: 1}
	if err := b.Publish(ctx, "nobody.listening", msg); err != nil {
		t.Fatalf("publish to empty topic: %v", err)
	}
}

func TestChannelBrokerMultipleSubscribers(t *testing.T) {
	b := NewChannelBroker()
	defer b.Close()

	ctx := context.Background()
	ch1, _ := b.Subscribe(ctx, "shared.topic")
	ch2, _ := b.Subscribe(ctx, "shared.topic")

	msg := domain.Message{Source: "test", Timestamp: time.Now(), Topic: "shared.topic", SchemaVersion: 1}
	b.Publish(ctx, "shared.topic", msg)

	for _, ch := range []<-chan domain.Message{ch1, ch2} {
		select {
		case got := <-ch:
			if got.Source != "test" {
				t.Errorf("source = %q, want %q", got.Source, "test")
			}
		case <-time.After(time.Second):
			t.Fatal("timeout waiting for message")
		}
	}
}

func TestChannelBrokerCloseChannels(t *testing.T) {
	b := NewChannelBroker()
	ctx := context.Background()
	ch, _ := b.Subscribe(ctx, "will.close")
	b.Close()

	_, ok := <-ch
	if ok {
		t.Error("expected channel to be closed after broker.Close()")
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /Users/joe/dev/minibox/agentbox
go test ./internal/pubsub/ -v
```

Expected: FAIL (package does not exist yet)

- [ ] **Step 3: Implement message.go**

```go
package pubsub

import (
	"encoding/json"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// MarshalMessage serializes a Message to a single JSONL line.
func MarshalMessage(msg domain.Message) ([]byte, error) {
	return json.Marshal(msg)
}

// UnmarshalMessage deserializes a JSONL line into a Message.
func UnmarshalMessage(data []byte) (domain.Message, error) {
	var msg domain.Message
	err := json.Unmarshal(data, &msg)
	return msg, err
}
```

- [ ] **Step 4: Implement broker.go**

```go
package pubsub

import (
	"context"
	"sync"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// ChannelBroker is an in-process pub/sub broker using Go channels.
type ChannelBroker struct {
	mu     sync.RWMutex
	topics map[string][]chan domain.Message
	closed bool
}

// NewChannelBroker creates a new in-process broker.
func NewChannelBroker() *ChannelBroker {
	return &ChannelBroker{
		topics: make(map[string][]chan domain.Message),
	}
}

func (b *ChannelBroker) Publish(_ context.Context, topic string, msg domain.Message) error {
	b.mu.RLock()
	defer b.mu.RUnlock()
	if b.closed {
		return nil
	}
	for _, ch := range b.topics[topic] {
		select {
		case ch <- msg:
		default:
			// Drop message if subscriber is slow — avoid blocking publisher
		}
	}
	return nil
}

func (b *ChannelBroker) Subscribe(_ context.Context, topic string) (<-chan domain.Message, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	ch := make(chan domain.Message, 64)
	b.topics[topic] = append(b.topics[topic], ch)
	return ch, nil
}

func (b *ChannelBroker) Close() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.closed = true
	for _, subs := range b.topics {
		for _, ch := range subs {
			close(ch)
		}
	}
	b.topics = nil
	return nil
}
```

- [ ] **Step 5: Run tests**

```bash
go test ./internal/pubsub/ -v
```

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add agentbox/internal/pubsub/
git commit -m "feat(agentbox): add pub/sub core with ChannelBroker"
```

---

## Task 4: Output Writers

**Files:**
- Create: `agentbox/internal/output/jsonl.go`
- Create: `agentbox/internal/output/report.go`
- Create: `agentbox/internal/output/dual.go`
- Create: `agentbox/internal/output/jsonl_test.go`

- [ ] **Step 1: Write JSONL writer tests**

```go
package output

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

func TestJSONLWriterFormat(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "agent-runs.jsonl")
	w := NewJSONLWriter(path)

	run := domain.AgentRun{
		RunID:  "2026-03-25T12:00:00",
		Script: "council",
		Args:   map[string]any{"base": "main"},
		Status: "running",
	}
	if err := w.WriteRun(context.Background(), run); err != nil {
		t.Fatalf("WriteRun: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile: %v", err)
	}

	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) != 1 {
		t.Fatalf("expected 1 line, got %d", len(lines))
	}

	var m map[string]any
	if err := json.Unmarshal([]byte(lines[0]), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if m["run_id"] != "2026-03-25T12:00:00" {
		t.Errorf("run_id = %v, want 2026-03-25T12:00:00", m["run_id"])
	}
	if m["script"] != "council" {
		t.Errorf("script = %v, want council", m["script"])
	}
}

func TestJSONLWriterAppends(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "agent-runs.jsonl")
	w := NewJSONLWriter(path)

	ctx := context.Background()
	w.WriteRun(ctx, domain.AgentRun{RunID: "1", Script: "a", Status: "running"})
	w.WriteRun(ctx, domain.AgentRun{RunID: "2", Script: "b", Status: "complete"})

	data, _ := os.ReadFile(path)
	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected 2 lines, got %d", len(lines))
	}
}

func TestReportWriter(t *testing.T) {
	dir := t.TempDir()
	w := NewReportWriter(dir)

	report := domain.AgentReport{
		SHA:     "abc1234",
		Script:  "council-core",
		Content: "## Findings\nAll good.",
		Meta:    map[string]string{"base": "main", "mode": "core"},
	}
	if err := w.WriteReport(context.Background(), report); err != nil {
		t.Fatalf("WriteReport: %v", err)
	}

	path := filepath.Join(dir, "abc1234-council-core.md")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ReadFile: %v", err)
	}

	content := string(data)
	if !strings.Contains(content, "# council-core") {
		t.Error("missing header")
	}
	if !strings.Contains(content, "abc1234") {
		t.Error("missing SHA in header")
	}
	if !strings.Contains(content, "## Findings") {
		t.Error("missing content body")
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/output/ -v
```

Expected: FAIL

- [ ] **Step 3: Implement jsonl.go**

```go
package output

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// JSONLWriter writes AgentRun records as JSONL.
type JSONLWriter struct {
	path string
}

// NewJSONLWriter creates a writer targeting the given file path.
func NewJSONLWriter(path string) *JSONLWriter {
	return &JSONLWriter{path: path}
}

func (w *JSONLWriter) WriteRun(_ context.Context, run domain.AgentRun) error {
	if err := os.MkdirAll(filepath.Dir(w.path), 0o755); err != nil {
		return err
	}
	f, err := os.OpenFile(w.path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return err
	}
	defer f.Close()
	data, err := json.Marshal(run)
	if err != nil {
		return err
	}
	_, err = f.Write(append(data, '\n'))
	return err
}
```

- [ ] **Step 4: Implement report.go**

```go
package output

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// ReportWriter writes markdown reports to a directory.
type ReportWriter struct {
	dir string
}

// NewReportWriter creates a writer targeting the given directory.
func NewReportWriter(dir string) *ReportWriter {
	return &ReportWriter{dir: dir}
}

func (w *ReportWriter) WriteReport(_ context.Context, report domain.AgentReport) error {
	if err := os.MkdirAll(w.dir, 0o755); err != nil {
		return err
	}
	path := filepath.Join(w.dir, fmt.Sprintf("%s-%s.md", report.SHA, report.Script))

	var metaLines []string
	for k, v := range report.Meta {
		metaLines = append(metaLines, fmt.Sprintf("- **%s**: %s", k, v))
	}
	header := fmt.Sprintf("# %s · %s\n\n%s\n\n---\n\n",
		report.Script, report.SHA, strings.Join(metaLines, "\n"))

	return os.WriteFile(path, []byte(header+report.Content), 0o644)
}
```

- [ ] **Step 5: Implement dual.go**

```go
package output

import (
	"context"
	"os"
	"path/filepath"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// DualWriter writes to both JSONL and markdown report sinks.
type DualWriter struct {
	jsonl  *JSONLWriter
	report *ReportWriter
}

// NewDualWriter creates a writer targeting ~/.mbx/ paths.
func NewDualWriter() *DualWriter {
	home, _ := os.UserHomeDir()
	return &DualWriter{
		jsonl:  NewJSONLWriter(filepath.Join(home, ".mbx", "agent-runs.jsonl")),
		report: NewReportWriter(filepath.Join(home, ".mbx", "ai-logs")),
	}
}

func (w *DualWriter) WriteRun(ctx context.Context, run domain.AgentRun) error {
	return w.jsonl.WriteRun(ctx, run)
}

func (w *DualWriter) WriteReport(ctx context.Context, report domain.AgentReport) error {
	return w.report.WriteReport(ctx, report)
}
```

- [ ] **Step 6: Run tests**

```bash
go test ./internal/output/ -v
```

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add agentbox/internal/output/
git commit -m "feat(agentbox): add JSONL and markdown output writers"
```

---

## Task 5: LLM Provider — Anthropic

**Files:**
- Create: `agentbox/internal/llm/provider.go`
- Create: `agentbox/internal/llm/anthropic.go`
- Create: `agentbox/internal/llm/anthropic_test.go`

- [ ] **Step 1: Add Anthropic SDK dependency**

```bash
cd /Users/joe/dev/minibox/agentbox
go get github.com/anthropics/anthropic-sdk-go
```

- [ ] **Step 2: Write Anthropic provider test**

```go
package llm

import (
	"context"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// mockProvider implements domain.LlmProvider for testing.
type mockProvider struct {
	name string
	resp domain.CompletionResponse
	err  error
}

func (m *mockProvider) Name() string { return m.name }
func (m *mockProvider) Complete(_ context.Context, _ domain.CompletionRequest) (domain.CompletionResponse, error) {
	return m.resp, m.err
}

func TestAnthropicProviderName(t *testing.T) {
	p := NewAnthropicProvider("test-key")
	if p.Name() != "anthropic/claude-sonnet-4-6" {
		t.Errorf("name = %q, want anthropic/claude-sonnet-4-6", p.Name())
	}
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
go test ./internal/llm/ -v -run TestAnthropicProviderName
```

Expected: FAIL

- [ ] **Step 4: Implement provider.go**

```go
package llm

// Re-export domain types for convenience within this package.
// Actual interface definition lives in domain/interfaces.go.
```

- [ ] **Step 5: Implement anthropic.go**

```go
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
	client *anthropic.Client
	model  anthropic.Model
}

// NewAnthropicProvider creates a provider with an explicit API key.
func NewAnthropicProvider(apiKey string) *AnthropicProvider {
	client := anthropic.NewClient(option.WithAPIKey(apiKey))
	return &AnthropicProvider{
		client: client,
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
			anthropic.NewTextBlock(req.System),
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
```

- [ ] **Step 6: Run test**

```bash
go test ./internal/llm/ -v -run TestAnthropicProviderName
```

Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add agentbox/internal/llm/
git commit -m "feat(agentbox): add Anthropic LLM provider wrapping official SDK"
```

---

## Task 6: FallbackChain

**Files:**
- Create: `agentbox/internal/llm/chain.go`
- Create: `agentbox/internal/llm/chain_test.go`

- [ ] **Step 1: Write chain tests**

```go
package llm

import (
	"context"
	"errors"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

func TestFallbackChainFirstSuccess(t *testing.T) {
	chain := NewFallbackChain(
		&mockProvider{name: "primary", resp: domain.CompletionResponse{Text: "hello", Provider: "primary"}},
		&mockProvider{name: "fallback", resp: domain.CompletionResponse{Text: "world", Provider: "fallback"}},
	)

	resp, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.Provider != "primary" {
		t.Errorf("provider = %q, want primary", resp.Provider)
	}
	if resp.Text != "hello" {
		t.Errorf("text = %q, want hello", resp.Text)
	}
}

func TestFallbackChainFallsThrough(t *testing.T) {
	chain := NewFallbackChain(
		&mockProvider{name: "broken", err: errors.New("rate limited")},
		&mockProvider{name: "backup", resp: domain.CompletionResponse{Text: "ok", Provider: "backup"}},
	)

	resp, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.Provider != "backup" {
		t.Errorf("provider = %q, want backup", resp.Provider)
	}
}

func TestFallbackChainAllFail(t *testing.T) {
	chain := NewFallbackChain(
		&mockProvider{name: "a", err: errors.New("fail a")},
		&mockProvider{name: "b", err: errors.New("fail b")},
	)

	_, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error when all providers fail")
	}
}

func TestFallbackChainEmpty(t *testing.T) {
	chain := NewFallbackChain()
	_, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error with empty chain")
	}
}

func TestFallbackChainSkipsNilProviders(t *testing.T) {
	chain := NewFallbackChain(
		nil,
		&mockProvider{name: "real", resp: domain.CompletionResponse{Text: "ok", Provider: "real"}},
	)
	resp, err := chain.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.Provider != "real" {
		t.Errorf("provider = %q, want real", resp.Provider)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/llm/ -v -run TestFallbackChain
```

Expected: FAIL

- [ ] **Step 3: Implement chain.go**

```go
package llm

import (
	"context"
	"fmt"
	"strings"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// FallbackChain tries providers in order, returning the first success.
type FallbackChain struct {
	providers []domain.LlmProvider
}

// NewFallbackChain creates a chain from the given providers. Nil providers are skipped.
func NewFallbackChain(providers ...domain.LlmProvider) *FallbackChain {
	var valid []domain.LlmProvider
	for _, p := range providers {
		if p != nil {
			valid = append(valid, p)
		}
	}
	return &FallbackChain{providers: valid}
}

func (c *FallbackChain) Name() string {
	names := make([]string, len(c.providers))
	for i, p := range c.providers {
		names[i] = p.Name()
	}
	return fmt.Sprintf("chain[%s]", strings.Join(names, ","))
}

func (c *FallbackChain) Complete(ctx context.Context, req domain.CompletionRequest) (domain.CompletionResponse, error) {
	if len(c.providers) == 0 {
		return domain.CompletionResponse{}, fmt.Errorf("no providers configured")
	}

	var errs []string
	for _, p := range c.providers {
		resp, err := p.Complete(ctx, req)
		if err == nil {
			return resp, nil
		}
		errs = append(errs, fmt.Sprintf("%s: %v", p.Name(), err))
	}
	return domain.CompletionResponse{}, fmt.Errorf("all providers failed: %s", strings.Join(errs, "; "))
}
```

- [ ] **Step 4: Run tests**

```bash
go test ./internal/llm/ -v -run TestFallbackChain
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add agentbox/internal/llm/chain.go agentbox/internal/llm/chain_test.go
git commit -m "feat(agentbox): add FallbackChain with sequential provider fallback"
```

---

## Task 7: RetryingProvider

**Files:**
- Create: `agentbox/internal/llm/retry.go`
- Create: `agentbox/internal/llm/retry_test.go`

- [ ] **Step 1: Write retry tests**

```go
package llm

import (
	"context"
	"errors"
	"sync/atomic"
	"testing"
	"time"

	"github.com/joe/minibox/agentbox/internal/domain"
)

type countingProvider struct {
	calls    atomic.Int32
	failN    int
	resp     domain.CompletionResponse
}

func (p *countingProvider) Name() string { return "counting" }
func (p *countingProvider) Complete(_ context.Context, _ domain.CompletionRequest) (domain.CompletionResponse, error) {
	n := int(p.calls.Add(1))
	if n <= p.failN {
		return domain.CompletionResponse{}, errors.New("transient error")
	}
	return p.resp, nil
}

func TestRetryingProviderRetriesOnFailure(t *testing.T) {
	inner := &countingProvider{failN: 2, resp: domain.CompletionResponse{Text: "ok", Provider: "counting"}}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 3, BackoffBase: time.Millisecond})

	resp, err := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err != nil {
		t.Fatalf("complete: %v", err)
	}
	if resp.Text != "ok" {
		t.Errorf("text = %q, want ok", resp.Text)
	}
	if inner.calls.Load() != 3 {
		t.Errorf("calls = %d, want 3", inner.calls.Load())
	}
}

func TestRetryingProviderExhaustsRetries(t *testing.T) {
	inner := &countingProvider{failN: 10, resp: domain.CompletionResponse{Text: "ok"}}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 2, BackoffBase: time.Millisecond})

	_, err := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if err == nil {
		t.Fatal("expected error when retries exhausted")
	}
	if inner.calls.Load() != 3 { // 1 initial + 2 retries
		t.Errorf("calls = %d, want 3", inner.calls.Load())
	}
}

func TestRetryingProviderSucceedsImmediately(t *testing.T) {
	inner := &countingProvider{failN: 0, resp: domain.CompletionResponse{Text: "fast"}}
	p := NewRetryingProvider(inner, RetryConfig{MaxRetries: 3, BackoffBase: time.Millisecond})

	resp, _ := p.Complete(context.Background(), domain.CompletionRequest{Prompt: "test"})
	if inner.calls.Load() != 1 {
		t.Errorf("calls = %d, want 1", inner.calls.Load())
	}
	if resp.Text != "fast" {
		t.Errorf("text = %q, want fast", resp.Text)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/llm/ -v -run TestRetrying
```

Expected: FAIL

- [ ] **Step 3: Implement retry.go**

```go
package llm

import (
	"context"
	"fmt"
	"time"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// RetryConfig controls retry behavior.
type RetryConfig struct {
	MaxRetries  int
	BackoffBase time.Duration
}

// DefaultRetryConfig returns sensible defaults (2 retries, 1s base).
func DefaultRetryConfig() RetryConfig {
	return RetryConfig{
		MaxRetries:  2,
		BackoffBase: time.Second,
	}
}

// RetryingProvider wraps an LlmProvider with exponential backoff retries.
type RetryingProvider struct {
	inner  domain.LlmProvider
	config RetryConfig
}

// NewRetryingProvider wraps a provider with retry logic.
func NewRetryingProvider(inner domain.LlmProvider, config RetryConfig) *RetryingProvider {
	return &RetryingProvider{inner: inner, config: config}
}

func (p *RetryingProvider) Name() string { return p.inner.Name() }

func (p *RetryingProvider) Complete(ctx context.Context, req domain.CompletionRequest) (domain.CompletionResponse, error) {
	var lastErr error
	for attempt := 0; attempt <= p.config.MaxRetries; attempt++ {
		if attempt > 0 {
			delay := p.config.BackoffBase * (1 << (attempt - 1))
			if delay > 30*time.Second {
				delay = 30 * time.Second
			}
			select {
			case <-time.After(delay):
			case <-ctx.Done():
				return domain.CompletionResponse{}, fmt.Errorf("%s: %w", p.inner.Name(), ctx.Err())
			}
		}
		resp, err := p.inner.Complete(ctx, req)
		if err == nil {
			return resp, nil
		}
		lastErr = err
	}
	return domain.CompletionResponse{}, fmt.Errorf("%s: retries exhausted: %w", p.inner.Name(), lastErr)
}
```

- [ ] **Step 4: Run tests**

```bash
go test ./internal/llm/ -v -run TestRetrying
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add agentbox/internal/llm/retry.go agentbox/internal/llm/retry_test.go
git commit -m "feat(agentbox): add RetryingProvider with exponential backoff"
```

---

## Task 8: Context Provider

**Files:**
- Create: `agentbox/internal/context/git.go`
- Create: `agentbox/internal/context/git_test.go`

- [ ] **Step 1: Write context provider tests**

```go
package gitctx

import (
	"context"
	"testing"
)

func TestBranchContextFormat(t *testing.T) {
	// Test that formatBranchContext produces expected format
	ctx := formatBranchContext("main", "feature-x", "abc1234 feat: something\ndef5678 fix: bug",
		"file.go\nother.go", "diff content here")

	if ctx == "" {
		t.Fatal("expected non-empty context")
	}
	if !containsAll(ctx, "Branch:", "Commits:", "Changed files:", "Diff:") {
		t.Error("missing expected sections in branch context")
	}
}

func containsAll(s string, substrs ...string) bool {
	for _, sub := range substrs {
		if !contains(s, sub) {
			return false
		}
	}
	return true
}

func contains(s, sub string) bool {
	return len(s) >= len(sub) && searchString(s, sub)
}

func searchString(s, sub string) bool {
	for i := 0; i <= len(s)-len(sub); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
go test ./internal/context/ -v
```

Expected: FAIL

- [ ] **Step 3: Implement git.go**

```go
package gitctx

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// GitProvider implements domain.ContextProvider using git CLI.
type GitProvider struct{}

// NewGitProvider creates a new git-based context provider.
func NewGitProvider() *GitProvider {
	return &GitProvider{}
}

func git(args ...string) string {
	out, _ := exec.Command("git", args...).Output()
	return strings.TrimSpace(string(out))
}

func (p *GitProvider) GitLog(_ context.Context, n int) ([]domain.Commit, error) {
	out := git("log", fmt.Sprintf("-%d", n), "--oneline")
	if out == "" {
		return nil, nil
	}
	var commits []domain.Commit
	for _, line := range strings.Split(out, "\n") {
		parts := strings.SplitN(line, " ", 2)
		if len(parts) == 2 {
			commits = append(commits, domain.Commit{SHA: parts[0], Message: parts[1]})
		}
	}
	return commits, nil
}

func (p *GitProvider) Diff(_ context.Context, base string) (string, error) {
	diff := git("diff", base+"...HEAD")
	if diff == "" {
		diff = git("diff", "HEAD")
	}
	return diff, nil
}

func (p *GitProvider) BranchContext(_ context.Context, base string) (string, error) {
	branch := git("rev-parse", "--abbrev-ref", "HEAD")
	commits := git("log", base+"...HEAD", "--oneline")
	files := git("diff", base+"...HEAD", "--name-only")
	diff := git("diff", base+"...HEAD")
	if diff == "" {
		diff = git("diff", "HEAD")
		commits = git("log", "-5", "--oneline")
		files = git("diff", "HEAD", "--name-only")
	}
	return formatBranchContext(base, branch, commits, files, diff), nil
}

func formatBranchContext(base, branch, commits, files, diff string) string {
	if commits == "" {
		commits = "(none ahead of base)"
	}
	if files == "" {
		files = "(none)"
	}
	if diff == "" {
		diff = "(no diff)"
	}
	return fmt.Sprintf("Branch: %s (vs %s)\n\nCommits:\n%s\n\nChanged files:\n%s\n\nDiff:\n```diff\n%s\n```",
		branch, base, commits, files, diff)
}

func (p *GitProvider) ProjectRules(_ context.Context) (domain.ProjectContext, error) {
	branch := git("rev-parse", "--abbrev-ref", "HEAD")
	gitLog := git("log", "--oneline", "-20")
	gitStatus := git("status", "--short")
	gitStat := git("diff", "HEAD", "--stat")

	// Discover rule files
	candidates := []string{"CLAUDE.md", "AGENTS.md", "GEMINI.md", "README.md"}
	var rules strings.Builder
	charBudget := 6000
	for _, name := range candidates {
		if charBudget <= 0 {
			break
		}
		data, err := os.ReadFile(name)
		if err != nil {
			continue
		}
		chunk := string(data)
		if len(chunk) > charBudget {
			chunk = chunk[:charBudget]
		}
		fmt.Fprintf(&rules, "\n### %s\n%s\n", name, chunk)
		charBudget -= len(chunk)
	}

	// Shallow structure
	var structure strings.Builder
	count := 0
	filepath.WalkDir(".", func(path string, d os.DirEntry, err error) error {
		if err != nil || count >= 150 {
			return filepath.SkipDir
		}
		for _, skip := range []string{".git", "target", "node_modules", "__pycache__", ".worktrees"} {
			if strings.Contains(path, skip) {
				return filepath.SkipDir
			}
		}
		fmt.Fprintln(&structure, path)
		count++
		return nil
	})

	return domain.ProjectContext{
		Rules:     rules.String(),
		Branch:    branch,
		GitLog:    gitLog,
		GitStatus: gitStatus,
		GitStat:   gitStat,
		Structure: structure.String(),
	}, nil
}
```

- [ ] **Step 4: Run test**

```bash
go test ./internal/context/ -v
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add agentbox/internal/context/
git commit -m "feat(agentbox): add GitContextProvider for repo context discovery"
```

---

## Task 9: Agent SDK Wrapper

**Files:**
- Create: `agentbox/internal/agent/sdk.go`
- Create: `agentbox/internal/agent/sdk_test.go`

- [ ] **Step 1: Add Agent SDK dependency**

```bash
cd /Users/joe/dev/minibox/agentbox
go get github.com/severity1/claude-agent-sdk-go
```

- [ ] **Step 2: Write SDK wrapper test**

```go
package agent

import (
	"context"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

func TestAgentConfigToQueryOptions(t *testing.T) {
	config := domain.AgentConfig{
		Name:   "test-agent",
		Prompt: "Analyze this code",
		Tools:  []string{"Read", "Glob", "Grep"},
	}

	opts := configToQueryOptions(config)
	if len(opts) == 0 {
		t.Error("expected non-empty query options")
	}
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
go test ./internal/agent/ -v
```

Expected: FAIL

- [ ] **Step 4: Implement sdk.go**

```go
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

func configToQueryOptions(config domain.AgentConfig) []claudecode.QueryOption {
	var opts []claudecode.QueryOption
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
		if result, ok := msg.(claudecode.ResultMessage); ok {
			parts = append(parts, result.Result)
		}
	}

	output := ""
	for _, p := range parts {
		if output != "" {
			output += "\n"
		}
		output += p
	}

	return domain.AgentResult{Name: config.Name, Output: output}, nil
}
```

- [ ] **Step 5: Run test**

```bash
go test ./internal/agent/ -v
```

Expected: PASS (unit test only tests option conversion, not actual CLI)

- [ ] **Step 6: Commit**

```bash
git add agentbox/internal/agent/
git commit -m "feat(agentbox): add ClaudeSDKRunner wrapping claude-agent-sdk-go"
```

---

## Task 10: Council Agent

**Files:**
- Create: `agentbox/internal/orchestrator/council.go`
- Create: `agentbox/internal/orchestrator/council_test.go`

- [ ] **Step 1: Write council test**

```go
package orchestrator

import (
	"context"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

type mockRunner struct {
	responses map[string]string
}

func (m *mockRunner) Run(_ context.Context, config domain.AgentConfig) (domain.AgentResult, error) {
	output := m.responses[config.Name]
	if output == "" {
		output = "Mock output for " + config.Name
	}
	return domain.AgentResult{Name: config.Name, Output: output}, nil
}

func TestCouncilCoreRoles(t *testing.T) {
	roles := CoreRoles()
	if len(roles) != 3 {
		t.Errorf("core roles = %d, want 3", len(roles))
	}
	names := map[string]bool{}
	for _, r := range roles {
		names[r.Key] = true
	}
	for _, expected := range []string{"strict-critic", "creative-explorer", "general-analyst"} {
		if !names[expected] {
			t.Errorf("missing role %q", expected)
		}
	}
}

func TestCouncilExtensiveRoles(t *testing.T) {
	roles := ExtensiveRoles()
	if len(roles) != 5 {
		t.Errorf("extensive roles = %d, want 5", len(roles))
	}
}

func TestCouncilRunCollectsAllRoles(t *testing.T) {
	runner := &mockRunner{responses: map[string]string{
		"strict-critic":    "Score: 0.7\nBad code",
		"creative-explorer": "Score: 0.9\nGreat ideas",
		"general-analyst":   "Score: 0.8\nBalanced view",
	}}

	council := &Council{runner: runner}
	results, err := council.RunRoles(context.Background(), CoreRoles(), "test context")
	if err != nil {
		t.Fatalf("RunRoles: %v", err)
	}
	if len(results) != 3 {
		t.Errorf("results = %d, want 3", len(results))
	}
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
go test ./internal/orchestrator/ -v
```

Expected: FAIL

- [ ] **Step 3: Implement council.go**

```go
package orchestrator

import (
	"context"
	"fmt"
	"strings"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// Role defines a council reviewer role.
type Role struct {
	Key    string
	Label  string
	Prompt string
}

var roles = map[string]Role{
	"strict-critic": {
		Key:   "strict-critic",
		Label: "Strict Critic",
		Prompt: "You are the STRICT CRITIC on a council of expert reviewers. Be conservative and demanding.\n" +
			"Your output must include:\n" +
			"- **Health Score**: 0.0–1.0 (be conservative — only near-perfect code scores above 0.85)\n" +
			"- **Summary**: What concerns you most (2–3 sentences)\n" +
			"- **Key Observations**: Specific findings — cite file paths and line numbers where possible\n" +
			"- **Risks Identified**: Technical debt, incomplete paths, missing invariants, breaking changes\n" +
			"- **Code Smells**: Quality and maintainability issues\n" +
			"- **Recommendations**: Concrete risk-mitigation actions\n\n" +
			"Format each observation as: 'finding — source: file/symbol or commit hash'",
	},
	"creative-explorer": {
		Key:   "creative-explorer",
		Label: "Creative Explorer",
		Prompt: "You are the CREATIVE EXPLORER on a council of expert reviewers. Be optimistic and inventive.\n" +
			"Your output must include:\n" +
			"- **Health Score**: 0.0–1.0 (reward ambition and potential)\n" +
			"- **Summary**: What excites you about this change (2–3 sentences)\n" +
			"- **Innovation Opportunities**: Simpler approaches, pattern unifications, new possibilities unlocked\n" +
			"- **Architectural Potential**: How this lays groundwork for future improvements\n" +
			"- **Experimental Value**: What this change validates or disproves\n" +
			"- **Recommendations**: Ideas to extend or amplify the value of this work\n\n" +
			"Format each observation as: 'finding — source: file/symbol or commit hash'",
	},
	"general-analyst": {
		Key:   "general-analyst",
		Label: "General Analyst",
		Prompt: "You are the GENERAL ANALYST on a council of expert reviewers. Be balanced and evidence-based.\n" +
			"Your output must include:\n" +
			"- **Health Score**: 0.0–1.0 (weight quality, tests, and conventions equally)\n" +
			"- **Summary**: Overall assessment of branch state (2–3 sentences)\n" +
			"- **Progress Indicators**: What is done well, with evidence\n" +
			"- **Work Patterns**: Development approach and consistency\n" +
			"- **Gaps**: Missing tests, docs, or convention violations (cite CLAUDE.md where relevant)\n" +
			"- **Recommendations**: Balanced improvements\n\n" +
			"Format each observation as: 'finding — source: file/symbol or commit hash'",
	},
	"security-reviewer": {
		Key:   "security-reviewer",
		Label: "Security Reviewer",
		Prompt: "You are the SECURITY REVIEWER on a council of expert reviewers. Focus on attack surface.\n" +
			"For this Rust container runtime, scrutinise: path traversal, symlink attacks, tar extraction,\n" +
			"privilege escalation, unsafe block soundness, socket auth bypasses, resource exhaustion,\n" +
			"cgroup/namespace escapes, and any new attack surface introduced.\n" +
			"Your output must include:\n" +
			"- **Health Score**: 0.0–1.0 (any critical vuln = max 0.4)\n" +
			"- **Summary**: Security posture of this change\n" +
			"- **Findings**: Each rated critical / high / medium / low — cite exact code locations\n" +
			"- **Recommendations**: Specific hardening actions\n\n" +
			"Format each observation as: 'finding — source: file/symbol or commit hash'",
	},
	"performance-analyst": {
		Key:   "performance-analyst",
		Label: "Performance Analyst",
		Prompt: "You are the PERFORMANCE ANALYST on a council of expert reviewers. Focus on efficiency.\n" +
			"Your output must include:\n" +
			"- **Health Score**: 0.0–1.0\n" +
			"- **Summary**: Performance posture of this change\n" +
			"- **Bottlenecks**: Unnecessary allocations, blocking calls in async context, redundant syscalls,\n" +
			"  lock contention, inefficient algorithms — cite exact code locations\n" +
			"- **Zero-Copy / Benchmark Risks**: Missed opportunities and regression risks\n" +
			"- **Recommendations**: Concrete alternatives with expected impact\n\n" +
			"Format each observation as: 'finding — source: file/symbol or commit hash'",
	},
}

// CoreRoles returns the 3 core council roles.
func CoreRoles() []Role {
	return []Role{roles["strict-critic"], roles["creative-explorer"], roles["general-analyst"]}
}

// ExtensiveRoles returns all 5 council roles.
func ExtensiveRoles() []Role {
	return append(CoreRoles(), roles["security-reviewer"], roles["performance-analyst"])
}

// SynthesisPrompt returns the synthesis prompt for combining role outputs.
func SynthesisPrompt(roleOutputs map[string]string, branchContext string) string {
	var council strings.Builder
	for key, output := range roleOutputs {
		role := roles[key]
		fmt.Fprintf(&council, "\n\n---\n\n### %s\n%s", role.Label, output)
	}
	return "You are synthesising a multi-role council code review into a final verdict.\n\n" +
		"Your synthesis must contain exactly these sections:\n\n" +
		"**Health Scores**\n" +
		"List each role's score and compute the meta-score (weighted average, " +
		"give Strict Critic 1.5× weight).\n\n" +
		"**Areas of Consensus**\n" +
		"Bullet points of findings where 2+ roles agree.\n\n" +
		"**Areas of Tension**\n" +
		"For each disagreement use this dialectic format:\n" +
		"'[Role A] sees [X] (conservative/optimistic view), AND [Role B] sees [Y], " +
		"suggesting [balanced resolution].'\n\n" +
		"**Balanced Recommendations**\n" +
		"Top 3–5 ranked actions the developer should take, synthesising all perspectives.\n\n" +
		"**Branch Health**\n" +
		"One of: Good / Needs work / Significant issues — with a one-line justification.\n\n" +
		fmt.Sprintf("Branch context:\n%s\n\nCouncil findings:%s", branchContext, council.String())
}

// Council orchestrates multi-role analysis.
type Council struct {
	runner domain.AgentRunner
}

// NewCouncil creates a council orchestrator.
func NewCouncil(runner domain.AgentRunner) *Council {
	return &Council{runner: runner}
}

// RunRoles runs all roles sequentially and returns their outputs.
func (c *Council) RunRoles(ctx context.Context, roleList []Role, branchContext string) (map[string]string, error) {
	results := make(map[string]string)
	for _, role := range roleList {
		config := domain.AgentConfig{
			Name: role.Key,
			Role: role.Label,
			Prompt: fmt.Sprintf("%s\n\nAnalyse this branch. Read relevant source files to support your findings.\n\n%s",
				role.Prompt, branchContext),
			Tools: []string{"Read", "Glob", "Grep"},
		}
		result, err := c.runner.Run(ctx, config)
		if err != nil {
			return nil, fmt.Errorf("role %s: %w", role.Key, err)
		}
		results[role.Key] = result.Output
		fmt.Printf("  [%s] done\n", role.Label)
	}
	return results, nil
}

// RunSynthesis runs the synthesis step on collected role outputs.
func (c *Council) RunSynthesis(ctx context.Context, roleOutputs map[string]string, branchContext string) (string, error) {
	config := domain.AgentConfig{
		Name:   "synthesis",
		Role:   "Synthesis",
		Prompt: SynthesisPrompt(roleOutputs, branchContext),
		Tools:  []string{"Read", "Glob", "Grep"},
	}
	result, err := c.runner.Run(ctx, config)
	if err != nil {
		return "", fmt.Errorf("synthesis: %w", err)
	}
	return result.Output, nil
}
```

- [ ] **Step 4: Run tests**

```bash
go test ./internal/orchestrator/ -v
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add agentbox/internal/orchestrator/
git commit -m "feat(agentbox): add council agent with 5 roles and synthesis"
```

---

## Task 11: Meta-Agent

**Files:**
- Create: `agentbox/internal/orchestrator/meta.go`
- Create: `agentbox/internal/orchestrator/meta_test.go`

- [ ] **Step 1: Write meta-agent tests**

```go
package orchestrator

import (
	"context"
	"encoding/json"
	"testing"
)

func TestParseAgentPlan(t *testing.T) {
	raw := `[
		{"name": "analyzer", "role": "Analyze code", "prompt": "Look at the code", "tools": ["Read", "Glob"]},
		{"name": "reviewer", "role": "Review tests", "prompt": "Check tests", "tools": ["Read"]}
	]`

	plan, err := parseAgentPlan(raw)
	if err != nil {
		t.Fatalf("parseAgentPlan: %v", err)
	}
	if len(plan) != 2 {
		t.Fatalf("plan length = %d, want 2", len(plan))
	}
	if plan[0].Name != "analyzer" {
		t.Errorf("plan[0].Name = %q, want analyzer", plan[0].Name)
	}
	if len(plan[1].Tools) != 1 || plan[1].Tools[0] != "Read" {
		t.Errorf("plan[1].Tools = %v, want [Read]", plan[1].Tools)
	}
}

func TestParseAgentPlanWithMarkdownFences(t *testing.T) {
	raw := "```json\n[{\"name\": \"test\", \"role\": \"Test\", \"prompt\": \"Do thing\", \"tools\": [\"Read\"]}]\n```"
	plan, err := parseAgentPlan(raw)
	if err != nil {
		t.Fatalf("parseAgentPlan with fences: %v", err)
	}
	if len(plan) != 1 {
		t.Fatalf("plan length = %d, want 1", len(plan))
	}
}

func TestParseAgentPlanInvalid(t *testing.T) {
	_, err := parseAgentPlan("not json at all")
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
}

func TestMetaAgentRunParallel(t *testing.T) {
	runner := &mockRunner{responses: map[string]string{
		"agent-a": "Result A",
		"agent-b": "Result B",
	}}

	meta := NewMetaAgent(runner)
	plan := []AgentSpec{
		{Name: "agent-a", Role: "A", Prompt: "Do A", Tools: []string{"Read"}},
		{Name: "agent-b", Role: "B", Prompt: "Do B", Tools: []string{"Read"}},
	}

	results, err := meta.RunParallel(context.Background(), plan)
	if err != nil {
		t.Fatalf("RunParallel: %v", err)
	}
	if len(results) != 2 {
		t.Fatalf("results = %d, want 2", len(results))
	}
	if results["agent-a"] != "Result A" {
		t.Errorf("agent-a = %q, want Result A", results["agent-a"])
	}
}

func TestMetaAgentSynthesisPrompt(t *testing.T) {
	outputs := map[string]string{"a": "found X", "b": "found Y"}
	prompt := MetaSynthesisPrompt("fix bugs", outputs)
	if prompt == "" {
		t.Fatal("expected non-empty synthesis prompt")
	}

	// Verify it contains expected sections
	for _, expected := range []string{"Summary", "Key Findings", "Recommended Actions", "Open Questions"} {
		if !contains(prompt, expected) {
			t.Errorf("missing section %q in synthesis prompt", expected)
		}
	}
}

// marshalJSON is a test helper
func marshalJSON(v any) string {
	data, _ := json.Marshal(v)
	return string(data)
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/orchestrator/ -v -run TestMeta
```

Expected: FAIL

- [ ] **Step 3: Implement meta.go**

```go
package orchestrator

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// AgentSpec is a single agent in a meta-agent plan.
type AgentSpec struct {
	Name   string   `json:"name"`
	Role   string   `json:"role"`
	Prompt string   `json:"prompt"`
	Tools  []string `json:"tools"`
}

var allowedTools = map[string]bool{
	"Read": true, "Glob": true, "Grep": true,
	"Bash": true, "Write": true, "Edit": true,
}

var safeTools = []string{"Read", "Glob", "Grep"}

// parseAgentPlan parses JSON (optionally fenced in markdown) into agent specs.
func parseAgentPlan(raw string) ([]AgentSpec, error) {
	raw = strings.TrimSpace(raw)
	if strings.HasPrefix(raw, "```") {
		lines := strings.SplitN(raw, "\n", 2)
		if len(lines) == 2 {
			raw = strings.TrimSuffix(lines[1], "```")
			raw = strings.TrimSpace(raw)
		}
	}
	var plan []AgentSpec
	if err := json.Unmarshal([]byte(raw), &plan); err != nil {
		return nil, fmt.Errorf("parse agent plan: %w", err)
	}
	// Validate tools
	for i := range plan {
		var valid []string
		for _, t := range plan[i].Tools {
			if allowedTools[t] {
				valid = append(valid, t)
			}
		}
		if len(valid) == 0 {
			plan[i].Tools = safeTools
		} else {
			plan[i].Tools = valid
		}
	}
	return plan, nil
}

// DesignerPrompt returns the prompt for the designer agent.
func DesignerPrompt(task, repoContext, sdkDocs string) string {
	if len(repoContext) > 4000 {
		repoContext = repoContext[:4000]
	}
	if len(sdkDocs) > 6000 {
		sdkDocs = sdkDocs[:6000]
	}
	return "You are a meta-agent designer. Given a task, repo context, and the Claude Agent SDK docs, " +
		"design the smallest set of parallel agents that efficiently accomplishes the task.\n\n" +
		"Output ONLY a valid JSON array — no markdown fences, no explanation — with this schema:\n" +
		"[\n" +
		"  {\n" +
		"    \"name\": \"kebab-case-name\",\n" +
		"    \"role\": \"one sentence describing this agent's independent concern\",\n" +
		"    \"prompt\": \"complete self-contained prompt for this agent\",\n" +
		"    \"tools\": [\"Read\", \"Glob\", \"Grep\"]\n" +
		"  }\n" +
		"]\n\n" +
		"Rules:\n" +
		"- 2–5 agents; each must have a distinct, non-overlapping concern\n" +
		"- Prompts must be fully self-contained (the agent has no other context)\n" +
		"- Include the repo context in each prompt only where relevant to that agent's concern\n" +
		"- Available tools: Read, Glob, Grep (safe reads); Bash, Write, Edit (modifications)\n" +
		"- Only grant Write/Edit/Bash when the agent genuinely needs to modify or execute\n" +
		"- Do NOT include a synthesis agent — synthesis is handled externally\n\n" +
		fmt.Sprintf("## Task\n%s\n\n## Repo context\n%s\n\n## Claude Agent SDK docs\n%s",
			task, repoContext, sdkDocs)
}

// MetaSynthesisPrompt returns the prompt for synthesizing agent outputs.
func MetaSynthesisPrompt(task string, agentOutputs map[string]string) string {
	var combined strings.Builder
	for name, output := range agentOutputs {
		fmt.Fprintf(&combined, "\n\n---\n\n### %s\n%s", name, output)
	}
	return "Synthesize the outputs from multiple parallel agents into a single coherent report.\n\n" +
		"Sections (use exactly these headings):\n\n" +
		"**Summary** — 2–3 sentences: what the agents found and the overall verdict\n\n" +
		"**Key Findings** — deduplicated bullet points from all agents, grouped by theme\n\n" +
		"**Recommended Actions** — ranked list; include who/what/why for each\n\n" +
		"**Open Questions** — anything unresolved or needing follow-up\n\n" +
		fmt.Sprintf("Original task: %s\n\nAgent outputs:%s", task, combined.String())
}

// MetaAgent orchestrates the design-spawn-synthesize workflow.
type MetaAgent struct {
	runner domain.AgentRunner
}

// NewMetaAgent creates a meta-agent orchestrator.
func NewMetaAgent(runner domain.AgentRunner) *MetaAgent {
	return &MetaAgent{runner: runner}
}

// RunParallel executes all agents in the plan concurrently.
func (m *MetaAgent) RunParallel(ctx context.Context, plan []AgentSpec) (map[string]string, error) {
	type result struct {
		name   string
		output string
		err    error
	}

	results := make(chan result, len(plan))
	var wg sync.WaitGroup

	for _, spec := range plan {
		wg.Add(1)
		go func(s AgentSpec) {
			defer wg.Done()
			config := domain.AgentConfig{
				Name:   s.Name,
				Role:   s.Role,
				Prompt: s.Prompt,
				Tools:  s.Tools,
			}
			r, err := m.runner.Run(ctx, config)
			results <- result{name: s.Name, output: r.Output, err: err}
		}(spec)
	}

	go func() {
		wg.Wait()
		close(results)
	}()

	outputs := make(map[string]string)
	var errs []string
	for r := range results {
		if r.err != nil {
			errs = append(errs, fmt.Sprintf("%s: %v", r.name, r.err))
			continue
		}
		outputs[r.name] = r.output
		fmt.Printf("  [%s] done\n", r.name)
	}

	if len(errs) > 0 && len(outputs) == 0 {
		return nil, fmt.Errorf("all agents failed: %s", strings.Join(errs, "; "))
	}
	return outputs, nil
}
```

- [ ] **Step 4: Run tests**

```bash
go test ./internal/orchestrator/ -v -run TestMeta
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add agentbox/internal/orchestrator/meta.go agentbox/internal/orchestrator/meta_test.go
git commit -m "feat(agentbox): add meta-agent with design-spawn-synthesize workflow"
```

---

## Task 12: Commit Message Tool Agent

**Files:**
- Create: `agentbox/internal/tools/commitmsg.go`
- Create: `agentbox/internal/tools/commitmsg_test.go`

- [ ] **Step 1: Write commit-msg tests**

```go
package tools

import (
	"context"
	"testing"

	"github.com/joe/minibox/agentbox/internal/domain"
)

type mockRunner struct {
	lastPrompt string
}

func (m *mockRunner) Run(_ context.Context, config domain.AgentConfig) (domain.AgentResult, error) {
	m.lastPrompt = config.Prompt
	return domain.AgentResult{Name: config.Name, Output: "feat(agentbox): add commit message tool"}, nil
}

func TestCommitMsgPromptContainsRules(t *testing.T) {
	runner := &mockRunner{}
	cm := NewCommitMsg(runner)

	ctx := CommitMsgContext{
		Branch:     "feature-x",
		StagedDiff: "diff --git a/foo.go\n+new line",
		StagedStat: "1 file changed, 1 insertion",
		RecentLog:  "abc123 feat: prior change",
	}

	_, err := cm.Generate(context.Background(), ctx)
	if err != nil {
		t.Fatalf("Generate: %v", err)
	}

	for _, expected := range []string{"conventional commits", "feat, fix, docs", "≤72 chars"} {
		if !contains(runner.lastPrompt, expected) {
			t.Errorf("prompt missing %q", expected)
		}
	}
}

func contains(s, sub string) bool {
	for i := 0; i <= len(s)-len(sub); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
go test ./internal/tools/ -v
```

Expected: FAIL

- [ ] **Step 3: Implement commitmsg.go**

```go
package tools

import (
	"context"
	"fmt"

	"github.com/joe/minibox/agentbox/internal/domain"
)

const maxDiffBytes = 64 * 1024

// CommitMsgContext is the input for commit message generation.
type CommitMsgContext struct {
	Branch       string
	StagedDiff   string
	StagedStat   string
	UnstagedStat string
	RecentLog    string
	Status       string
}

// CommitMsg generates conventional commit messages.
type CommitMsg struct {
	runner domain.AgentRunner
}

// NewCommitMsg creates a commit message generator.
func NewCommitMsg(runner domain.AgentRunner) *CommitMsg {
	return &CommitMsg{runner: runner}
}

// Generate produces a commit message from staged changes.
func (c *CommitMsg) Generate(ctx context.Context, input CommitMsgContext) (string, error) {
	diff := input.StagedDiff
	if len(diff) > maxDiffBytes {
		diff = fmt.Sprintf("(diff too large — %d KB; using stat only)\n%s", len(diff)/1024, input.StagedStat)
	}

	prompt := "Generate a git commit message for the staged changes below.\n\n" +
		"Rules:\n" +
		"- Follow the existing commit style shown in the recent log\n" +
		"- Use conventional commits format: `type(scope): description`\n" +
		"  Types: feat, fix, docs, refactor, test, chore, perf, ci\n" +
		"  Scope: crate name, module, or area (e.g. linuxbox, standup, justfile)\n" +
		"- First line: ≤72 chars, imperative mood, no period\n" +
		"- If the change warrants it, add a blank line then a short body (2–4 lines max)\n" +
		"- Do NOT add 'Co-Authored-By' lines — those are added separately\n" +
		"- Output ONLY the commit message, nothing else — no explanation, no markdown fences\n\n" +
		fmt.Sprintf("Branch: %s\n\n", input.Branch) +
		fmt.Sprintf("Recent commits (style reference):\n%s\n\n", input.RecentLog) +
		fmt.Sprintf("Staged changes (%s):\n```diff\n%s\n```\n",
			orDefault(input.StagedStat, "none"), orDefault(diff, "(nothing staged)"))

	if input.UnstagedStat != "" {
		prompt += fmt.Sprintf("\nUnstaged (not included):\n%s\n", input.UnstagedStat)
	}

	config := domain.AgentConfig{
		Name:   "commit-msg",
		Prompt: prompt,
		Tools:  []string{"Read", "Glob", "Grep"},
	}

	result, err := c.runner.Run(ctx, config)
	if err != nil {
		return "", fmt.Errorf("generate commit message: %w", err)
	}
	return result.Output, nil
}

func orDefault(s, def string) string {
	if s == "" {
		return def
	}
	return s
}
```

- [ ] **Step 4: Run tests**

```bash
go test ./internal/tools/ -v
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add agentbox/internal/tools/
git commit -m "feat(agentbox): add commit message tool agent"
```

---

## Task 13: Orchestration Binary (cmd/agentbox)

**Files:**
- Create: `agentbox/cmd/agentbox/main.go`

- [ ] **Step 1: Implement main.go with subcommand dispatch**

```go
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/joe/minibox/agentbox/internal/agent"
	gitctx "github.com/joe/minibox/agentbox/internal/context"
	"github.com/joe/minibox/agentbox/internal/domain"
	"github.com/joe/minibox/agentbox/internal/orchestrator"
	"github.com/joe/minibox/agentbox/internal/output"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "Usage: agentbox <command> [args]")
		fmt.Fprintln(os.Stderr, "Commands: council, meta-agent")
		os.Exit(1)
	}

	ctx := context.Background()

	switch os.Args[1] {
	case "council":
		runCouncil(ctx, os.Args[2:])
	case "meta-agent":
		runMetaAgent(ctx, os.Args[2:])
	default:
		fmt.Fprintf(os.Stderr, "unknown command: %s\n", os.Args[1])
		os.Exit(1)
	}
}

func gitShortSHA() string {
	out, _ := exec.Command("git", "rev-parse", "--short", "HEAD").Output()
	return strings.TrimSpace(string(out))
}

func runCouncil(ctx context.Context, args []string) {
	fs := flag.NewFlagSet("council", flag.ExitOnError)
	base := fs.String("base", "main", "Base branch/ref")
	mode := fs.String("mode", "core", "core or extensive")
	noSynth := fs.Bool("no-synthesis", false, "Skip synthesis step")
	fs.Parse(args)

	var roleList []orchestrator.Role
	if *mode == "extensive" {
		roleList = orchestrator.ExtensiveRoles()
	} else {
		roleList = orchestrator.CoreRoles()
	}

	sha := gitShortSHA()
	fmt.Printf("\nCouncil analysis — %s mode · %d roles + synthesis · vs %s @ %s\n\n",
		*mode, len(roleList), *base, sha)

	runner := agent.NewClaudeSDKRunner()
	gitProvider := gitctx.NewGitProvider()
	writer := output.NewDualWriter()

	branchCtx, err := gitProvider.BranchContext(ctx, *base)
	if err != nil {
		fmt.Fprintf(os.Stderr, "branch context: %v\n", err)
		os.Exit(1)
	}

	runID := time.Now().Format(time.RFC3339)
	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "council",
		Args: map[string]any{"base": *base, "mode": *mode}, Status: "running",
	})
	start := time.Now()

	council := orchestrator.NewCouncil(runner)
	roleOutputs, err := council.RunRoles(ctx, roleList, branchCtx)
	if err != nil {
		fmt.Fprintf(os.Stderr, "council: %v\n", err)
		os.Exit(1)
	}

	var allOutput []string
	fmt.Println()
	for key, out := range roleOutputs {
		label := key // simplified
		fmt.Printf("──── %s %s\n%s\n\n", label, strings.Repeat("─", 56-len(label)), out)
		allOutput = append(allOutput, fmt.Sprintf("## %s\n%s", label, out))
	}

	if !*noSynth {
		synthesis, err := council.RunSynthesis(ctx, roleOutputs, branchCtx)
		if err != nil {
			fmt.Fprintf(os.Stderr, "synthesis: %v\n", err)
			os.Exit(1)
		}
		fmt.Printf("%s\n  SYNTHESIS\n%s\n%s\n\n", strings.Repeat("─", 60), strings.Repeat("─", 60), synthesis)
		allOutput = append(allOutput, fmt.Sprintf("## Synthesis\n%s", synthesis))
	}

	fullOutput := strings.Join(allOutput, "\n\n")
	duration := time.Since(start).Seconds()
	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "council",
		Args: map[string]any{"base": *base, "mode": *mode},
		Status: "complete", DurationS: duration, Output: fullOutput,
	})
	writer.WriteReport(ctx, domain.AgentReport{
		SHA: sha, Script: fmt.Sprintf("council-%s", *mode), Content: fullOutput,
		Meta: map[string]string{"base": *base, "mode": *mode, "date": time.Now().Format("2006-01-02 15:04")},
	})
}

func runMetaAgent(ctx context.Context, args []string) {
	fs := flag.NewFlagSet("meta-agent", flag.ExitOnError)
	noSynth := fs.Bool("no-synthesis", false, "Skip synthesis step")
	fs.Parse(args)

	task := strings.Join(fs.Args(), " ")
	if task == "" {
		fmt.Fprintln(os.Stderr, "Usage: agentbox meta-agent <task description>")
		os.Exit(1)
	}

	sha := gitShortSHA()
	fmt.Printf("\nmeta-agent @ %s — %s\n\nTask: %s\n\n", sha, time.Now().Format("2006-01-02 15:04"), task)

	runner := agent.NewClaudeSDKRunner()
	gitProvider := gitctx.NewGitProvider()
	writer := output.NewDualWriter()

	fmt.Print("Collecting repo context... ")
	projectCtx, err := gitProvider.ProjectRules(ctx)
	if err != nil {
		fmt.Fprintf(os.Stderr, "context: %v\n", err)
		os.Exit(1)
	}
	repoCtx := fmt.Sprintf("## Project rules\n%s\n\n## Branch: %s\n## Recent commits\n%s\n\n## Working tree\n%s\n%s\n\n## Structure\n%s",
		projectCtx.Rules, projectCtx.Branch, projectCtx.GitLog, projectCtx.GitStatus, projectCtx.GitStat, projectCtx.Structure)
	fmt.Println("done")

	runID := time.Now().Format(time.RFC3339)
	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "meta-agent",
		Args: map[string]any{"task": truncate(task, 120)}, Status: "running",
	})
	start := time.Now()

	// Phase 1: Design
	fmt.Println("\nDesigning agent configuration...")
	designPrompt := orchestrator.DesignerPrompt(task, repoCtx, "")
	designResult, err := runner.Run(ctx, domain.AgentConfig{
		Name: "designer", Prompt: designPrompt, Tools: []string{"Read", "Glob", "Grep"},
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "designer: %v\n", err)
		os.Exit(1)
	}

	plan, err := orchestrator.ParseAgentPlanExported(designResult.Output)
	if err != nil {
		fmt.Printf("Warning: designer returned invalid JSON (%v) — using single fallback agent.\n", err)
		plan = []orchestrator.AgentSpec{{Name: "analyst", Role: "General analysis", Prompt: task, Tools: []string{"Read", "Glob", "Grep"}}}
	}

	var allOutput []string
	var planLines []string
	for _, a := range plan {
		planLines = append(planLines, fmt.Sprintf("- **%s**: %s (tools: %s)", a.Name, a.Role, strings.Join(a.Tools, ", ")))
	}
	planMd := strings.Join(planLines, "\n")
	fmt.Printf("\nPlan (%d agents):\n%s\n\n", len(plan), planMd)
	allOutput = append(allOutput, fmt.Sprintf("## Agent Plan\n%s", planMd))

	// Phase 2: Execute in parallel
	meta := orchestrator.NewMetaAgent(runner)
	fmt.Printf("Running %d agents in parallel...\n", len(plan))
	agentOutputs, err := meta.RunParallel(ctx, plan)
	if err != nil {
		fmt.Fprintf(os.Stderr, "agents: %v\n", err)
		os.Exit(1)
	}

	fmt.Println()
	for name, out := range agentOutputs {
		fmt.Printf("──── %s %s\n%s\n\n", name, strings.Repeat("─", max(0, 56-len(name))), out)
		allOutput = append(allOutput, fmt.Sprintf("## %s\n%s", name, out))
	}

	// Phase 3: Synthesize
	if !*noSynth {
		fmt.Println("Synthesizing...")
		synthPrompt := orchestrator.MetaSynthesisPrompt(task, agentOutputs)
		synthResult, err := runner.Run(ctx, domain.AgentConfig{
			Name: "synthesis", Prompt: synthPrompt, Tools: []string{"Read", "Glob", "Grep"},
		})
		if err != nil {
			fmt.Fprintf(os.Stderr, "synthesis: %v\n", err)
			os.Exit(1)
		}
		fmt.Printf("%s\n  SYNTHESIS\n%s\n%s\n\n", strings.Repeat("─", 60), strings.Repeat("─", 60), synthResult.Output)
		allOutput = append(allOutput, fmt.Sprintf("## Synthesis\n%s", synthResult.Output))
	}

	fullOutput := strings.Join(allOutput, "\n\n")
	duration := time.Since(start).Seconds()
	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "meta-agent",
		Args: map[string]any{"task": truncate(task, 120)},
		Status: "complete", DurationS: duration, Output: fullOutput,
	})
	writer.WriteReport(ctx, domain.AgentReport{
		SHA: sha, Script: "meta-agent", Content: fullOutput,
		Meta: map[string]string{"task": truncate(task, 120), "agents": fmt.Sprintf("%d", len(plan)),
			"date": time.Now().Format("2006-01-02 15:04")},
	})
	fmt.Printf("\nDone in %.1fs\n", duration)
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n]
}
```

- [ ] **Step 2: Export parseAgentPlan for use by main.go**

Add to `agentbox/internal/orchestrator/meta.go`:

```go
// ParseAgentPlanExported is the exported version of parseAgentPlan.
func ParseAgentPlanExported(raw string) ([]AgentSpec, error) {
	return parseAgentPlan(raw)
}
```

- [ ] **Step 3: Verify it compiles**

```bash
cd /Users/joe/dev/minibox/agentbox
go build ./cmd/agentbox/
```

Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add agentbox/cmd/agentbox/ agentbox/internal/orchestrator/meta.go
git commit -m "feat(agentbox): add orchestration binary with council and meta-agent subcommands"
```

---

## Task 14: Standalone Commit-Msg Binary

**Files:**
- Create: `agentbox/cmd/mbx-commit-msg/main.go`

- [ ] **Step 1: Implement standalone binary**

```go
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/joe/minibox/agentbox/internal/agent"
	"github.com/joe/minibox/agentbox/internal/domain"
	"github.com/joe/minibox/agentbox/internal/output"
	"github.com/joe/minibox/agentbox/internal/tools"
)

func main() {
	stageAll := flag.Bool("a", false, "Stage all changes (git add -A) before generating")
	commit := flag.Bool("c", false, "Commit with the generated message after confirming")
	yes := flag.Bool("y", false, "Skip confirmation and commit immediately (implies -c)")
	flag.Parse()

	if *yes {
		*commit = true
	}

	if *stageAll {
		exec.Command("git", "add", "-A").Run()
	}

	stagedDiff := gitRun("diff", "--cached")
	stagedStat := gitRun("diff", "--cached", "--stat")
	if strings.TrimSpace(stagedDiff) == "" {
		status := gitRun("status", "--short")
		if status != "" {
			fmt.Println("Nothing staged. Use -a to stage all, or `git add` files first.")
		} else {
			fmt.Println("Working tree is clean — nothing to commit.")
		}
		os.Exit(1)
	}

	ctx := context.Background()
	runner := agent.NewClaudeSDKRunner()
	writer := output.NewDualWriter()

	input := tools.CommitMsgContext{
		Branch:       gitRun("rev-parse", "--abbrev-ref", "HEAD"),
		StagedDiff:   stagedDiff,
		StagedStat:   stagedStat,
		UnstagedStat: gitRun("diff", "--stat"),
		RecentLog:    gitRun("log", "-8", "--oneline"),
		Status:       gitRun("status", "--short"),
	}

	fmt.Printf("Generating commit message for %s...\n\n", input.StagedStat)

	runID := time.Now().Format(time.RFC3339)
	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "commit-msg",
		Args: map[string]any{"stage": *stageAll, "commit": *commit}, Status: "running",
	})
	start := time.Now()

	cm := tools.NewCommitMsg(runner)
	msg, err := cm.Generate(ctx, input)
	if err != nil {
		fmt.Fprintf(os.Stderr, "generate: %v\n", err)
		os.Exit(1)
	}

	fmt.Printf("%s\n%s\n%s\n", strings.Repeat("─", 60), msg, strings.Repeat("─", 60))

	if *commit {
		if !*yes {
			fmt.Print("\nCommit with this message? [y/N] ")
			var answer string
			fmt.Scanln(&answer)
			if strings.ToLower(strings.TrimSpace(answer)) != "y" {
				fmt.Println("Aborted.")
				writer.WriteRun(ctx, domain.AgentRun{
					RunID: runID, Script: "commit-msg",
					Args: map[string]any{"stage": *stageAll, "commit": false},
					Status: "complete", DurationS: time.Since(start).Seconds(), Output: msg,
				})
				os.Exit(0)
			}
		}

		fullMsg := msg + "\n\nCo-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
		result := exec.Command("git", "commit", "-m", fullMsg)
		result.Stdout = os.Stdout
		result.Stderr = os.Stderr
		if err := result.Run(); err != nil {
			fmt.Println("\nCommit failed — check git output above.")
			os.Exit(1)
		}
		fmt.Println("\nCommitted.")
	}

	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "commit-msg",
		Args: map[string]any{"stage": *stageAll, "commit": *commit},
		Status: "complete", DurationS: time.Since(start).Seconds(), Output: msg,
	})
}

func gitRun(args ...string) string {
	out, _ := exec.Command("git", append([]string{args[0]}, args[1:]...)...).Output()
	return strings.TrimSpace(string(out))
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd /Users/joe/dev/minibox/agentbox
go build ./cmd/mbx-commit-msg/
```

Expected: no errors

- [ ] **Step 3: Commit**

```bash
git add agentbox/cmd/mbx-commit-msg/
git commit -m "feat(agentbox): add standalone mbx-commit-msg binary"
```

---

## Task 15: Build Integration

**Files:**
- Modify: `Justfile`

- [ ] **Step 1: Add agentbox build and run recipes to Justfile**

Add the following recipes to the Justfile:

```just
# ── Agentbox (Go) ─────────────────────────────────────────────────────

# Build all agentbox binaries
agentbox-build:
    cd agentbox && go build ./cmd/agentbox/ && go build ./cmd/mbx-commit-msg/

# Run agentbox tests
agentbox-test:
    cd agentbox && go test ./... -v

# Run council analysis (Go)
agentbox-council *ARGS:
    cd agentbox && go run ./cmd/agentbox/ council {{ARGS}}

# Run meta-agent (Go)
agentbox-meta-agent *ARGS:
    cd agentbox && go run ./cmd/agentbox/ meta-agent {{ARGS}}

# Generate commit message (Go)
agentbox-commit-msg *ARGS:
    cd agentbox && go run ./cmd/mbx-commit-msg/ {{ARGS}}
```

- [ ] **Step 2: Verify recipes work**

```bash
just agentbox-test
```

Expected: all Go tests pass

- [ ] **Step 3: Commit**

```bash
git add Justfile
git commit -m "chore: add agentbox build and test recipes to Justfile"
```

---

## Task 16: Run All Tests and Final Verification

- [ ] **Step 1: Run full Go test suite**

```bash
cd /Users/joe/dev/minibox/agentbox
go test ./... -v -count=1
```

Expected: all tests pass

- [ ] **Step 2: Build all binaries**

```bash
go build ./cmd/agentbox/
go build ./cmd/mbx-commit-msg/
```

Expected: both binaries compile

- [ ] **Step 3: Verify binary runs**

```bash
./agentbox --help 2>&1 || true
./agentbox council --help 2>&1 || true
```

Expected: usage output (not a crash)

- [ ] **Step 4: Run Rust pre-commit to verify nothing is broken**

```bash
cd /Users/joe/dev/minibox
cargo xtask pre-commit
```

Expected: Rust workspace unaffected

- [ ] **Step 5: Final commit with all go.sum updates**

```bash
cd /Users/joe/dev/minibox
git add agentbox/go.sum
git commit -m "chore(agentbox): lock Go module dependencies"
```
