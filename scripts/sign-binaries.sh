#!/usr/bin/env bash
#
# Sign standalone macOS executables (the CLI and GUI binaries shipped in the
# release tarball) with a Developer ID, hardened runtime and a secure
# timestamp, and notarise them when credentials are set.
#
# Usage:
#   scripts/sign-binaries.sh <binary>...
#
# Identity: $XPRESS_SIGN_ID, else the first "Developer ID Application" in the
# keychain; without one, the binaries are left ad-hoc signed (local use).
# Notarisation: set XPRESS_NOTARIZE=1 plus the credentials described in
# scripts/notarize.sh. Bare executables can't be stapled, so Gatekeeper checks
# the notarisation online the first time one runs.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ $# -eq 0 ]]; then
  echo "usage: sign-binaries.sh <binary>..." >&2
  exit 1
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "sign-binaries.sh only runs on macOS" >&2
  exit 1
fi

SIGN_ID="${XPRESS_SIGN_ID:-}"
if [[ -z "$SIGN_ID" ]]; then
  SIGN_ID="$(security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Developer ID Application/{print $2; exit}')"
fi
if [[ -z "$SIGN_ID" ]]; then
  echo "==> No Developer ID found — leaving binaries ad-hoc signed"
  exit 0
fi

echo "==> Signing with: $SIGN_ID"
for bin in "$@"; do
  codesign --force --options runtime --timestamp --sign "$SIGN_ID" "$bin"
  codesign --verify --strict --verbose=2 "$bin" && echo "    ✓ $(basename "$bin")"
done

if [[ "${XPRESS_NOTARIZE:-0}" == "1" ]]; then
  # notarytool takes one archive; ditto keeps the signatures intact.
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  mkdir "$work/binaries"
  cp "$@" "$work/binaries/"
  ditto -c -k "$work/binaries" "$work/binaries.zip"
  "$ROOT/scripts/notarize.sh" "$work/binaries.zip"
fi
