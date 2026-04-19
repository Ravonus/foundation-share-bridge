#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DIST_ROOT="$PROJECT_DIR/dist"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/Cargo.toml" | head -n 1)"
APP_NAME="FoundationShareBridge-${VERSION}-macos-bundle"
PACKAGE_DIR="$DIST_ROOT/$APP_NAME"
PAYLOAD_DIR="$PACKAGE_DIR/payload"
PAYLOAD_BIN_DIR="$PAYLOAD_DIR/bin"
PAYLOAD_SCRIPT_DIR="$PAYLOAD_DIR/scripts"
PAYLOAD_ASSET_DIR="$PAYLOAD_DIR/assets"
ARCHIVE_PATH="$DIST_ROOT/$APP_NAME.tar.gz"
MENU_SOURCE="$PROJECT_DIR/scripts/menu/foundation-share-bridge-menu.swift"
MENU_BINARY="$PROJECT_DIR/target/foundation-share-bridge-menu"

cd "$PROJECT_DIR"
cargo build --release
/usr/bin/xcrun swiftc -O -framework AppKit "$MENU_SOURCE" -o "$MENU_BINARY"

rm -rf "$PACKAGE_DIR"
mkdir -p "$PAYLOAD_BIN_DIR" "$PAYLOAD_SCRIPT_DIR" "$PAYLOAD_ASSET_DIR"

cp "$PROJECT_DIR/target/release/foundation-share-bridge" "$PAYLOAD_BIN_DIR/foundation-share-bridge"
cp "$MENU_BINARY" "$PAYLOAD_BIN_DIR/foundation-share-bridge-menu"
cp "$PROJECT_DIR/docker-compose.yml" "$PAYLOAD_DIR/docker-compose.yml"
cp "$PROJECT_DIR/scripts/runtime/run-bridge-stack.sh" "$PAYLOAD_SCRIPT_DIR/run-bridge-stack.sh"
cp "$PROJECT_DIR/scripts/runtime/handle-deep-link.sh" "$PAYLOAD_SCRIPT_DIR/handle-deep-link.sh"
cp "$PROJECT_DIR/scripts/menu/foundation-share-bridge-menu.swift" "$PAYLOAD_SCRIPT_DIR/foundation-share-bridge-menu.swift"
cp "$PROJECT_DIR/scripts/install/install-macos-service.sh" "$PAYLOAD_SCRIPT_DIR/install-macos-service.sh"
cp "$PROJECT_DIR/scripts/uninstall/uninstall-macos-service.sh" "$PAYLOAD_SCRIPT_DIR/uninstall-macos-service.sh"
cp "$PROJECT_DIR/scripts/install/install.sh" "$PAYLOAD_SCRIPT_DIR/install.sh"
cp "$PROJECT_DIR/scripts/uninstall/uninstall.sh" "$PAYLOAD_SCRIPT_DIR/uninstall.sh"
cp "$PROJECT_DIR/assets/logo-light.png" "$PAYLOAD_ASSET_DIR/logo-light.png"
cp "$PROJECT_DIR/assets/logo-dark.png" "$PAYLOAD_ASSET_DIR/logo-dark.png"
cp "$PROJECT_DIR/LICENSE" "$PACKAGE_DIR/LICENSE"

cat > "$PACKAGE_DIR/Install Foundation Share Bridge.command" <<'EOF'
#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$SCRIPT_DIR/payload/scripts/install-macos-service.sh" "$@"
EOF

cat > "$PACKAGE_DIR/Uninstall Foundation Share Bridge.command" <<'EOF'
#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$SCRIPT_DIR/payload/scripts/uninstall-macos-service.sh" "$@"
EOF

cat > "$PACKAGE_DIR/README.txt" <<'EOF'
Foundation Share Bridge for macOS
================================

1. Install Docker Desktop or Colima if the machine does not already have one.
2. Double-click "Install Foundation Share Bridge.command".
3. macOS will register the bridge as a per-user background item.
4. Click the desktop app link from the Foundation archive site to pair it.

This helper keeps a local Kubo node plus the Rust bridge running in the
background after login so artists can pin rescued Foundation works without using
CLI tools.

If you ever want to remove it, double-click
"Uninstall Foundation Share Bridge.command".
EOF

chmod +x \
  "$PAYLOAD_BIN_DIR/foundation-share-bridge" \
  "$PAYLOAD_BIN_DIR/foundation-share-bridge-menu" \
  "$PAYLOAD_SCRIPT_DIR/run-bridge-stack.sh" \
  "$PAYLOAD_SCRIPT_DIR/handle-deep-link.sh" \
  "$PAYLOAD_SCRIPT_DIR/foundation-share-bridge-menu.swift" \
  "$PAYLOAD_SCRIPT_DIR/install-macos-service.sh" \
  "$PAYLOAD_SCRIPT_DIR/uninstall-macos-service.sh" \
  "$PAYLOAD_SCRIPT_DIR/install.sh" \
  "$PAYLOAD_SCRIPT_DIR/uninstall.sh" \
  "$PACKAGE_DIR/Install Foundation Share Bridge.command" \
  "$PACKAGE_DIR/Uninstall Foundation Share Bridge.command"

mkdir -p "$DIST_ROOT"
rm -f "$ARCHIVE_PATH"
tar -czf "$ARCHIVE_PATH" -C "$DIST_ROOT" "$APP_NAME"

cat <<EOF
Built macOS installer bundle:
  $PACKAGE_DIR

Archive:
  $ARCHIVE_PATH

Ship the extracted folder or the tar.gz. End users can double-click the install
command and macOS will register the bridge as a background item for that user.
EOF
