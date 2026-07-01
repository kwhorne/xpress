#!/usr/bin/env bash
#
# Build a macOS .dmg disk image containing xpress.app and an Applications
# shortcut for drag-to-install.
#
# Usage:
#   scripts/make-dmg.sh [--app <path>] [--out <file.dmg>] [--version <v>]
#
# Defaults assume `scripts/make-app.sh` has produced dist/xpress.app.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/dist/xpress.app"
OUT="$ROOT/dist/xpress.dmg"
VERSION="0.1.0"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app) APP="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "make-dmg.sh only runs on macOS" >&2
  exit 1
fi
if [[ ! -d "$APP" ]]; then
  echo "app bundle not found at $APP (run scripts/make-app.sh first)" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
cp -R "$APP" "$staging/"
ln -s /Applications "$staging/Applications"

echo "==> Creating $OUT (version $VERSION)"
hdiutil create \
  -volname "xpress $VERSION" \
  -srcfolder "$staging" \
  -fs HFS+ \
  -format UDZO \
  -ov \
  "$OUT" >/dev/null

# Sign the image with a Developer ID if available, else ad-hoc.
SIGN_ID="${XPRESS_SIGN_ID:-}"
if [[ -z "$SIGN_ID" ]]; then
  SIGN_ID="$(security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Developer ID Application/{print $2; exit}')"
fi
if [[ -n "$SIGN_ID" ]]; then
  codesign --force --timestamp --sign "$SIGN_ID" "$OUT"
else
  codesign --force --sign - "$OUT" 2>/dev/null || true
fi

echo "==> Done: $OUT"

if [[ "${XPRESS_NOTARIZE:-0}" == "1" && -n "$SIGN_ID" ]]; then
  "$ROOT/scripts/notarize.sh" "$OUT"
fi
