#!/usr/bin/env bash
# Signs an archived iOS app and its Packet Tunnel extension with a distribution
# certificate and two provisioning profiles, then packages a signed IPA.
#
# Usage: sign-ipa.sh <App.app> <output.ipa> <app.mobileprovision> <tunnel.mobileprovision>
# Requires: SIGN_IDENTITY set to a codesigning identity in the active keychain.
# The entitlements embedded in each profile are used verbatim, so the profiles must
# already contain App Groups, Keychain Sharing and (for the extension) Network Extension.
set -euo pipefail

APP="${1:?App path is required}"
OUT="${2:?Output IPA path is required}"
APP_PROFILE="${3:?App provisioning profile is required}"
TUNNEL_PROFILE="${4:?Tunnel provisioning profile is required}"
IDENTITY="${SIGN_IDENTITY:-}"

[ -d "$APP" ] || { echo "App bundle not found: $APP" >&2; exit 1; }
APPEX="$APP/PlugIns/FarmHelperTunnel.appex"
[ -d "$APPEX" ] || { echo "Packet Tunnel extension not found: $APPEX" >&2; exit 1; }
[ -f "$APP_PROFILE" ] || { echo "App profile not found: $APP_PROFILE" >&2; exit 1; }
[ -f "$TUNNEL_PROFILE" ] || { echo "Tunnel profile not found: $TUNNEL_PROFILE" >&2; exit 1; }
[ -n "$IDENTITY" ] || { echo "SIGN_IDENTITY is empty" >&2; exit 1; }

APP_BUNDLE=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Info.plist")
APPEX_BUNDLE=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APPEX/Info.plist")

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

extract_entitlements() {
  local profile="$1"
  local output="$2"
  local bundle_id="$3"
  local label="$4"

  /usr/bin/security cms -D -i "$profile" > "$TMP/$label-profile.plist"
  rm -f "$output"
  /usr/bin/plutil -extract Entitlements xml1 -o "$output" "$TMP/$label-profile.plist" >/dev/null

  local app_id
  app_id=$(/usr/libexec/PlistBuddy -c 'Print :application-identifier' "$output" 2>/dev/null || true)
  case "$app_id" in
    *".$bundle_id") ;;
    *)
      echo "$label profile application-identifier '$app_id' does not match bundle '$bundle_id'" >&2
      exit 1
      ;;
  esac

  if ! /usr/libexec/PlistBuddy -c 'Print :com.apple.security.application-groups' "$output" 2>/dev/null \
      | /usr/bin/grep -Eq 'group\.io\.github\.ccpopy\.farmhelper[[:space:]]*$'; then
    echo "$label profile is missing App Group group.io.github.ccpopy.farmhelper" >&2
    exit 1
  fi

  if ! /usr/libexec/PlistBuddy -c 'Print :keychain-access-groups' "$output" 2>/dev/null \
      | /usr/bin/grep -Eq '\.io\.github\.ccpopy\.farmhelper[[:space:]]*$'; then
    echo "$label profile is missing shared Keychain group *.io.github.ccpopy.farmhelper" >&2
    exit 1
  fi
}

extract_entitlements "$APP_PROFILE" "$TMP/app-entitlements.plist" "$APP_BUNDLE" "app"
extract_entitlements "$TUNNEL_PROFILE" "$TMP/tunnel-entitlements.plist" "$APPEX_BUNDLE" "tunnel"

if ! /usr/libexec/PlistBuddy -c 'Print :com.apple.developer.networking.networkextension' \
    "$TMP/tunnel-entitlements.plist" 2>/dev/null | /usr/bin/grep -Fq 'packet-tunnel-provider'; then
  echo "Tunnel profile is missing Network Extension entitlement packet-tunnel-provider" >&2
  exit 1
fi

if ! /usr/libexec/PlistBuddy -c 'Print :com.apple.developer.networking.networkextension' \
    "$TMP/app-entitlements.plist" 2>/dev/null | /usr/bin/grep -Fq 'packet-tunnel-provider'; then
  echo "App profile is missing Network Extension entitlement packet-tunnel-provider; NETunnelProviderManager will return permission denied." >&2
  exit 1
fi

APP_TEAM=$(/usr/libexec/PlistBuddy -c 'Print :com.apple.developer.team-identifier' "$TMP/app-entitlements.plist" 2>/dev/null || true)
TUNNEL_TEAM=$(/usr/libexec/PlistBuddy -c 'Print :com.apple.developer.team-identifier' "$TMP/tunnel-entitlements.plist" 2>/dev/null || true)
if [ -z "$APP_TEAM" ] || [ "$APP_TEAM" != "$TUNNEL_TEAM" ]; then
  echo "App and tunnel profiles are from different teams ('$APP_TEAM' vs '$TUNNEL_TEAM')." >&2
  exit 1
fi

# The profile has to sit inside the bundle it belongs to, exactly as Xcode exports it.
/bin/cp "$APP_PROFILE" "$APP/embedded.mobileprovision"
/bin/cp "$TUNNEL_PROFILE" "$APPEX/embedded.mobileprovision"

# Sign nested code first, then the containing app.
/usr/bin/codesign --force --timestamp --generate-entitlement-der \
  --sign "$IDENTITY" --entitlements "$TMP/tunnel-entitlements.plist" "$APPEX"
/usr/bin/codesign --force --timestamp --generate-entitlement-der \
  --sign "$IDENTITY" --entitlements "$TMP/app-entitlements.plist" "$APP"

/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP"
/usr/bin/codesign -d --entitlements :- "$APP" >/dev/null 2>&1 || true

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
/bin/bash "$SCRIPT_DIR/package-ipa.sh" "$APP" "$OUT"
echo "Signed $OUT"
