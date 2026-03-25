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
					Args:      map[string]any{"stage": *stageAll, "commit": false},
					Status:    "complete", DurationS: time.Since(start).Seconds(), Output: msg,
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
		Args:      map[string]any{"stage": *stageAll, "commit": *commit},
		Status:    "complete", DurationS: time.Since(start).Seconds(), Output: msg,
	})
}

func gitRun(args ...string) string {
	out, _ := exec.Command(args[0], args[1:]...).Output()
	return strings.TrimSpace(string(out))
}
