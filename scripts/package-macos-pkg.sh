#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_ROOT="$PROJECT_DIR/dist"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$PROJECT_DIR/Cargo.toml" | head -n 1)"
PACKAGE_ID="${MACOS_INSTALLER_IDENTIFIER:-app.decenterlize.foundation-share-bridge}"
SIGNING_IDENTITY="${MACOS_INSTALLER_SIGNING_IDENTITY:-}"
PKG_NAME="FoundationShareBridge-${VERSION}-macos.pkg"
SIGNED_PKG_NAME="FoundationShareBridge-${VERSION}-macos-signed.pkg"
STAGING_DIR="$DIST_ROOT/macos-pkg-staging"
ROOT_DIR="$STAGING_DIR/root"
SCRIPTS_DIR="$STAGING_DIR/scripts"
PAYLOAD_DIR="$ROOT_DIR/usr/local/lib/foundation-share-bridge/payload"
PAYLOAD_BIN_DIR="$PAYLOAD_DIR/bin"
PAYLOAD_SCRIPT_DIR="$PAYLOAD_DIR/scripts"
BIN_DIR="$ROOT_DIR/usr/local/bin"
UNSIGNED_PKG_PATH="$DIST_ROOT/$PKG_NAME"
SIGNED_PKG_PATH="$DIST_ROOT/$SIGNED_PKG_NAME"

if ! command -v pkgbuild >/dev/null 2>&1; then
  echo "pkgbuild is required to build the macOS installer package." >&2
  exit 1
fi

cd "$PROJECT_DIR"
cargo build --release

rm -rf "$STAGING_DIR"
mkdir -p "$PAYLOAD_BIN_DIR" "$PAYLOAD_SCRIPT_DIR" "$BIN_DIR" "$SCRIPTS_DIR" "$DIST_ROOT"

cp "$PROJECT_DIR/target/release/foundation-share-bridge" "$PAYLOAD_BIN_DIR/foundation-share-bridge"
cp "$PROJECT_DIR/docker-compose.yml" "$PAYLOAD_DIR/docker-compose.yml"
cp "$PROJECT_DIR/scripts/run-bridge-stack.sh" "$PAYLOAD_SCRIPT_DIR/run-bridge-stack.sh"
cp "$PROJECT_DIR/scripts/handle-deep-link.sh" "$PAYLOAD_SCRIPT_DIR/handle-deep-link.sh"
cp "$PROJECT_DIR/scripts/install.sh" "$PAYLOAD_SCRIPT_DIR/install.sh"
cp "$PROJECT_DIR/scripts/uninstall.sh" "$PAYLOAD_SCRIPT_DIR/uninstall.sh"
cp "$PROJECT_DIR/scripts/install-macos-service.sh" "$PAYLOAD_SCRIPT_DIR/install-macos-service.sh"
cp "$PROJECT_DIR/scripts/uninstall-macos-service.sh" "$PAYLOAD_SCRIPT_DIR/uninstall-macos-service.sh"

cat > "$BIN_DIR/foundation-share-bridge-install" <<'EOF'
#!/bin/bash
set -euo pipefail
exec /usr/local/lib/foundation-share-bridge/payload/scripts/install-macos-service.sh "$@"
EOF

cat > "$BIN_DIR/foundation-share-bridge-uninstall" <<'EOF'
#!/bin/bash
set -euo pipefail
exec /usr/local/lib/foundation-share-bridge/payload/scripts/uninstall-macos-service.sh "$@"
EOF

cat > "$SCRIPTS_DIR/postinstall" <<'EOF'
#!/bin/bash
set -euo pipefail

TARGET_USER="$(stat -f %Su /dev/console || true)"

if [ -n "$TARGET_USER" ] && [ "$TARGET_USER" != "root" ] && [ "$TARGET_USER" != "loginwindow" ]; then
  /usr/bin/sudo -H -u "$TARGET_USER" /usr/local/bin/foundation-share-bridge-install || true
fi
EOF

chmod +x \
  "$PAYLOAD_BIN_DIR/foundation-share-bridge" \
  "$PAYLOAD_SCRIPT_DIR/run-bridge-stack.sh" \
  "$PAYLOAD_SCRIPT_DIR/handle-deep-link.sh" \
  "$PAYLOAD_SCRIPT_DIR/install.sh" \
  "$PAYLOAD_SCRIPT_DIR/uninstall.sh" \
  "$PAYLOAD_SCRIPT_DIR/install-macos-service.sh" \
  "$PAYLOAD_SCRIPT_DIR/uninstall-macos-service.sh" \
  "$BIN_DIR/foundation-share-bridge-install" \
  "$BIN_DIR/foundation-share-bridge-uninstall" \
  "$SCRIPTS_DIR/postinstall"

rm -f "$UNSIGNED_PKG_PATH" "$SIGNED_PKG_PATH"
pkgbuild \
  --identifier "$PACKAGE_ID" \
  --version "$VERSION" \
  --root "$ROOT_DIR" \
  --scripts "$SCRIPTS_DIR" \
  "$UNSIGNED_PKG_PATH"

if [ -n "$SIGNING_IDENTITY" ]; then
  if ! command -v productsign >/dev/null 2>&1; then
    echo "productsign is required when MACOS_INSTALLER_SIGNING_IDENTITY is set." >&2
    exit 1
  fi

  productsign --sign "$SIGNING_IDENTITY" "$UNSIGNED_PKG_PATH" "$SIGNED_PKG_PATH"
  echo "Built signed macOS pkg:"
  echo "  $SIGNED_PKG_PATH"
else
  echo "Built unsigned macOS pkg:"
  echo "  $UNSIGNED_PKG_PATH"
  echo "Set MACOS_INSTALLER_SIGNING_IDENTITY to produce a signed pkg."
fi
