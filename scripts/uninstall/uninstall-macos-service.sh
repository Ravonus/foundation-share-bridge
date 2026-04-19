#!/bin/bash
set -euo pipefail

AGENT_LABEL="com.ravonus.foundation-share-bridge"
MENU_AGENT_LABEL="com.ravonus.foundation-share-bridge.menu"
SERVICE_NAME="Foundation Share Bridge"
PLIST_PATH="$HOME/Library/LaunchAgents/$AGENT_LABEL.plist"
MENU_PLIST_PATH="$HOME/Library/LaunchAgents/$MENU_AGENT_LABEL.plist"
RUNTIME_DIR="$HOME/Library/Application Support/FoundationShareBridge"
COMPOSE_FILE="$RUNTIME_DIR/docker-compose.yml"
PROTOCOL_APP_DIR="$HOME/Applications/Foundation Share Bridge Link.app"
PURGE_DATA="${1:-}"

launchctl bootout "gui/$(id -u)" "$PLIST_PATH" >/dev/null 2>&1 || true
launchctl bootout "gui/$(id -u)" "$MENU_PLIST_PATH" >/dev/null 2>&1 || true
rm -f "$PLIST_PATH"
rm -f "$MENU_PLIST_PATH"
rm -rf "$PROTOCOL_APP_DIR"

if command -v docker >/dev/null 2>&1 && [ -f "$COMPOSE_FILE" ]; then
  docker compose -f "$COMPOSE_FILE" down >/dev/null 2>&1 || true
fi

if [ "$PURGE_DATA" = "--purge-data" ]; then
  rm -rf "$RUNTIME_DIR"
fi

if [ "$PURGE_DATA" = "--purge-data" ]; then
  DATA_MESSAGE="Runtime data deleted."
else
  DATA_MESSAGE="Pass --purge-data if you also want to delete the watched-pin state and bundled Kubo repo from the runtime directory."
fi

cat <<EOF
Removed $SERVICE_NAME

Bridge LaunchAgent:
  $PLIST_PATH
Menu bar LaunchAgent:
  $MENU_PLIST_PATH

Runtime data:
  $RUNTIME_DIR
App link handler:
  $PROTOCOL_APP_DIR

$DATA_MESSAGE
EOF
