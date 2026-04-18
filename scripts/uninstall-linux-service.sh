#!/bin/bash
set -euo pipefail

SERVICE_NAME="Foundation Share Bridge"
SERVICE_ID="foundation-share-bridge"
RUNTIME_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/foundation-share-bridge"
COMPOSE_FILE="$RUNTIME_DIR/docker-compose.yml"
SYSTEMD_USER_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
SERVICE_FILE="$SYSTEMD_USER_DIR/$SERVICE_ID.service"
APPLICATIONS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
DESKTOP_FILE="$APPLICATIONS_DIR/foundation-share-bridge.desktop"
PURGE_DATA="${1:-}"

systemctl --user disable --now "$SERVICE_ID.service" >/dev/null 2>&1 || true
rm -f "$SERVICE_FILE"
rm -f "$DESKTOP_FILE"
systemctl --user daemon-reload >/dev/null 2>&1 || true

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APPLICATIONS_DIR" >/dev/null 2>&1 || true
fi

if command -v docker >/dev/null 2>&1 && [ -f "$COMPOSE_FILE" ]; then
  docker compose -f "$COMPOSE_FILE" down >/dev/null 2>&1 || true
elif command -v podman >/dev/null 2>&1 && [ -f "$COMPOSE_FILE" ]; then
  podman compose -f "$COMPOSE_FILE" down >/dev/null 2>&1 || true
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

systemd user service:
  $SERVICE_FILE

Runtime data:
  $RUNTIME_DIR
Link handler:
  $DESKTOP_FILE

$DATA_MESSAGE
EOF
