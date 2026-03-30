#!/usr/bin/env nu
# Install git hooks: pre-commit, pre-push, commit-msg

def main [] {
    let hooks_dir = ".git/hooks"

    if not ($hooks_dir | path exists) {
        error make { msg: ".git/hooks not found — run from the repo root" }
    }

    # pre-commit
    let pre_commit = $"($hooks_dir)/pre-commit"
    "#!/bin/sh\nset -e\njust pre-commit\n" | save --force $pre_commit
    ^chmod +x $pre_commit
    print $"installed ($pre_commit)"

    # pre-push
    let pre_push = $"($hooks_dir)/pre-push"
    "#!/bin/sh\nset -e\njust prepush\n" | save --force $pre_push
    ^chmod +x $pre_push
    print $"installed ($pre_push)"

    # commit-msg — validates conventional commit format (warning only)
    let commit_msg_hook = $"($hooks_dir)/commit-msg"
    '#!/bin/sh
MSG=$(cat "$1")
# Allow merge commits and reverts
echo "$MSG" | grep -qE "^(Merge |Revert )" && exit 0
# Warn if message does not match conventional commit pattern
if ! echo "$MSG" | grep -qE "^(feat|fix|docs|chore|refactor|test|perf|ci|build|style|revert)(\(.+\))?: .{1,72}"; then
    echo "warning: commit message does not follow conventional commits format"
    echo "  expected: type(scope): description"
    echo "  e.g.: feat(macbox): add VZ.framework adapter"
fi
exit 0
' | save --force $commit_msg_hook
    ^chmod +x $commit_msg_hook
    print $"installed ($commit_msg_hook)"

    print "\nhooks installed ✓"
}
