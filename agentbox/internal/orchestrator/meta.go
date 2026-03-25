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

// ParseAgentPlanExported is the exported version of parseAgentPlan.
func ParseAgentPlanExported(raw string) ([]AgentSpec, error) {
	return parseAgentPlan(raw)
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

// contains checks if s contains sub (used in tests via package-level access).
func contains(s, sub string) bool {
	return strings.Contains(s, sub)
}
