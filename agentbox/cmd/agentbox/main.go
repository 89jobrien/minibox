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
		Args:      map[string]any{"base": *base, "mode": *mode},
		Status:    "complete", DurationS: duration, Output: fullOutput,
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
		Args:      map[string]any{"task": truncate(task, 120)},
		Status:    "complete", DurationS: duration, Output: fullOutput,
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
