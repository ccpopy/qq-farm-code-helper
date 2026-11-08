#!/usr/bin/env bash
# Packages an already-built .app into an .ipa (Payload/Name.app).
# Usage: package-ipa.sh <App.app> <output.ipa>
set -euo pipefail

APP="${1:?App path is required}"
OUT="${2:?Output IPA path is required}"

[ -d "$APP" ] || { echo "App bundle not found: $APP" >&2; exit 1; }
[ -f "$APP/Info.plist" ] || { echo "App Info.plist not found: $APP" >&2; exit 1; }

OUT_DIR_RAW="$(dirname "$OUT")"
mkdir -p "$OUT_DIR_RAW"
OUT_DIR="$(cd "$OUT_DIR_RAW" && pwd)"
OUT_ABS="$OUT_DIR/$(basename "$OUT")"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/Payload"
/usr/bin/ditto "$APP" "$STAGE/Payload/$(basename "$APP")"
rm -f "$OUT_ABS"
(
  cd "$STAGE"
  /usr/bin/ditto -c -k --sequesterRsrc --keepParent Payload "$OUT_ABS"
)

[ -f "$OUT_ABS" ] || { echo "IPA was not created: $OUT_ABS" >&2; exit 1; }
echo "Packaged $OUT_ABS"
