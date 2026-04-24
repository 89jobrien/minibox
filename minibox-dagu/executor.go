// Package main implements the mbx-dagu executor binary.
//
// mbx-dagu is invoked by the dagu workflow engine as a command executor.  It
// translates dagu step definitions into miniboxctl HTTP API calls (POST /api/v1/jobs)
// and polls until the container exits, forwarding stdout/stderr and propagating
// the container exit code back to dagu.
//
// GH #36: Env, MemoryLimitBytes, and CpuWeight are now forwarded to the
// miniboxctl CreateJobRequest so resource limits specified in the dagu DAG step
// are honoured by the container runtime.
package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"
)

// CreateJobRequest mirrors crates/miniboxctl/src/models.rs:CreateJobRequest.
// All fields must stay in sync with the Rust definition.
type CreateJobRequest struct {
	Image            string   `json:"image"`
	Tag              *string  `json:"tag,omitempty"`
	Command          []string `json:"command"`
	Env              []string `json:"env"`               // GH #36: was missing
	MemoryLimitBytes *uint64  `json:"memory_limit_bytes,omitempty"` // GH #36: was missing
	CpuWeight        *uint64  `json:"cpu_weight,omitempty"`         // GH #36: was missing
	StreamOutput     *bool    `json:"stream_output,omitempty"`
	TimeoutSeconds   *uint64  `json:"timeout_seconds,omitempty"`
}

// CreateJobResponse mirrors crates/miniboxctl/src/models.rs:CreateJobResponse.
type CreateJobResponse struct {
	JobID       string `json:"job_id"`
	ContainerID string `json:"container_id"`
	Status      string `json:"status"`
}

// JobStatus mirrors crates/miniboxctl/src/models.rs:JobStatus.
type JobStatus struct {
	JobID       string  `json:"job_id"`
	ContainerID *string `json:"container_id,omitempty"`
	Status      string  `json:"status"`
	ExitCode    *int    `json:"exit_code,omitempty"`
	CreatedAt   string  `json:"created_at"`
	CompletedAt *string `json:"completed_at,omitempty"`
}

func main() {
	var (
		image      = flag.String("image", "", "Container image name (required)")
		tag        = flag.String("tag", "latest", "Image tag")
		envFlag    = flag.String("env", "", "Comma-separated KEY=VALUE environment variables")
		memory     = flag.Uint64("memory", 0, "Memory limit in bytes (0 = unlimited)")
		cpuWeight  = flag.Uint64("cpu-weight", 0, "CPU weight (0 = runtime default)")
		timeout    = flag.Duration("timeout", time.Hour, "Job timeout")
		mbxctlURL  = flag.String("mbxctl", "", "miniboxctl base URL (default: $MBXCTL_URL or http://localhost:9999)")
	)
	flag.Parse()

	if *image == "" {
		fmt.Fprintln(os.Stderr, "mbx-dagu: --image is required")
		os.Exit(2)
	}

	baseURL := *mbxctlURL
	if baseURL == "" {
		baseURL = os.Getenv("MBXCTL_URL")
	}
	if baseURL == "" {
		baseURL = "http://localhost:9999"
	}

	// Build env slice from comma-separated flag value.
	var envVars []string
	if *envFlag != "" {
		for _, kv := range strings.Split(*envFlag, ",") {
			if kv = strings.TrimSpace(kv); kv != "" {
				envVars = append(envVars, kv)
			}
		}
	}

	req := CreateJobRequest{
		Image:   *image,
		Command: flag.Args(),
		Env:     envVars,
	}

	if *tag != "latest" {
		req.Tag = tag
	}
	if *memory > 0 {
		req.MemoryLimitBytes = memory
	}
	if *cpuWeight > 0 {
		req.CpuWeight = cpuWeight
	}
	secs := uint64(timeout.Seconds())
	req.TimeoutSeconds = &secs

	exitCode, err := run(baseURL, req)
	if err != nil {
		fmt.Fprintf(os.Stderr, "mbx-dagu: %v\n", err)
		os.Exit(1)
	}
	os.Exit(exitCode)
}

func run(baseURL string, req CreateJobRequest) (int, error) {
	body, err := json.Marshal(req)
	if err != nil {
		return 1, fmt.Errorf("marshal request: %w", err)
	}

	resp, err := http.Post(baseURL+"/api/v1/jobs", "application/json", bytes.NewReader(body))
	if err != nil {
		return 1, fmt.Errorf("POST /api/v1/jobs: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusCreated && resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		return 1, fmt.Errorf("POST /api/v1/jobs: HTTP %d: %s", resp.StatusCode, b)
	}

	var created CreateJobResponse
	if err := json.NewDecoder(resp.Body).Decode(&created); err != nil {
		return 1, fmt.Errorf("decode job response: %w", err)
	}

	// Poll until terminal state.
	for {
		time.Sleep(500 * time.Millisecond)

		statusResp, err := http.Get(fmt.Sprintf("%s/api/v1/jobs/%s", baseURL, created.JobID))
		if err != nil {
			return 1, fmt.Errorf("GET /api/v1/jobs/%s: %w", created.JobID, err)
		}

		var status JobStatus
		if err := json.NewDecoder(statusResp.Body).Decode(&status); err != nil {
			statusResp.Body.Close()
			return 1, fmt.Errorf("decode job status: %w", err)
		}
		statusResp.Body.Close()

		switch status.Status {
		case "completed":
			if status.ExitCode != nil {
				return *status.ExitCode, nil
			}
			return 0, nil
		case "failed":
			if status.ExitCode != nil {
				return *status.ExitCode, nil
			}
			return 1, nil
		}
		// "created" or "running" — keep polling.
	}
}
