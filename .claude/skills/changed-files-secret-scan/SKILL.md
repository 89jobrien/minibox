---
name: changed-files-secret-scan
description: Use when scanning a dirty git tree for secrets before staging, committing, or pushing. Symptoms - broad gitleaks scans report old/history findings, TruffleHog needs redacted output, or zsh wrappers break because of special variable names.
---

# Changed Files Secret Scan

## When to Use

Use before staging/committing when the repo has unrelated dirty files or historical secret-scan findings. Scans only changed and untracked files, redacts scanner output, and avoids printing raw secret values.

## Commands

### Snapshot changed files

```bash
git status --short
git diff --stat HEAD
```

### TruffleHog changed-file scan with sanitized output

```bash
scan_out=$(mktemp -t changed-trufflehog-jsonl)
paths_file=$(mktemp -t changed-trufflehog-paths)

{ git diff --name-only HEAD --; git ls-files --others --exclude-standard; } \
  | awk 'NF && !seen[$0]++' > "$paths_file"

if [ -s "$paths_file" ]; then
  trufflehog filesystem --json --no-update \
    --results=verified,unknown,unverified \
    $(cat "$paths_file") > "$scan_out"

  if [ -s "$scan_out" ]; then
    jq -c '{
      SourceMetadata,
      SourceName,
      DetectorName,
      DetectorType,
      Verified,
      VerificationError,
      ExtraData: (.ExtraData // {})
    }' "$scan_out"
  else
    printf '%s\n' 'trufflehog: no findings'
  fi
else
  printf '%s\n' 'trufflehog: no changed files to scan'
fi

rm -f "$scan_out" "$paths_file"
```

### Gitleaks changed-file scan via temporary copy

```bash
tmpdir=$(mktemp -d -t changed-gitleaks)
paths_file=$(mktemp -t changed-gitleaks-paths)
report_file=$(mktemp -t changed-gitleaks-report)

{ git diff --name-only HEAD --; git ls-files --others --exclude-standard; } \
  | awk 'NF && !seen[$0]++' > "$paths_file"

if [ -s "$paths_file" ]; then
  while IFS= read -r filepath; do
    if [ -f "$filepath" ]; then
      dir=$(dirname "$filepath")
      mkdir -p "$tmpdir/$dir"
      cp "$filepath" "$tmpdir/$filepath"
    fi
  done < "$paths_file"

  gitleaks detect --source "$tmpdir" --no-git --redact --no-banner \
    --report-format json --report-path "$report_file" --exit-code 0

  if [ -s "$report_file" ] && [ "$(jq 'length' "$report_file")" -gt 0 ]; then
    jq -c '[.[] | {
      RuleID,
      Description,
      File,
      StartLine,
      EndLine,
      Commit,
      Secret: "REDACTED"
    }]' "$report_file"
  else
    printf '%s\n' 'gitleaks changed-files: no findings'
  fi
else
  printf '%s\n' 'gitleaks changed-files: no changed files'
fi

rm -rf "$tmpdir" "$paths_file" "$report_file"
```

### Restore PATH if zsh command lookup breaks

Avoid loop variables named `path` or `status` in zsh. If command lookup breaks:

```bash
PATH="/opt/homebrew/bin:$HOME/.nix-profile/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
export PATH
command -v mkdir
command -v gitleaks
command -v trufflehog
```

## Common Failures

| Symptom | Fix |
|---------|-----|
| Broad `gitleaks detect --source .` reports old findings | Run changed-file-only scan via temp copy before deciding whether candidate commit is safe |
| Scanner output might print raw secrets | Use TruffleHog JSON piped through `jq` metadata projection and Gitleaks `--redact` |
| `zsh: read-only variable: status` | Use `rc` for exit code, not `status` |
| `zsh: command not found: mkdir/cp/gitleaks` after a loop | You probably assigned to zsh's special `path` array; use `filepath` and restore `PATH` |
| `mktemp: ... File exists` on macOS | Use `mktemp -t name`, not GNU-style `/tmp/name.XXXXXX.ext` templates |
| Unrelated dirty files exist | Stage explicit file paths only; do not use `git add -A` |
