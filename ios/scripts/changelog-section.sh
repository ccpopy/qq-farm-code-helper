#!/usr/bin/env bash
# Prints the changelog section whose heading is exactly "## <tag>".
# Usage: changelog-section.sh <tag> [changelog]
set -euo pipefail

TAG="${1:?Tag is required}"
FILE="${2:-ios/CHANGELOG.md}"
[ -f "$FILE" ] || { echo "Changelog not found: $FILE" >&2; exit 1; }

/usr/bin/awk -v header="## $TAG" '
  $0 == header { found = 1; next }
  found && /^## / { exit }
  found { print }
' "$FILE"
