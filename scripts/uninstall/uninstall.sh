#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
OS_NAME="$(uname -s)"

case "$OS_NAME" in
  Darwin)
    exec "$SCRIPT_DIR/uninstall-macos-service.sh" "$@"
    ;;
  Linux)
    exec "$SCRIPT_DIR/uninstall-linux-service.sh" "$@"
    ;;
  *)
    cat >&2 <<EOF
Unsupported platform for scripts/uninstall/uninstall.sh: $OS_NAME

On Windows, run:
  powershell -ExecutionPolicy Bypass -File .\\scripts\\uninstall\\uninstall.ps1
EOF
    exit 1
    ;;
esac
