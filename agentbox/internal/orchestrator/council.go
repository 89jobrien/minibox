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
