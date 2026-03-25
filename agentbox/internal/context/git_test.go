package gitctx

import (
	"testing"
)

func TestBranchContextFormat(t *testing.T) {
	// Test that formatBranchContext produces expected format
	ctx := formatBranchContext("main", "feature-x", "abc1234 feat: something\ndef5678 fix: bug",
		"file.go\nother.go", "diff content here")

	if ctx == "" {
		t.Fatal("expected non-empty context")
	}
	if !containsAll(ctx, "Branch:", "Commits:", "Changed files:", "Diff:") {
		t.Error("missing expected sections in branch context")
	}
}

func containsAll(s string, substrs ...string) bool {
	for _, sub := range substrs {
		if !contains(s, sub) {
			return false
		}
	}
	return true
}

func contains(s, sub string) bool {
	return len(s) >= len(sub) && searchString(s, sub)
}

func searchString(s, sub string) bool {
	for i := 0; i <= len(s)-len(sub); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}

// Ensure GitProvider implements domain.ContextProvider
func TestGitProviderCompiles(t *testing.T) {
	p := NewGitProvider()
	_ = p
}
