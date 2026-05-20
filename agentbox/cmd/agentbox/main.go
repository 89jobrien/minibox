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

	gitctx "github.com/joe/minibox/agentbox/internal/context"
	"github.com/joe/minibox/agentbox/internal/domain"
	"github.com/joe/minibox/agentbox/internal/llm"
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

// buildRunners creates LLM-backed runners for all configured providers.
// Returns a map of provider name -> runner. Skips providers without API keys.
func buildRunners(ctx context.Context) map[string]domain.AgentRunner {
	runners := make(map[string]domain.AgentRunner)

	if p := llm.NewOpenAIFromEnv(); p != nil {
		runners["openai"] = llm.NewLlmRunner(p)
		fmt.Printf("  [openai] %s\n", p.Name())
	}
	if len(runners) == 0 {
		fmt.Fprintln(os.Stderr, "error: no providers configured. Set OPENAI_API_KEY.")
		os.Exit(1)
	}
	return runners
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
	fmt.Printf("\nCouncil analysis — %s mode · %d roles · vs %s @ %s\n\n",
		*mode, len(roleList), *base, sha)

	fmt.Println("Providers:")
	runners := buildRunners(ctx)

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
		Args: map[string]any{"base": *base, "mode": *mode, "providers": providerNames(runners)},
		Status: "running",
	})
	start := time.Now()

	// Run all providers in parallel, each running all roles.
	type providerResult struct {
		provider    string
		roleOutputs map[string]string
		err         error
	}

	results := make(chan providerResult, len(runners))
	var wg sync.WaitGroup

	for name, runner := range runners {
		wg.Add(1)
		go func(provName string, r domain.AgentRunner) {
			defer wg.Done()
			council := orchestrator.NewCouncil(r)
			roleOutputs, err := council.RunRoles(ctx, roleList, branchCtx)
			results <- providerResult{provider: provName, roleOutputs: roleOutputs, err: err}
		}(name, runner)
	}

	go func() {
		wg.Wait()
		close(results)
	}()

	allProviderOutputs := make(map[string]map[string]string)
	for res := range results {
		if res.err != nil {
			fmt.Fprintf(os.Stderr, "warning: %s council failed: %v\n", res.provider, res.err)
			continue
		}
		allProviderOutputs[res.provider] = res.roleOutputs
	}

	if len(allProviderOutputs) == 0 {
		fmt.Fprintln(os.Stderr, "error: all providers failed")
		os.Exit(1)
	}

	// Print results grouped by provider
	var allOutput []string
	fmt.Println()
	for provider, roleOutputs := range allProviderOutputs {
		header := fmt.Sprintf("═══ %s ", strings.ToUpper(provider))
		fmt.Printf("\n%s%s\n", header, strings.Repeat("═", max(0, 60-len(header))))
		for key, out := range roleOutputs {
			fmt.Printf("\n──── %s %s\n%s\n", key, strings.Repeat("─", max(0, 56-len(key))), out)
			allOutput = append(allOutput, fmt.Sprintf("## [%s] %s\n%s", provider, key, out))
		}
	}

	if !*noSynth {
		// Synthesize across all providers — merge all role outputs
		merged := make(map[string]string)
		for provider, roleOutputs := range allProviderOutputs {
			for key, out := range roleOutputs {
				mergedKey := fmt.Sprintf("%s (%s)", key, provider)
				merged[mergedKey] = out
			}
		}

		// Use the first available runner for synthesis
		var synthRunner domain.AgentRunner
		for _, r := range runners {
			synthRunner = r
			break
		}
		council := orchestrator.NewCouncil(synthRunner)
		synthesis, err := council.RunSynthesis(ctx, merged, branchCtx)
		if err != nil {
			fmt.Fprintf(os.Stderr, "synthesis: %v\n", err)
			os.Exit(1)
		}
		fmt.Printf("\n%s\n  CROSS-PROVIDER SYNTHESIS\n%s\n%s\n\n",
			strings.Repeat("═", 60), strings.Repeat("═", 60), synthesis)
		allOutput = append(allOutput, fmt.Sprintf("## Cross-Provider Synthesis\n%s", synthesis))
	}

	fullOutput := strings.Join(allOutput, "\n\n")
	duration := time.Since(start).Seconds()
	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "council",
		Args:      map[string]any{"base": *base, "mode": *mode, "providers": providerNames(runners)},
		Status:    "complete", DurationS: duration, Output: fullOutput,
	})
	writer.WriteReport(ctx, domain.AgentReport{
		SHA: sha, Script: fmt.Sprintf("council-%s", *mode), Content: fullOutput,
		Meta: map[string]string{"base": *base, "mode": *mode,
			"providers": strings.Join(providerNames(runners), ","),
			"date":      time.Now().Format("2006-01-02 15:04")},
	})
	fmt.Printf("\nDone in %.1fs (%d providers)\n", duration, len(allProviderOutputs))
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

	fmt.Println("Providers:")
	runners := buildRunners(ctx)

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
		Args: map[string]any{"task": truncate(task, 120), "providers": providerNames(runners)},
		Status: "running",
	})
	start := time.Now()

	// Run design + execute + synthesize on all providers in parallel
	type providerResult struct {
		provider string
		output   string
		err      error
	}

	results := make(chan providerResult, len(runners))
	var wg sync.WaitGroup

	for name, runner := range runners {
		wg.Add(1)
		go func(provName string, r domain.AgentRunner) {
			defer wg.Done()
			out, err := runMetaAgentWithRunner(ctx, r, task, repoCtx, *noSynth)
			results <- providerResult{provider: provName, output: out, err: err}
		}(name, runner)
	}

	go func() {
		wg.Wait()
		close(results)
	}()

	var allOutput []string
	for res := range results {
		if res.err != nil {
			fmt.Fprintf(os.Stderr, "warning: %s meta-agent failed: %v\n", res.provider, res.err)
			continue
		}
		header := fmt.Sprintf("═══ %s ", strings.ToUpper(res.provider))
		fmt.Printf("\n%s%s\n%s\n", header, strings.Repeat("═", max(0, 60-len(header))), res.output)
		allOutput = append(allOutput, fmt.Sprintf("## [%s]\n%s", res.provider, res.output))
	}

	if len(allOutput) == 0 {
		fmt.Fprintln(os.Stderr, "error: all providers failed")
		os.Exit(1)
	}

	fullOutput := strings.Join(allOutput, "\n\n")
	duration := time.Since(start).Seconds()
	writer.WriteRun(ctx, domain.AgentRun{
		RunID: runID, Script: "meta-agent",
		Args:      map[string]any{"task": truncate(task, 120), "providers": providerNames(runners)},
		Status:    "complete", DurationS: duration, Output: fullOutput,
	})
	writer.WriteReport(ctx, domain.AgentReport{
		SHA: sha, Script: "meta-agent", Content: fullOutput,
		Meta: map[string]string{"task": truncate(task, 120),
			"providers": strings.Join(providerNames(runners), ","),
			"date":      time.Now().Format("2006-01-02 15:04")},
	})
	fmt.Printf("\nDone in %.1fs\n", duration)
}

func runMetaAgentWithRunner(ctx context.Context, runner domain.AgentRunner, task, repoCtx string, noSynth bool) (string, error) {
	// Phase 1: Design
	designPrompt := orchestrator.DesignerPrompt(task, repoCtx, "")
	designResult, err := runner.Run(ctx, domain.AgentConfig{
		Name: "designer", Prompt: designPrompt,
	})
	if err != nil {
		return "", fmt.Errorf("designer: %w", err)
	}

	plan, err := orchestrator.ParseAgentPlanExported(designResult.Output)
	if err != nil {
		plan = []orchestrator.AgentSpec{{Name: "analyst", Role: "General analysis", Prompt: task}}
	}

	var allOutput []string
	var planLines []string
	for _, a := range plan {
		planLines = append(planLines, fmt.Sprintf("- **%s**: %s", a.Name, a.Role))
	}
	allOutput = append(allOutput, fmt.Sprintf("### Plan (%d agents)\n%s", len(plan), strings.Join(planLines, "\n")))

	// Phase 2: Execute
	meta := orchestrator.NewMetaAgent(runner)
	agentOutputs, err := meta.RunParallel(ctx, plan)
	if err != nil {
		return "", fmt.Errorf("agents: %w", err)
	}

	for name, out := range agentOutputs {
		allOutput = append(allOutput, fmt.Sprintf("### %s\n%s", name, out))
	}

	// Phase 3: Synthesize
	if !noSynth {
		synthPrompt := orchestrator.MetaSynthesisPrompt(task, agentOutputs)
		synthResult, err := runner.Run(ctx, domain.AgentConfig{
			Name: "synthesis", Prompt: synthPrompt,
		})
		if err != nil {
			return "", fmt.Errorf("synthesis: %w", err)
		}
		allOutput = append(allOutput, fmt.Sprintf("### Synthesis\n%s", synthResult.Output))
	}

	return strings.Join(allOutput, "\n\n"), nil
}

func providerNames(runners map[string]domain.AgentRunner) []string {
	names := make([]string, 0, len(runners))
	for name := range runners {
		names = append(names, name)
	}
	return names
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n]
}
