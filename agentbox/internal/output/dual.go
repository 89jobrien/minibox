package output

import (
	"context"
	"os"
	"path/filepath"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// DualWriter writes to both JSONL and markdown report sinks.
type DualWriter struct {
	jsonl  *JSONLWriter
	report *ReportWriter
}

// NewDualWriter creates a writer targeting ~/.minibox/ paths.
func NewDualWriter() *DualWriter {
	home, _ := os.UserHomeDir()
	return &DualWriter{
		jsonl:  NewJSONLWriter(filepath.Join(home, ".minibox", "agent-runs.jsonl")),
		report: NewReportWriter(filepath.Join(home, ".minibox", "ai-logs")),
	}
}

func (w *DualWriter) WriteRun(ctx context.Context, run domain.AgentRun) error {
	return w.jsonl.WriteRun(ctx, run)
}

func (w *DualWriter) WriteReport(ctx context.Context, report domain.AgentReport) error {
	return w.report.WriteReport(ctx, report)
}
