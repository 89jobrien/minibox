package output

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// ReportWriter writes markdown reports to a directory.
type ReportWriter struct {
	dir string
}

// NewReportWriter creates a writer targeting the given directory.
func NewReportWriter(dir string) *ReportWriter {
	return &ReportWriter{dir: dir}
}

func (w *ReportWriter) WriteReport(_ context.Context, report domain.AgentReport) error {
	if err := os.MkdirAll(w.dir, 0o755); err != nil {
		return err
	}
	path := filepath.Join(w.dir, fmt.Sprintf("%s-%s.md", report.SHA, report.Script))

	var metaLines []string
	for k, v := range report.Meta {
		metaLines = append(metaLines, fmt.Sprintf("- **%s**: %s", k, v))
	}
	header := fmt.Sprintf("# %s · %s\n\n%s\n\n---\n\n",
		report.Script, report.SHA, strings.Join(metaLines, "\n"))

	return os.WriteFile(path, []byte(header+report.Content), 0o644)
}
