#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DIST_ROOT="$PROJECT_DIR/dist"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/Cargo.toml" | head -n 1)"
APP_NAME="FoundationShareBridge-${VERSION}-windows"
PACKAGE_DIR="$DIST_ROOT/$APP_NAME"
PAYLOAD_DIR="$PACKAGE_DIR/payload"
PAYLOAD_BIN_DIR="$PAYLOAD_DIR/bin"
PAYLOAD_SCRIPT_DIR="$PAYLOAD_DIR/scripts"
ARCHIVE_PATH="$DIST_ROOT/$APP_NAME.zip"
BINARY_PATH="${FOUNDATION_SHARE_BRIDGE_BINARY_WINDOWS:-}"

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
  case "$(uname -s)" in
    CYGWIN*|MINGW*|MSYS*)
      cd "$PROJECT_DIR"
      cargo build --release
      BINARY_PATH="$PROJECT_DIR/target/release/foundation-share-bridge.exe"
      ;;
    *)
      echo "Provide --binary /path/to/foundation-share-bridge.exe when packaging Windows from a non-Windows host." >&2
      exit 1
      ;;
  esac
fi

if [ ! -f "$BINARY_PATH" ]; then
  echo "Windows binary was not found at $BINARY_PATH" >&2
  exit 1
fi

rm -rf "$PACKAGE_DIR"
mkdir -p "$PAYLOAD_BIN_DIR" "$PAYLOAD_SCRIPT_DIR" "$DIST_ROOT"

cp "$BINARY_PATH" "$PAYLOAD_BIN_DIR/foundation-share-bridge.exe"
cp "$PROJECT_DIR/docker-compose.yml" "$PAYLOAD_DIR/docker-compose.yml"
cp "$PROJECT_DIR/scripts/install/install.ps1" "$PAYLOAD_SCRIPT_DIR/install.ps1"
cp "$PROJECT_DIR/scripts/uninstall/uninstall.ps1" "$PAYLOAD_SCRIPT_DIR/uninstall.ps1"
cp "$PROJECT_DIR/scripts/install/install-windows-service.ps1" "$PAYLOAD_SCRIPT_DIR/install-windows-service.ps1"
cp "$PROJECT_DIR/scripts/uninstall/uninstall-windows-service.ps1" "$PAYLOAD_SCRIPT_DIR/uninstall-windows-service.ps1"
cp "$PROJECT_DIR/scripts/runtime/run-bridge-stack.ps1" "$PAYLOAD_SCRIPT_DIR/run-bridge-stack.ps1"
cp "$PROJECT_DIR/scripts/runtime/handle-deep-link.ps1" "$PAYLOAD_SCRIPT_DIR/handle-deep-link.ps1"
cp "$PROJECT_DIR/LICENSE" "$PACKAGE_DIR/LICENSE"

cat > "$PACKAGE_DIR/Install Foundation Share Bridge.cmd" <<'EOF'
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0payload\scripts\install.ps1" %*
EOF

cat > "$PACKAGE_DIR/Uninstall Foundation Share Bridge.cmd" <<'EOF'
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0payload\scripts\uninstall.ps1" %*
EOF

cat > "$PACKAGE_DIR/README.txt" <<'EOF'
Foundation Share Bridge for Windows
===================================

1. Install Docker Desktop if the machine does not already have it.
2. Double-click "Install Foundation Share Bridge.cmd" or run the PowerShell installer from a terminal.
3. The installer registers a Task Scheduler item that starts at logon.
4. Click the desktop app link from the Foundation archive site to pair it.
EOF

mkdir -p "$DIST_ROOT"
rm -f "$ARCHIVE_PATH"
(
  cd "$DIST_ROOT"
  zip -rq "$ARCHIVE_PATH" "$APP_NAME"
)

cat <<EOF
Built Windows bundle:
  $PACKAGE_DIR

Archive:
  $ARCHIVE_PATH
EOF
