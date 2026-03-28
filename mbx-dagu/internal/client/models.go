package client

// CreateJobRequest matches mbxctl's POST /jobs body.
type CreateJobRequest struct {
	Image              string   `json:"image"`
	Tag                *string  `json:"tag,omitempty"`
	Command            []string `json:"command"`
	MemoryLimitBytes   *int64   `json:"memory_limit_bytes,omitempty"`
	CPUWeight          *int64   `json:"cpu_weight,omitempty"`
	StreamOutput       *bool    `json:"stream_output,omitempty"`
	TimeoutSeconds     *int64   `json:"timeout_seconds,omitempty"`
	Env                []string `json:"env,omitempty"`
}

// CreateJobResponse matches mbxctl's POST /jobs response.
type CreateJobResponse struct {
	JobID       string `json:"job_id"`
	ContainerID string `json:"container_id"`
	Status      string `json:"status"`
}

// JobStatus matches mbxctl's GET /jobs/:id response.
type JobStatus struct {
	JobID       string  `json:"job_id"`
	ContainerID *string `json:"container_id,omitempty"`
	Status      string  `json:"status"`
	ExitCode    *int    `json:"exit_code,omitempty"`
	CreatedAt   string  `json:"created_at"`
	CompletedAt *string `json:"completed_at,omitempty"`
}
