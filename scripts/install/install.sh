#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
OS_NAME="$(uname -s)"

case "$OS_NAME" in
  Darwin)
    exec "$SCRIPT_DIR/install-macos-service.sh" "$@"
    ;;
  Linux)
    exec "$SCRIPT_DIR/install-linux-service.sh" "$@"
    ;;
  *)
    cat >&2 <<EOF
Unsupported platform for scripts/install/install.sh: $OS_NAME

On Windows, run:
  powershell -ExecutionPolicy Bypass -File .\\scripts\\install\\install.ps1
EOF
    exit 1
    ;;
esac
