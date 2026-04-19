#!/usr/bin/env bash
# Run every lint and guard check. Exit non-zero if anything fails.
# Usage: bash scripts/lint/run-all.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
cyan()  { printf '\033[36m%s\033[0m\n' "$*"; }

fails=()

run_step() {
    local label="$1"
    shift
    cyan "→ $label"
    if "$@"; then
        green "  ok"
    else
        red "  FAILED: $label"
        fails+=("$label")
    fi
}

# Rust checks.
run_step "cargo fmt --check"    cargo fmt --check
run_step "cargo clippy (strict)" cargo clippy --all-targets -- -D warnings
run_step "cargo test"            cargo test --all-targets

# cargo-deny is optional if not installed.
if command -v cargo-deny >/dev/null 2>&1; then
    run_step "cargo deny check" cargo deny check
else
    cyan "→ cargo deny: skipped (cargo-deny not installed)"
fi

# Custom guards.
run_step "scripts/lint/check-file-size.sh"      bash scripts/lint/check-file-size.sh
run_step "scripts/lint/check-folder-fanout.sh"  bash scripts/lint/check-folder-fanout.sh
run_step "scripts/lint/check-no-monolith.sh"    bash scripts/lint/check-no-monolith.sh

echo
if [ "${#fails[@]}" -gt 0 ]; then
    red "FAILED (${#fails[@]} step(s)):"
    for f in "${fails[@]}"; do
        printf '  - %s\n' "$f"
    done
    exit 1
fi

green "All lint + guard checks passed."
