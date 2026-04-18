#!/bin/bash
set -euo pipefail

RUNTIME_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LOG_DIR="$RUNTIME_DIR/logs"
BIN_PATH="$RUNTIME_DIR/bin/foundation-share-bridge"
COMPOSE_FILE="$RUNTIME_DIR/docker-compose.yml"
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$HOME/.cargo/bin:$PATH"

mkdir -p "$LOG_DIR" "$RUNTIME_DIR/data/kubo"

wait_for_docker() {
  local attempt
  for attempt in {1..60}; do
    if docker info >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  return 1
}

ensure_container_runtime() {
  if docker info >/dev/null 2>&1; then
    return 0
  fi

  if command -v colima >/dev/null 2>&1; then
    colima start >/dev/null 2>&1 || true
    if wait_for_docker; then
      return 0
    fi
  fi

  if [ -d "/Applications/Docker.app" ]; then
    open -a Docker >/dev/null 2>&1 || true
  elif [ -d "$HOME/Applications/Docker.app" ]; then
    open -a "$HOME/Applications/Docker.app" >/dev/null 2>&1 || true
  fi

  wait_for_docker
}

wait_for_kubo_api() {
  local attempt
  for attempt in {1..60}; do
    if curl -fsS -X POST http://127.0.0.1:5001/api/v0/version >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  return 1
}

if ! ensure_container_runtime; then
  echo "Docker Desktop or Colima is required for the bundled Kubo node. Install or start one of them, then relaunch the background item." >&2
  exit 1
fi

cd "$RUNTIME_DIR"
docker compose -f "$COMPOSE_FILE" up -d kubo >/dev/null

if ! wait_for_kubo_api; then
  echo "The bundled Kubo API did not come online at http://127.0.0.1:5001. Check Docker and the logs under $LOG_DIR." >&2
  exit 1
fi

if [ ! -x "$BIN_PATH" ]; then
  echo "Bridge binary was not installed at $BIN_PATH" >&2
  exit 1
fi

export IPFS_API_URL="http://127.0.0.1:5001"
export BRIDGE_STATE_FILE="$RUNTIME_DIR/bridge-state.json"
export SELF_REPAIR_INTERVAL_SECONDS="${SELF_REPAIR_INTERVAL_SECONDS:-900}"

exec "$BIN_PATH"
