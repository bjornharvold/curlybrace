#!/usr/bin/env bash
# Package the release binary as a macOS .app bundle and zip it.
#
#   scripts/package-macos.sh <version> <target-triple> [binary-path]
#
# Example: scripts/package-macos.sh 0.1.0 aarch64-apple-darwin
# Output (in ./dist): the-mongodb-gui-v<version>-macos-arm64.zip and a .sha256 file.
#
# The bundle is ad-hoc signed (a valid signature is required for arm64 binaries to run),
# but NOT notarized: macOS Gatekeeper asks for confirmation on first launch.
set -euo pipefail

VERSION="${1:?usage: package-macos.sh <version> <target-triple> [binary-path]}"
TARGET="${2:?usage: package-macos.sh <version> <target-triple> [binary-path]}"
BINARY="${3:-target/${TARGET}/release/mongostudio}"

case "$TARGET" in
  aarch64-apple-darwin) ARCH_LABEL="arm64" ;;
  x86_64-apple-darwin) ARCH_LABEL="x86_64" ;;
  *) echo "unsupported target: $TARGET" >&2; exit 1 ;;
esac

[[ -x "$BINARY" ]] || { echo "binary not found or not executable: $BINARY" >&2; exit 1; }

APP_NAME="The MongoDB GUI"
BUNDLE_ID="io.github.bjornharvold.the-mongodb-gui"
DIST="dist"
APP="${DIST}/${APP_NAME}.app"
ASSET="the-mongodb-gui-v${VERSION}-macos-${ARCH_LABEL}"

rm -rf "$DIST"
mkdir -p "${APP}/Contents/MacOS" "${APP}/Contents/Resources"
cp "$BINARY" "${APP}/Contents/MacOS/mongostudio"
chmod 755 "${APP}/Contents/MacOS/mongostudio"
cp LICENSE-MIT LICENSE-APACHE "${APP}/Contents/Resources/"

cat > "${APP}/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleExecutable</key><string>mongostudio</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

# Ad-hoc signature: required for arm64 code to execute; not a Developer ID signature.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

( cd "$DIST" && ditto -c -k --norsrc --noextattr --noqtn --keepParent "${APP_NAME}.app" "${ASSET}.zip" \
  && shasum -a 256 "${ASSET}.zip" > "${ASSET}.zip.sha256" )

echo "wrote ${DIST}/${ASSET}.zip"
cat "${DIST}/${ASSET}.zip.sha256"
