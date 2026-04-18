#!/bin/bash
set -euo pipefail

DEEP_LINK="${1:-}"
if [ -z "$DEEP_LINK" ]; then
  echo "Usage: handle-deep-link.sh '<foundationsharebridge://...>'" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RUNTIME_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY_PATH="$RUNTIME_DIR/bin/foundation-share-bridge"
BRIDGE_HOST="${BRIDGE_HOST:-127.0.0.1}"
BRIDGE_PORT="${BRIDGE_PORT:-43128}"
BRIDGE_URL="http://$BRIDGE_HOST:$BRIDGE_PORT"
OS_NAME="$(uname -s)"

if [ ! -x "$BINARY_PATH" ]; then
  echo "Bridge binary was not found at $BINARY_PATH" >&2
  exit 1
fi

case "$OS_NAME" in
  Darwin)
    launchctl kickstart -k "gui/$(id -u)/com.ravonus.foundation-share-bridge" >/dev/null 2>&1 || true
    OPEN_CMD="open"
    ;;
  Linux)
    systemctl --user start foundation-share-bridge.service >/dev/null 2>&1 || true
    OPEN_CMD="xdg-open"
    ;;
  *)
    echo "Unsupported platform for handle-deep-link.sh: $OS_NAME" >&2
    exit 1
    ;;
esac

if "$BINARY_PATH" handle-url "$DEEP_LINK"; then
  "$OPEN_CMD" "$BRIDGE_URL/?linked=1" >/dev/null 2>&1 || true
  exit 0
fi

"$OPEN_CMD" "$BRIDGE_URL/" >/dev/null 2>&1 || true
exit 1
