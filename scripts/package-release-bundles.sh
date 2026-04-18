#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
OS_NAME="$(uname -s)"

case "$OS_NAME" in
  Darwin)
    "$SCRIPT_DIR/package-macos-installer.sh"
    "$SCRIPT_DIR/package-macos-pkg.sh"
    ;;
  Linux)
    "$SCRIPT_DIR/package-linux-bundle.sh"
    ;;
  CYGWIN*|MINGW*|MSYS*)
    "$SCRIPT_DIR/package-windows-bundle.sh"
    ;;
  *)
    echo "Unsupported host platform for automatic native packaging: $OS_NAME" >&2
    ;;
esac

if [ -n "${FOUNDATION_SHARE_BRIDGE_BINARY_LINUX:-}" ]; then
  "$SCRIPT_DIR/package-linux-bundle.sh" --binary "$FOUNDATION_SHARE_BRIDGE_BINARY_LINUX"
fi

if [ -n "${FOUNDATION_SHARE_BRIDGE_BINARY_WINDOWS:-}" ]; then
  "$SCRIPT_DIR/package-windows-bundle.sh" --binary "$FOUNDATION_SHARE_BRIDGE_BINARY_WINDOWS"
fi
