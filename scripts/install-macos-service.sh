#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PAYLOAD_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGENT_LABEL="com.ravonus.foundation-share-bridge"
MENU_AGENT_LABEL="com.ravonus.foundation-share-bridge.menu"
SERVICE_NAME="Foundation Share Bridge"
RUNTIME_DIR="$HOME/Library/Application Support/FoundationShareBridge"
RUNTIME_BIN_DIR="$RUNTIME_DIR/bin"
RUNTIME_SCRIPT_DIR="$RUNTIME_DIR/scripts"
RUNTIME_ASSET_DIR="$RUNTIME_DIR/assets"
RUNTIME_LOG_DIR="$RUNTIME_DIR/logs"
RUNTIME_DATA_DIR="$RUNTIME_DIR/data/kubo"
RUNTIME_STATE_FILE="$RUNTIME_DIR/bridge-state.json"
RUNTIME_CONFIG_FILE="$RUNTIME_DIR/bridge-config.yaml"
RUNTIME_LOGO_LIGHT_FILE="$RUNTIME_ASSET_DIR/logo-light.png"
RUNTIME_LOGO_DARK_FILE="$RUNTIME_ASSET_DIR/logo-dark.png"
PLIST_PATH="$HOME/Library/LaunchAgents/$AGENT_LABEL.plist"
MENU_PLIST_PATH="$HOME/Library/LaunchAgents/$MENU_AGENT_LABEL.plist"
RUN_SCRIPT="$RUNTIME_SCRIPT_DIR/run-bridge-stack.sh"
DEEP_LINK_SCRIPT="$RUNTIME_SCRIPT_DIR/handle-deep-link.sh"
PROTOCOL_APP_DIR="$HOME/Applications/Foundation Share Bridge Link.app"
PROTOCOL_APP_CONTENTS_DIR="$PROTOCOL_APP_DIR/Contents"
PROTOCOL_APP_MACOS_DIR="$PROTOCOL_APP_CONTENTS_DIR/MacOS"
PROTOCOL_APP_INFO_PLIST="$PROTOCOL_APP_CONTENTS_DIR/Info.plist"

warn_missing_container_runtime() {
  if command -v docker >/dev/null 2>&1; then
    return 0
  fi

  if command -v colima >/dev/null 2>&1; then
    return 0
  fi

  if [ -d "/Applications/Docker.app" ] || [ -d "$HOME/Applications/Docker.app" ]; then
    return 0
  fi

  cat <<EOF >&2
Warning: Docker Desktop or Colima was not found.
The background item can still be installed, but the bundled Kubo node will not
come online until the user installs one of those container runtimes.
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

build_or_resolve_menu_binary() {
  local source_root="$1"
  local bundled_binary="$source_root/bin/foundation-share-bridge-menu"
  local menu_source="$source_root/scripts/foundation-share-bridge-menu.swift"
  local built_binary="$source_root/target/foundation-share-bridge-menu"

  if [ -x "$bundled_binary" ]; then
    printf '%s\n' "$bundled_binary"
    return 0
  fi

  if [ ! -f "$menu_source" ]; then
    echo "Menu bar source was not found at $menu_source" >&2
    exit 1
  fi

  if ! command -v xcrun >/dev/null 2>&1; then
    echo "xcrun is required to build the macOS menu bar app from source." >&2
    exit 1
  fi

  mkdir -p "$source_root/target"
  /usr/bin/xcrun swiftc -O -framework AppKit "$menu_source" -o "$built_binary"

  printf '%s\n' "$built_binary"
}

SOURCE_ROOT="$(resolve_source_root)"
BINARY_SOURCE="$(build_or_resolve_binary "$SOURCE_ROOT")"
MENU_BINARY_SOURCE="$(build_or_resolve_menu_binary "$SOURCE_ROOT")"
COMPOSE_SOURCE="$SOURCE_ROOT/docker-compose.yml"
RUN_SCRIPT_SOURCE="$SOURCE_ROOT/scripts/run-bridge-stack.sh"
DEEP_LINK_SCRIPT_SOURCE="$SOURCE_ROOT/scripts/handle-deep-link.sh"
LOGO_LIGHT_SOURCE="$SOURCE_ROOT/assets/logo-light.png"
LOGO_DARK_SOURCE="$SOURCE_ROOT/assets/logo-dark.png"

if [ ! -x "$BINARY_SOURCE" ] || [ ! -x "$MENU_BINARY_SOURCE" ]; then
  echo "Bridge binary was not found at $BINARY_SOURCE" >&2
  exit 1
fi

if [ ! -f "$COMPOSE_SOURCE" ] || [ ! -f "$RUN_SCRIPT_SOURCE" ] || [ ! -f "$DEEP_LINK_SCRIPT_SOURCE" ]; then
  echo "Installer assets were not found under $SOURCE_ROOT" >&2
  exit 1
fi

mkdir -p \
  "$HOME/Applications" \
  "$HOME/Library/LaunchAgents" \
  "$RUNTIME_BIN_DIR" \
  "$RUNTIME_SCRIPT_DIR" \
  "$RUNTIME_ASSET_DIR" \
  "$RUNTIME_LOG_DIR" \
  "$RUNTIME_DATA_DIR" \
  "$PROTOCOL_APP_MACOS_DIR"

cp "$BINARY_SOURCE" "$RUNTIME_BIN_DIR/foundation-share-bridge"
cp "$MENU_BINARY_SOURCE" "$RUNTIME_BIN_DIR/foundation-share-bridge-menu"
cp "$RUN_SCRIPT_SOURCE" "$RUN_SCRIPT"
cp "$DEEP_LINK_SCRIPT_SOURCE" "$DEEP_LINK_SCRIPT"
cp "$COMPOSE_SOURCE" "$RUNTIME_DIR/docker-compose.yml"
if [ -f "$LOGO_LIGHT_SOURCE" ]; then
  cp "$LOGO_LIGHT_SOURCE" "$RUNTIME_LOGO_LIGHT_FILE"
fi
if [ -f "$LOGO_DARK_SOURCE" ]; then
  cp "$LOGO_DARK_SOURCE" "$RUNTIME_LOGO_DARK_FILE"
fi
chmod +x \
  "$RUNTIME_BIN_DIR/foundation-share-bridge" \
  "$RUNTIME_BIN_DIR/foundation-share-bridge-menu" \
  "$RUN_SCRIPT" \
  "$DEEP_LINK_SCRIPT"

cat > "$PROTOCOL_APP_MACOS_DIR/foundation-share-bridge-link" <<EOF
#!/bin/bash
set -euo pipefail
exec "$DEEP_LINK_SCRIPT" "\$@"
EOF

cat > "$PROTOCOL_APP_INFO_PLIST" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>foundation-share-bridge-link</string>
  <key>CFBundleIdentifier</key>
  <string>com.ravonus.foundation-share-bridge.link</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Foundation Share Bridge Link</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>
      <string>Foundation Share Bridge Pairing</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>foundationsharebridge</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
EOF

chmod +x "$PROTOCOL_APP_MACOS_DIR/foundation-share-bridge-link"

copy_seed_data "$SOURCE_ROOT"
warn_missing_container_runtime

cat > "$PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$AGENT_LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$RUN_SCRIPT</string>
  </array>
  <key>WorkingDirectory</key>
  <string>$RUNTIME_DIR</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>15</integer>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>$RUNTIME_LOG_DIR/launchd.out.log</string>
  <key>StandardErrorPath</key>
  <string>$RUNTIME_LOG_DIR/launchd.err.log</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$HOME/.cargo/bin</string>
  </dict>
</dict>
</plist>
EOF

cat > "$MENU_PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$MENU_AGENT_LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$RUNTIME_BIN_DIR/foundation-share-bridge-menu</string>
  </array>
  <key>WorkingDirectory</key>
  <string>$RUNTIME_DIR</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>15</integer>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>StandardOutPath</key>
  <string>$RUNTIME_LOG_DIR/menu.out.log</string>
  <key>StandardErrorPath</key>
  <string>$RUNTIME_LOG_DIR/menu.err.log</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>FOUNDATION_SHARE_BRIDGE_RUNTIME_DIR</key>
    <string>$RUNTIME_DIR</string>
    <key>FOUNDATION_SHARE_BRIDGE_CONFIG_FILE</key>
    <string>$RUNTIME_CONFIG_FILE</string>
    <key>FOUNDATION_SHARE_BRIDGE_LOCAL_URL</key>
    <string>http://127.0.0.1:43128</string>
    <key>FOUNDATION_SHARE_BRIDGE_SITE_URL</key>
    <string>https://foundation.agorix.io</string>
    <key>FOUNDATION_SHARE_BRIDGE_LOGO_LIGHT_FILE</key>
    <string>$RUNTIME_LOGO_LIGHT_FILE</string>
    <key>FOUNDATION_SHARE_BRIDGE_LOGO_DARK_FILE</key>
    <string>$RUNTIME_LOGO_DARK_FILE</string>
    <key>PATH</key>
    <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$HOME/.cargo/bin</string>
  </dict>
</dict>
</plist>
EOF

launchctl bootout "gui/$(id -u)" "$PLIST_PATH" >/dev/null 2>&1 || true
launchctl bootout "gui/$(id -u)" "$MENU_PLIST_PATH" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"
launchctl bootstrap "gui/$(id -u)" "$MENU_PLIST_PATH"
launchctl kickstart -k "gui/$(id -u)/$AGENT_LABEL"
launchctl kickstart -k "gui/$(id -u)/$MENU_AGENT_LABEL"

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [ -x "$LSREGISTER" ]; then
  "$LSREGISTER" -f "$PROTOCOL_APP_DIR" >/dev/null 2>&1 || true
fi
touch "$PROTOCOL_APP_DIR"

cat <<EOF
Installed and started $SERVICE_NAME

Bridge LaunchAgent:
  $PLIST_PATH
Menu bar LaunchAgent:
  $MENU_PLIST_PATH
Runtime:
  $RUNTIME_DIR
Logs:
  $RUNTIME_LOG_DIR
App link handler:
  $PROTOCOL_APP_DIR

This installs a per-user background item. It will start after login and keep the
Rust bridge plus bundled Kubo node alive in the background. It also starts a
menu bar app for visibility and quick settings, and registers
foundationsharebridge:// links so the archive site can open the installed app.
EOF
