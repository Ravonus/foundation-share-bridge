#!/usr/bin/env bash
# Soft warning if any single .rs file accounts for > 30% of total src/ LOC.
# Exits 0 regardless — this is a heuristic, not a gate. Flip to exit 1 in Stage 11.
# Usage: bash scripts/lint/check-no-monolith.sh

set -euo pipefail

THRESHOLD_PCT=30
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

cd "$REPO_ROOT"

if [ ! -d src ]; then
    exit 0
fi

total=0
while IFS= read -r -d '' file; do
    lines=$(wc -l < "$file" | tr -d ' ')
    total=$((total + lines))
done < <(find src -type f -name '*.rs' -print0)

if [ "$total" -eq 0 ]; then
    exit 0
fi

while IFS= read -r -d '' file; do
    lines=$(wc -l < "$file" | tr -d ' ')
    pct=$(( (lines * 100) / total ))
    if [ "$pct" -gt "$THRESHOLD_PCT" ]; then
        rel="${file#./}"
        printf 'WARN  %s  %d lines  %d%% of src/ total (soft limit %d%%)\n' \
            "$rel" "$lines" "$pct" "$THRESHOLD_PCT" >&2
    fi
done < <(find src -type f -name '*.rs' -print0)

exit 0
