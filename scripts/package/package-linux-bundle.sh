#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DIST_ROOT="$PROJECT_DIR/dist"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/Cargo.toml" | head -n 1)"
APP_NAME="FoundationShareBridge-${VERSION}-linux"
PACKAGE_DIR="$DIST_ROOT/$APP_NAME"
PAYLOAD_DIR="$PACKAGE_DIR/payload"
PAYLOAD_BIN_DIR="$PAYLOAD_DIR/bin"
PAYLOAD_SCRIPT_DIR="$PAYLOAD_DIR/scripts"
ARCHIVE_PATH="$DIST_ROOT/$APP_NAME.tar.gz"
BINARY_PATH="${FOUNDATION_SHARE_BRIDGE_BINARY_LINUX:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --binary)
      BINARY_PATH="${2:-}"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [ -z "$BINARY_PATH" ]; then
  if [ "$(uname -s)" != "Linux" ]; then
    echo "Provide --binary /path/to/foundation-share-bridge when packaging Linux from a non-Linux host." >&2
    exit 1
  fi

  cd "$PROJECT_DIR"
  cargo build --release
  BINARY_PATH="$PROJECT_DIR/target/release/foundation-share-bridge"
fi

if [ ! -x "$BINARY_PATH" ]; then
  echo "Linux binary was not found at $BINARY_PATH" >&2
  exit 1
fi

rm -rf "$PACKAGE_DIR"
mkdir -p "$PAYLOAD_BIN_DIR" "$PAYLOAD_SCRIPT_DIR" "$DIST_ROOT"

cp "$BINARY_PATH" "$PAYLOAD_BIN_DIR/foundation-share-bridge"
cp "$PROJECT_DIR/docker-compose.yml" "$PAYLOAD_DIR/docker-compose.yml"
cp "$PROJECT_DIR/scripts/install/install.sh" "$PAYLOAD_SCRIPT_DIR/install.sh"
cp "$PROJECT_DIR/scripts/uninstall/uninstall.sh" "$PAYLOAD_SCRIPT_DIR/uninstall.sh"
cp "$PROJECT_DIR/scripts/install/install-linux-service.sh" "$PAYLOAD_SCRIPT_DIR/install-linux-service.sh"
cp "$PROJECT_DIR/scripts/uninstall/uninstall-linux-service.sh" "$PAYLOAD_SCRIPT_DIR/uninstall-linux-service.sh"
cp "$PROJECT_DIR/scripts/runtime/run-bridge-stack-linux.sh" "$PAYLOAD_SCRIPT_DIR/run-bridge-stack-linux.sh"
cp "$PROJECT_DIR/scripts/runtime/handle-deep-link.sh" "$PAYLOAD_SCRIPT_DIR/handle-deep-link.sh"
cp "$PROJECT_DIR/LICENSE" "$PACKAGE_DIR/LICENSE"

cat > "$PACKAGE_DIR/install-foundation-share-bridge.sh" <<'EOF'
#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$SCRIPT_DIR/payload/scripts/install.sh" "$@"
EOF

cat > "$PACKAGE_DIR/uninstall-foundation-share-bridge.sh" <<'EOF'
#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$SCRIPT_DIR/payload/scripts/uninstall.sh" "$@"
EOF

cat > "$PACKAGE_DIR/README.txt" <<'EOF'
Foundation Share Bridge for Linux
=================================

1. Install Docker or Podman if the machine does not already have one.
2. Run ./install-foundation-share-bridge.sh
3. The installer registers a systemd user service that starts after login.
4. Click the desktop app link from the Foundation archive site to pair it.

If you want the service to keep running after logout, enable lingering:
  loginctl enable-linger $USER
EOF

chmod +x \
  "$PAYLOAD_BIN_DIR/foundation-share-bridge" \
  "$PAYLOAD_SCRIPT_DIR/install.sh" \
  "$PAYLOAD_SCRIPT_DIR/uninstall.sh" \
  "$PAYLOAD_SCRIPT_DIR/install-linux-service.sh" \
  "$PAYLOAD_SCRIPT_DIR/uninstall-linux-service.sh" \
  "$PAYLOAD_SCRIPT_DIR/run-bridge-stack-linux.sh" \
  "$PAYLOAD_SCRIPT_DIR/handle-deep-link.sh" \
  "$PACKAGE_DIR/install-foundation-share-bridge.sh" \
  "$PACKAGE_DIR/uninstall-foundation-share-bridge.sh"

rm -f "$ARCHIVE_PATH"
tar -czf "$ARCHIVE_PATH" -C "$DIST_ROOT" "$APP_NAME"

cat <<EOF
Built Linux bundle:
  $PACKAGE_DIR

Archive:
  $ARCHIVE_PATH
EOF
