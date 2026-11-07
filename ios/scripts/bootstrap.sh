#!/usr/bin/env bash
# One-time bootstrap for the iOS project. Run on macOS with Xcode installed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v xcodegen >/dev/null 2>&1; then
  echo "xcodegen is required: brew install xcodegen" >&2
  exit 1
fi
command -v xcodebuild >/dev/null 2>&1 || { echo "Xcode is required" >&2; exit 1; }

"$ROOT/scripts/build-hev.sh"
xcodegen generate

cat <<'EOF'

Generated FarmHelper.xcodeproj.

Next:
  1. Open FarmHelper.xcodeproj in Xcode.
  2. Select the FarmHelper target > Signing & Capabilities and choose your team.
  3. Repeat for FarmHelperTunnel (same team). The App Groups / Network Extensions /
     Keychain Sharing capabilities are already declared in Config/*.entitlements.
  4. Build and run on a real device. A paid Apple Developer account is required for
     the Network Extension entitlement.
EOF
