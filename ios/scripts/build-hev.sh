#!/usr/bin/env bash
# Builds the pinned hev-socks5-tunnel Apple xcframework used by the packet tunnel extension.
# Requires macOS, Xcode command line tools, git, make and a checkout with submodules.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="2.19.1"
CACHE="$ROOT/.build/hev-socks5-tunnel-$VERSION"
DEST="$ROOT/Vendor/HevSocks5Tunnel.xcframework"

if [ -d "$DEST" ]; then
  echo "Already present: $DEST"
  exit 0
fi
command -v xcrun >/dev/null 2>&1 || { echo "xcrun not found; install Xcode command line tools" >&2; exit 1; }

mkdir -p "$ROOT/.build" "$ROOT/Vendor"
if [ ! -d "$CACHE/.git" ]; then
  rm -rf "$CACHE"
  git clone --recursive --branch "$VERSION" https://github.com/heiher/hev-socks5-tunnel.git "$CACHE"
else
  git -C "$CACHE" submodule update --init --recursive
fi

(
  cd "$CACHE"
  ./build-apple.sh
)

rm -rf "$DEST"
cp -R "$CACHE/HevSocks5Tunnel.xcframework" "$DEST"
echo "Built $DEST"
