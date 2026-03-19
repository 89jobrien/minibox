#!/usr/bin/env bash
# Install git hooks for minibox
set -euo pipefail

echo "Installing git hooks..."

# ============================================================================
# Pre-commit hook
# ============================================================================
cat > .git/hooks/pre-commit << 'EOF'
#!/bin/sh
# minibox pre-commit hook
echo "Running pre-commit checks..."
just pre-commit
EOF

chmod +x .git/hooks/pre-commit

# ============================================================================
# Pre-push hook — nextest reuses pre-commit build artifacts, no recompile
# ============================================================================
cat > .git/hooks/pre-push << 'EOF'
#!/bin/sh
echo "Running pre-push checks..."
just nextest
EOF

chmod +x .git/hooks/pre-push

# ============================================================================
# Commit-msg hook (enforce conventional commits — warning only)
# ============================================================================
cat > .git/hooks/commit-msg << 'EOF'
#!/bin/sh
commit_msg_file=$1
commit_msg=$(cat "$commit_msg_file")

# Allow merge commits and reverts
if echo "$commit_msg" | grep -qE "^(Merge|Revert)"; then
    exit 0
fi

# Check format (warn only, does not fail)
if ! echo "$commit_msg" | grep -qE "^(feat|fix|docs|style|refactor|test|chore|perf|ci|build|obs)(\(.+\))?: .+"; then
    echo "Warning: Commit message doesn't follow conventional format"
    echo "Recommended format: type(scope): subject"
    echo "Types: feat, fix, docs, style, refactor, test, chore, perf, ci, build, obs"
    echo
    echo "Your commit message:"
    echo "$commit_msg"
    echo
fi

exit 0
EOF

chmod +x .git/hooks/commit-msg

echo "Git hooks installed successfully!"
echo
echo "Installed hooks:"
echo "  - pre-commit: fmt-check + lint + test-unit"
echo "  - pre-push:   nextest (fast, reuses pre-commit artifacts)"
echo "  - commit-msg: conventional commit format (warning only)"
echo
echo "To bypass hooks (not recommended):"
echo "  git commit --no-verify"
