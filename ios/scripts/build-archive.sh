#!/usr/bin/env bash
# Builds an unsigned Release archive for the iOS app. Signing is done separately by
# sign-ipa.sh so CI can produce both unsigned and signed artifacts.
#
# Usage: build-archive.sh <archive-path> <version> <build-number>
set -euo pipefail

ARCHIVE="${1:?Archive path is required}"
VERSION="${2:?Version is required}"
BUILD_NUMBER="${3:?Build number is required}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if ! [[ "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "Invalid version: $VERSION (expected X.Y.Z)" >&2
  exit 1
fi
case "$BUILD_NUMBER" in
  ''|*[!0-9]*) echo "Invalid build number: $BUILD_NUMBER" >&2; exit 1 ;;
esac

BACKUP="$(mktemp -d)"
trap 'cp "$BACKUP/app.plist" "$ROOT/FarmHelper/Info.plist"; cp "$BACKUP/tunnel.plist" "$ROOT/FarmHelperTunnel/Info.plist"; rm -rf "$BACKUP"' EXIT
cp "$ROOT/FarmHelper/Info.plist" "$BACKUP/app.plist"
cp "$ROOT/FarmHelperTunnel/Info.plist" "$BACKUP/tunnel.plist"

for plist in "$ROOT/FarmHelper/Info.plist" "$ROOT/FarmHelperTunnel/Info.plist"; do
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" "$plist"
done

mkdir -p "$(dirname "$ARCHIVE")"
cd "$ROOT"
/usr/bin/xcodebuild \
  -project FarmHelper.xcodeproj \
  -scheme FarmHelper \
  -configuration Release \
  -sdk iphoneos \
  -destination 'generic/platform=iOS' \
  -archivePath "$ARCHIVE" \
  CODE_SIGNING_ALLOWED=NO \
  CODE_SIGNING_REQUIRED=NO \
  CODE_SIGN_IDENTITY="" \
  DEVELOPMENT_TEAM="" \
  ONLY_ACTIVE_ARCH=NO \
  COMPILER_INDEX_STORE_ENABLE=NO \
  archive

APP="$ARCHIVE/Products/Applications/FarmHelper.app"
[ -d "$APP" ] || { echo "Archived app not found: $APP" >&2; exit 1; }
echo "Built $ARCHIVE"
