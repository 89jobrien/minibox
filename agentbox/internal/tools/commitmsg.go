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
		"  Scope: crate name, module, or area (e.g. minibox, standup, justfile)\n" +
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
