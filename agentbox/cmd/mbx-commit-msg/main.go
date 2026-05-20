package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"

	"github.com/joe/minibox/agentbox/internal/config"
	"github.com/joe/minibox/agentbox/internal/domain"
	"github.com/joe/minibox/agentbox/internal/llm"
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

	stagedDiff := gitRun("git", "diff", "--cached")
	stagedStat := gitRun("git", "diff", "--cached", "--stat")
	if strings.TrimSpace(stagedDiff) == "" {
		status := gitRun("git", "status", "--short")
		if status != "" {
			fmt.Println("Nothing staged. Use -a to stage all, or `git add` files first.")
		} else {
			fmt.Println("Working tree is clean — nothing to commit.")
		}
		os.Exit(1)
	}

	ctx := context.Background()
	writer := output.NewDualWriter()

	// Build runners from env
	runners := buildRunners(ctx)

	input := tools.CommitMsgContext{
		Branch:       gitRun("git", "rev-parse", "--abbrev-ref", "HEAD"),
		StagedDiff:   stagedDiff,
		StagedStat:   stagedStat,
		UnstagedStat: gitRun("git", "diff", "--stat"),
		RecentLog:    gitRun("git", "log", "-8", "--oneline"),
		Status:       gitRun("git", "status", "--short"),
	}

	fmt.Printf("Generating commit message (%d providers)...\n\n", len(runners))

	runID := time.Now().Format(time.RFC3339)
	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "commit-msg",
		Args: map[string]any{"stage": *stageAll, "commit": *commit}, Status: "running",
	})
	start := time.Now()

	// Run all providers in parallel
	type result struct {
		provider string
		msg      string
		err      error
	}
	results := make(chan result, len(runners))
	var wg sync.WaitGroup

	for name, runner := range runners {
		wg.Add(1)
		go func(provName string, r domain.AgentRunner) {
			defer wg.Done()
			cm := tools.NewCommitMsg(r)
			msg, err := cm.Generate(ctx, input)
			results <- result{provider: provName, msg: msg, err: err}
		}(name, runner)
	}

	go func() {
		wg.Wait()
		close(results)
	}()

	var msgs []string
	for res := range results {
		if res.err != nil {
			fmt.Fprintf(os.Stderr, "warning: %s failed: %v\n", res.provider, res.err)
			continue
		}
		fmt.Printf("──── %s %s\n%s\n\n", res.provider, strings.Repeat("─", max(0, 56-len(res.provider))), res.msg)
		msgs = append(msgs, res.msg)
	}

	if len(msgs) == 0 {
		fmt.Fprintln(os.Stderr, "error: all providers failed")
		os.Exit(1)
	}

	// Use the first result as the commit message
	msg := msgs[0]

	if *commit {
		if !*yes {
			fmt.Print("\nCommit with the first message? [y/N] ")
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

		result := exec.Command("git", "commit", "-m", msg)
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

func buildRunners(ctx context.Context) map[string]domain.AgentRunner {
	cfg := config.LoadFromEnv()
	retryCfg := llm.DefaultRetryConfig()
	runners := make(map[string]domain.AgentRunner)

	if p := llm.NewOpenAIFromConfig(cfg); p != nil {
		runners["openai"] = llm.NewLlmRunner(llm.NewRetryingProvider(p, retryCfg))
	}
	if len(runners) == 0 {
		fmt.Fprintln(os.Stderr, "error: no providers configured. Set OPENAI_API_KEY.")
		os.Exit(1)
	}
	return runners
}

func gitRun(args ...string) string {
	out, _ := exec.Command(args[0], args[1:]...).Output()
	return strings.TrimSpace(string(out))
}
