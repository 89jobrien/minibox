package synthesis

import (
	"testing"
)

func TestQualityRankerEmpty(t *testing.T) {
	r := &QualityRanker{}
	out, prov := r.Select(map[string]string{})
	if out != "" || prov != "" {
		t.Errorf("expected empty, got %q from %q", out, prov)
	}
}

func TestQualityRankerSingleCandidate(t *testing.T) {
	r := &QualityRanker{}
	out, prov := r.Select(map[string]string{"openai": "hello world"})
	if out != "hello world" || prov != "openai" {
		t.Errorf("got %q from %q", out, prov)
	}
}

func TestQualityRankerPrefersStructuredOutput(t *testing.T) {
	r := &QualityRanker{}
	candidates := map[string]string{
		"short": "ok",
		"structured": "# Analysis\n\n- Point one about the code\n- Point two about the design\n- Point three about testing\n\n## Details\n\nThis is a longer response with more substance and detail about the changes.",
	}
	_, prov := r.Select(candidates)
	if prov != "structured" {
		t.Errorf("expected structured provider, got %q", prov)
	}
}

func TestQualityRankerPenalizesRefusal(t *testing.T) {
	r := &QualityRanker{}
	candidates := map[string]string{
		"good":    "# Review\n\n- The code looks correct\n- Tests pass\n- No issues found with the implementation details here",
		"refused": "I apologize, but as an AI I cannot review this code properly. I'm unable to provide feedback.",
	}
	_, prov := r.Select(candidates)
	if prov != "good" {
		t.Errorf("expected good provider, got %q", prov)
	}
}

func TestMajorityVoterWithMajority(t *testing.T) {
	v := NewMajorityVoter()
	candidates := map[string]string{
		"a": "same answer",
		"b": "same answer",
		"c": "different",
	}
	out, _ := v.Select(candidates)
	if out != "same answer" {
		t.Errorf("expected majority answer, got %q", out)
	}
}

func TestMajorityVoterNoMajorityFallsBackToQuality(t *testing.T) {
	v := NewMajorityVoter()
	candidates := map[string]string{
		"short":      "ok",
		"structured": "# Good analysis\n\n- Point one\n- Point two\n\nThis has more substance.",
	}
	_, prov := v.Select(candidates)
	if prov != "structured" {
		t.Errorf("expected quality fallback to pick structured, got %q", prov)
	}
}

func TestMajorityVoterSingleCandidate(t *testing.T) {
	v := NewMajorityVoter()
	out, prov := v.Select(map[string]string{"only": "result"})
	if out != "result" || prov != "only" {
		t.Errorf("got %q from %q", out, prov)
	}
}

func TestFirstSuccessPicksNonEmpty(t *testing.T) {
	f := &FirstSuccess{}
	out, _ := f.Select(map[string]string{"a": "", "b": "good"})
	if out != "good" {
		t.Errorf("expected good, got %q", out)
	}
}

func TestFirstSuccessAllEmpty(t *testing.T) {
	f := &FirstSuccess{}
	out, prov := f.Select(map[string]string{"a": "", "b": "  "})
	if out != "" || prov != "" {
		t.Errorf("expected empty, got %q from %q", out, prov)
	}
}

func TestDefaultReturnsQualityRanker(t *testing.T) {
	s := Default()
	if _, ok := s.(*QualityRanker); !ok {
		t.Errorf("Default() should return *QualityRanker, got %T", s)
	}
}

func TestScoreOutputWordCount(t *testing.T) {
	short := scoreOutput("hello")
	long := scoreOutput("word " + repeat("more ", 200))
	if long <= short {
		t.Errorf("longer output should score higher: short=%d, long=%d", short, long)
	}
}

func repeat(s string, n int) string {
	out := ""
	for i := 0; i < n; i++ {
		out += s
	}
	return out
}
