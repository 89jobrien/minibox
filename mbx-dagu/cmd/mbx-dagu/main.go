// mbx-dagu is a CLI that submits a container job to mbxctl and waits for completion.
//
// Designed to be used as a dagu step via the `command` executor type:
//
//	steps:
//	  - name: run-alpine
//	    command: mbx-dagu --image alpine --tag latest -- /bin/echo hello
//
// Exit code mirrors the container's exit code (0 = success, non-zero = failure).
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/89jobrien/minibox/mbx-dagu/internal/client"
)

func main() {
	os.Exit(run())
}

func run() int {
	fs := flag.NewFlagSet("mbx-dagu", flag.ContinueOnError)
	mbxctlURL := fs.String("mbxctl", envOr("MBXCTL_URL", "http://localhost:9999"), "mbxctl base URL")
	image := fs.String("image", "", "container image name (required)")
	tag := fs.String("tag", "latest", "image tag")
	memLimit := fs.Int64("memory", 0, "memory limit in bytes (0 = unlimited)")
	cpuWeight := fs.Int64("cpu-weight", 0, "CPU weight (0 = default)")
	timeout := fs.Duration("timeout", 1*time.Hour, "job timeout")
	envVars := fs.String("env", "", "comma-separated KEY=VALUE env vars")

	if err := fs.Parse(os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "mbx-dagu: %v\n", err)
		return 2
	}

	if *image == "" {
		fmt.Fprintln(os.Stderr, "mbx-dagu: --image is required")
		fs.Usage()
		return 2
	}

	command := fs.Args()
	if len(command) == 0 {
		fmt.Fprintln(os.Stderr, "mbx-dagu: command is required (pass after --)")
		return 2
	}

	req := client.CreateJobRequest{
		Image:   *image,
		Command: command,
	}
	if *tag != "" {
		req.Tag = tag
	}
	if *memLimit > 0 {
		req.MemoryLimitBytes = memLimit
	}
	if *cpuWeight > 0 {
		req.CPUWeight = cpuWeight
	}
	if *envVars != "" {
		req.Env = strings.Split(*envVars, ",")
	}

	c := client.New(*mbxctlURL)
	ctx, cancel := context.WithTimeout(context.Background(), *timeout)
	defer cancel()

	job, err := c.CreateJob(ctx, req)
	if err != nil {
		fmt.Fprintf(os.Stderr, "mbx-dagu: create job: %v\n", err)
		return 1
	}
	fmt.Printf("mbx-dagu: job %s started (container %s)\n", job.JobID, job.ContainerID)

	status, err := c.WaitForJob(ctx, job.JobID, 0)
	if err != nil {
		fmt.Fprintf(os.Stderr, "mbx-dagu: wait: %v\n", err)
		return 1
	}

	fmt.Printf("mbx-dagu: job %s finished with status %s\n", job.JobID, status.Status)
	if status.ExitCode != nil {
		return *status.ExitCode
	}
	if status.Status == "completed" {
		return 0
	}
	return 1
}

func envOr(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}
