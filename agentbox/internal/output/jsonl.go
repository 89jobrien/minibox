package output

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// JSONLWriter writes AgentRun records as JSONL.
type JSONLWriter struct {
	path string
}

// NewJSONLWriter creates a writer targeting the given file path.
func NewJSONLWriter(path string) *JSONLWriter {
	return &JSONLWriter{path: path}
}

func (w *JSONLWriter) WriteRun(_ context.Context, run domain.AgentRun) error {
	if err := os.MkdirAll(filepath.Dir(w.path), 0o755); err != nil {
		return err
	}
	f, err := os.OpenFile(w.path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return err
	}
	defer f.Close()
	data, err := json.Marshal(run)
	if err != nil {
		return err
	}
	_, err = f.Write(append(data, '\n'))
	return err
}
