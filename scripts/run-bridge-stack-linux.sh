#!/bin/bash
set -euo pipefail

RUNTIME_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LOG_DIR="$RUNTIME_DIR/logs"
BIN_PATH="$RUNTIME_DIR/bin/foundation-share-bridge"
COMPOSE_FILE="$RUNTIME_DIR/docker-compose.yml"
export PATH="/usr/local/bin:/usr/bin:/bin:$HOME/.cargo/bin:$PATH"

mkdir -p "$LOG_DIR" "$RUNTIME_DIR/data/kubo"

choose_container_runtime() {
  if command -v docker >/dev/null 2>&1; then
    printf 'docker\n'
    return 0
  fi

  if command -v podman >/dev/null 2>&1; then
    printf 'podman\n'
    return 0
  fi

  return 1
}

wait_for_runtime() {
  local runtime="$1"
  local attempt
  for attempt in {1..60}; do
    if "$runtime" info >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  return 1
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

if ! CONTAINER_RUNTIME="$(choose_container_runtime)"; then
  echo "docker or podman is required for the bundled Kubo node." >&2
  exit 1
fi

if ! wait_for_runtime "$CONTAINER_RUNTIME"; then
  echo "$CONTAINER_RUNTIME is installed but not responding. Start it first, then restart the service." >&2
  exit 1
fi

cd "$RUNTIME_DIR"
"$CONTAINER_RUNTIME" compose -f "$COMPOSE_FILE" up -d kubo >/dev/null

if ! wait_for_kubo_api; then
  echo "The bundled Kubo API did not come online at http://127.0.0.1:5001." >&2
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
