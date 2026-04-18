#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PAYLOAD_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SERVICE_NAME="Foundation Share Bridge"
SERVICE_ID="foundation-share-bridge"
RUNTIME_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/foundation-share-bridge"
RUNTIME_BIN_DIR="$RUNTIME_DIR/bin"
RUNTIME_SCRIPT_DIR="$RUNTIME_DIR/scripts"
RUNTIME_LOG_DIR="$RUNTIME_DIR/logs"
RUNTIME_DATA_DIR="$RUNTIME_DIR/data/kubo"
RUNTIME_STATE_FILE="$RUNTIME_DIR/bridge-state.json"
SYSTEMD_USER_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
SERVICE_FILE="$SYSTEMD_USER_DIR/$SERVICE_ID.service"
RUN_SCRIPT="$RUNTIME_SCRIPT_DIR/run-bridge-stack-linux.sh"
DEEP_LINK_SCRIPT="$RUNTIME_SCRIPT_DIR/handle-deep-link.sh"
APPLICATIONS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
DESKTOP_FILE="$APPLICATIONS_DIR/foundation-share-bridge.desktop"

warn_missing_container_runtime() {
  if command -v docker >/dev/null 2>&1 || command -v podman >/dev/null 2>&1; then
    return 0
  fi

  cat <<EOF >&2
Warning: docker or podman was not found.
The service can still be installed, but the bundled Kubo node will not come
online until one of those container runtimes is installed.
EOF
}

resolve_source_root() {
  if [ -x "$PAYLOAD_ROOT/bin/foundation-share-bridge" ] && [ -f "$PAYLOAD_ROOT/docker-compose.yml" ]; then
    printf '%s\n' "$PAYLOAD_ROOT"
    return 0
  fi

  if [ -f "$REPO_ROOT/Cargo.toml" ]; then
    printf '%s\n' "$REPO_ROOT"
    return 0
  fi

  echo "Unable to find a payload bundle or repo checkout for $SERVICE_NAME." >&2
  exit 1
}

copy_seed_data() {
  local source_root="$1"
  local seed_dir="$source_root/seed"

  if [ -f "$seed_dir/bridge-state.json" ] && [ ! -f "$RUNTIME_STATE_FILE" ]; then
    cp "$seed_dir/bridge-state.json" "$RUNTIME_STATE_FILE"
  fi

  if [ -d "$seed_dir/data/kubo" ] && [ ! -f "$RUNTIME_DATA_DIR/config" ]; then
    cp -R "$seed_dir/data/kubo/." "$RUNTIME_DATA_DIR/"
  fi
}

build_or_resolve_binary() {
  local source_root="$1"
  local bundled_binary="$source_root/bin/foundation-share-bridge"

  if [ -x "$bundled_binary" ]; then
    printf '%s\n' "$bundled_binary"
    return 0
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required when installing from source." >&2
    exit 1
  fi

  (
    cd "$source_root"
    cargo build --release
  )

  printf '%s\n' "$source_root/target/release/foundation-share-bridge"
}

SOURCE_ROOT="$(resolve_source_root)"
BINARY_SOURCE="$(build_or_resolve_binary "$SOURCE_ROOT")"
COMPOSE_SOURCE="$SOURCE_ROOT/docker-compose.yml"
RUN_SCRIPT_SOURCE="$SOURCE_ROOT/scripts/run-bridge-stack-linux.sh"
DEEP_LINK_SCRIPT_SOURCE="$SOURCE_ROOT/scripts/handle-deep-link.sh"

if [ ! -x "$BINARY_SOURCE" ]; then
  echo "Bridge binary was not found at $BINARY_SOURCE" >&2
  exit 1
fi

if [ ! -f "$COMPOSE_SOURCE" ] || [ ! -f "$RUN_SCRIPT_SOURCE" ] || [ ! -f "$DEEP_LINK_SCRIPT_SOURCE" ]; then
  echo "Installer assets were not found under $SOURCE_ROOT" >&2
  exit 1
fi

mkdir -p \
  "$RUNTIME_BIN_DIR" \
  "$RUNTIME_SCRIPT_DIR" \
  "$RUNTIME_LOG_DIR" \
  "$RUNTIME_DATA_DIR" \
  "$SYSTEMD_USER_DIR" \
  "$APPLICATIONS_DIR"

cp "$BINARY_SOURCE" "$RUNTIME_BIN_DIR/foundation-share-bridge"
cp "$RUN_SCRIPT_SOURCE" "$RUN_SCRIPT"
cp "$DEEP_LINK_SCRIPT_SOURCE" "$DEEP_LINK_SCRIPT"
cp "$COMPOSE_SOURCE" "$RUNTIME_DIR/docker-compose.yml"
chmod +x "$RUNTIME_BIN_DIR/foundation-share-bridge" "$RUN_SCRIPT" "$DEEP_LINK_SCRIPT"

copy_seed_data "$SOURCE_ROOT"
warn_missing_container_runtime

cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=Foundation Share Bridge
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=$RUN_SCRIPT
WorkingDirectory=$RUNTIME_DIR
Restart=always
RestartSec=15
Environment=PATH=/usr/local/bin:/usr/bin:/bin:$HOME/.cargo/bin

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable --now "$SERVICE_ID.service"

cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Name=Foundation Share Bridge Link
Comment=Open Foundation Share Bridge pairing links
Exec=$DEEP_LINK_SCRIPT %u
Type=Application
NoDisplay=true
Terminal=false
MimeType=x-scheme-handler/foundationsharebridge;
Categories=Network;
EOF

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APPLICATIONS_DIR" >/dev/null 2>&1 || true
fi

if command -v xdg-mime >/dev/null 2>&1; then
  xdg-mime default foundation-share-bridge.desktop x-scheme-handler/foundationsharebridge >/dev/null 2>&1 || true
fi

cat <<EOF
Installed and started $SERVICE_NAME

systemd user service:
  $SERVICE_FILE
Runtime:
  $RUNTIME_DIR
Logs:
  $RUNTIME_LOG_DIR
Link handler:
  $DESKTOP_FILE

This installs a user service. It starts when you log in. If you want it to keep
running even after logout, enable lingering for your account:
  loginctl enable-linger $USER
EOF
