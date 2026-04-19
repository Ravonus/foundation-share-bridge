#!/usr/bin/env bash
# Fail if any Rust source file exceeds 600 lines.
# Warn (exit 0) if any file exceeds 400 lines.
# Usage: bash scripts/lint/check-file-size.sh

set -euo pipefail

HARD_LIMIT=600
SOFT_LIMIT=400
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

cd "$REPO_ROOT"

failures=0
warnings=0

# Use find (not globstar — more portable across zsh/bash/dash).
while IFS= read -r -d '' file; do
    lines=$(wc -l < "$file" | tr -d ' ')
    rel="${file#./}"
    if [ "$lines" -gt "$HARD_LIMIT" ]; then
        printf 'FAIL  %s  %d lines  (limit %d)\n' "$rel" "$lines" "$HARD_LIMIT" >&2
        failures=$((failures + 1))
    elif [ "$lines" -gt "$SOFT_LIMIT" ]; then
        printf 'WARN  %s  %d lines  (soft limit %d)\n' "$rel" "$lines" "$SOFT_LIMIT" >&2
        warnings=$((warnings + 1))
    fi
done < <(find src -type f -name '*.rs' -print0 2>/dev/null || true)

if [ "$failures" -gt 0 ]; then
    printf '\n%d file(s) over the %d-line hard limit. Split them into modules.\n' \
        "$failures" "$HARD_LIMIT" >&2
    exit 1
fi

if [ "$warnings" -gt 0 ]; then
    printf '\n%d file(s) over the %d-line soft limit.\n' "$warnings" "$SOFT_LIMIT" >&2
fi

exit 0
