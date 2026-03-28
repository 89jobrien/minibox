// Package client provides a minimal HTTP client for the mbxctl control plane.
package client

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"
)

// Client talks to an mbxctl HTTP API.
type Client struct {
	baseURL    string
	httpClient *http.Client
}

// New returns a Client targeting baseURL (e.g. "http://localhost:9999").
func New(baseURL string) *Client {
	return &Client{
		baseURL: baseURL,
		httpClient: &http.Client{Timeout: 30 * time.Second},
	}
}

// CreateJob submits a container job and returns the response.
func (c *Client) CreateJob(ctx context.Context, req CreateJobRequest) (*CreateJobResponse, error) {
	body, err := json.Marshal(req)
	if err != nil {
		return nil, fmt.Errorf("marshal request: %w", err)
	}

	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, c.baseURL+"/jobs", bytes.NewReader(body))
	if err != nil {
		return nil, fmt.Errorf("build request: %w", err)
	}
	httpReq.Header.Set("Content-Type", "application/json")

	resp, err := c.httpClient.Do(httpReq)
	if err != nil {
		return nil, fmt.Errorf("POST /jobs: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		b, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("POST /jobs returned %d: %s", resp.StatusCode, b)
	}

	var out CreateJobResponse
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		return nil, fmt.Errorf("decode response: %w", err)
	}
	return &out, nil
}

// GetJob returns the current status of a job by ID.
func (c *Client) GetJob(ctx context.Context, jobID string) (*JobStatus, error) {
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodGet, c.baseURL+"/jobs/"+jobID, nil)
	if err != nil {
		return nil, fmt.Errorf("build request: %w", err)
	}

	resp, err := c.httpClient.Do(httpReq)
	if err != nil {
		return nil, fmt.Errorf("GET /jobs/%s: %w", jobID, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("GET /jobs/%s returned %d: %s", jobID, resp.StatusCode, b)
	}

	var out JobStatus
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		return nil, fmt.Errorf("decode response: %w", err)
	}
	return &out, nil
}

// WaitForJob polls until the job reaches a terminal state or ctx is cancelled.
// pollInterval defaults to 2 s if zero.
func (c *Client) WaitForJob(ctx context.Context, jobID string, pollInterval time.Duration) (*JobStatus, error) {
	if pollInterval == 0 {
		pollInterval = 2 * time.Second
	}
	for {
		status, err := c.GetJob(ctx, jobID)
		if err != nil {
			return nil, err
		}
		switch status.Status {
		case "completed", "failed", "stopped":
			return status, nil
		}
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-time.After(pollInterval):
		}
	}
}
