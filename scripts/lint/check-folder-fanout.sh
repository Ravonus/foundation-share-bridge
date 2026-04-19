#!/usr/bin/env bash
# Fail if any directory under src/ or scripts/ has more than 6 direct children.
# Excludes target/, .git/, node_modules/, dist/.
# Usage: bash scripts/lint/check-folder-fanout.sh

set -euo pipefail

MAX_CHILDREN=6
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

cd "$REPO_ROOT"

failures=0

# Directories to scan. Extend ALLOWLIST via env var for transitional periods.
# Example: ALLOWLIST="scripts/vendor scripts/third_party" bash scripts/lint/check-folder-fanout.sh
ALLOWLIST="${ALLOWLIST:-}"

scan_roots=(src scripts)

is_allowed() {
    local dir="$1"
    for entry in $ALLOWLIST; do
        if [ "$dir" = "$entry" ]; then
            return 0
        fi
    done
    return 1
}

for root in "${scan_roots[@]}"; do
    if [ ! -d "$root" ]; then
        continue
    fi
    while IFS= read -r -d '' dir; do
        # Count direct children, excluding hidden files.
        count=$(find "$dir" -mindepth 1 -maxdepth 1 ! -name '.*' | wc -l | tr -d ' ')
        rel="${dir#./}"
        if [ "$count" -gt "$MAX_CHILDREN" ] && ! is_allowed "$rel"; then
            printf 'FAIL  %s  %d children  (limit %d)\n' "$rel" "$count" "$MAX_CHILDREN" >&2
            failures=$((failures + 1))
        fi
    done < <(find "$root" -type d \
        ! -path '*/target/*' ! -name 'target' \
        ! -path '*/.git/*' ! -name '.git' \
        ! -path '*/node_modules/*' ! -name 'node_modules' \
        ! -path '*/dist/*' ! -name 'dist' \
        -print0)
done

if [ "$failures" -gt 0 ]; then
    printf '\n%d folder(s) over the %d-child limit. Group into subfolders by concern.\n' \
        "$failures" "$MAX_CHILDREN" >&2
    exit 1
fi

exit 0
