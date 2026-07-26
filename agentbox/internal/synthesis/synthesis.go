package synthesis

import (
	"sort"
	"strings"
)

// Strategy selects the best result from multiple provider outputs.
type Strategy interface {
	// Select picks the best output from the given candidates.
	// Each candidate is a provider name -> output string.
	// Returns the selected output and the provider name that produced it.
	Select(candidates map[string]string) (output string, provider string)
}

// QualityRanker scores candidates by heuristic quality signals and picks
// the highest-scoring one. Falls back to the first candidate if all score
// equally.
type QualityRanker struct{}

// Select picks the highest-quality candidate based on structural signals.
func (q *QualityRanker) Select(candidates map[string]string) (string, string) {
	if len(candidates) == 0 {
		return "", ""
	}
	if len(candidates) == 1 {
		for provider, output := range candidates {
			return output, provider
		}
	}

	type scored struct {
		provider string
		output   string
		score    int
	}

	var entries []scored
	for provider, output := range candidates {
		entries = append(entries, scored{
			provider: provider,
			output:   output,
			score:    scoreOutput(output),
		})
	}

	// Stable sort by score descending, then provider name ascending for
	// determinism.
	sort.Slice(entries, func(i, j int) bool {
		if entries[i].score != entries[j].score {
			return entries[i].score > entries[j].score
		}
		return entries[i].provider < entries[j].provider
	})

	return entries[0].output, entries[0].provider
}

// scoreOutput assigns a heuristic quality score to an LLM output.
// Higher is better. Signals: length, structure (headings, lists),
// and absence of error/refusal markers.
func scoreOutput(text string) int {
	score := 0

	// Prefer longer, more substantive outputs (diminishing returns).
	words := len(strings.Fields(text))
	switch {
	case words > 500:
		score += 30
	case words > 200:
		score += 20
	case words > 50:
		score += 10
	}

	// Reward structural signals.
	lines := strings.Split(text, "\n")
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "#") {
			score += 3 // markdown heading
		}
		if strings.HasPrefix(trimmed, "- ") || strings.HasPrefix(trimmed, "* ") {
			score += 1 // list item
		}
		if strings.HasPrefix(trimmed, "```") {
			score += 2 // code block
		}
	}

	// Penalize refusal/error markers.
	lower := strings.ToLower(text)
	refusals := []string{
		"i cannot", "i'm unable", "i apologize", "as an ai",
		"error:", "failed to",
	}
	for _, marker := range refusals {
		if strings.Contains(lower, marker) {
			score -= 10
		}
	}

	return score
}

// MajorityVoter finds the most common output (by exact match) among
// candidates. When no majority exists, falls back to QualityRanker.
type MajorityVoter struct {
	fallback Strategy
}

// NewMajorityVoter creates a voter that falls back to QualityRanker
// when no majority exists.
func NewMajorityVoter() *MajorityVoter {
	return &MajorityVoter{fallback: &QualityRanker{}}
}

// Select picks the output that appears most often. If all outputs are
// unique, delegates to the quality ranker.
func (m *MajorityVoter) Select(candidates map[string]string) (string, string) {
	if len(candidates) <= 1 {
		return (&QualityRanker{}).Select(candidates)
	}

	// Count normalized occurrences.
	type vote struct {
		output   string
		provider string
		count    int
	}
	counts := make(map[string]*vote)
	for provider, output := range candidates {
		key := strings.TrimSpace(output)
		if v, ok := counts[key]; ok {
			v.count++
		} else {
			counts[key] = &vote{output: output, provider: provider, count: 1}
		}
	}

	// Find max count.
	var best *vote
	for _, v := range counts {
		if best == nil || v.count > best.count {
			best = v
		}
	}

	// If no actual majority (all unique), fall back to quality ranking.
	if best != nil && best.count > 1 {
		return best.output, best.provider
	}

	return m.fallback.Select(candidates)
}

// FirstSuccess returns the first non-empty candidate. This preserves the
// legacy behavior as a fallback strategy.
type FirstSuccess struct{}

// Select returns the first non-empty output found.
func (f *FirstSuccess) Select(candidates map[string]string) (string, string) {
	for provider, output := range candidates {
		if strings.TrimSpace(output) != "" {
			return output, provider
		}
	}
	return "", ""
}

// Default returns the default synthesis strategy: quality ranking.
func Default() Strategy {
	return &QualityRanker{}
}
