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
