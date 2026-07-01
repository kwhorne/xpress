#!/usr/bin/env bash
#
# Notarise and staple a macOS .app (or .dmg) with Apple's notary service.
#
# Usage:
#   scripts/notarize.sh <path-to-.app-or-.dmg>
#
# Credentials (choose one), provided via environment:
#
#   A) Apple ID + app-specific password:
#        APPLE_ID=you@example.com
#        APPLE_TEAM_ID=7G383N3VY7
#        APPLE_APP_PASSWORD=xxxx-xxxx-xxxx-xxxx   # app-specific password
#
#   B) App Store Connect API key:
#        ASC_KEY_ID=XXXXXXXXXX
#        ASC_ISSUER_ID=xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
#        ASC_KEY_PATH=/path/to/AuthKey_XXXXXXXXXX.p8
#
# An app-specific password is created at https://appleid.apple.com → Sign-In & Security.

set -euo pipefail

target="${1:?usage: notarize.sh <path-to-.app-or-.dmg>}"
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "notarize.sh only runs on macOS" >&2
  exit 1
fi
if [[ ! -e "$target" ]]; then
  echo "not found: $target" >&2
  exit 1
fi

# Build the notarytool credential arguments from whichever method is configured.
cred=()
if [[ -n "${ASC_KEY_ID:-}" && -n "${ASC_ISSUER_ID:-}" && -n "${ASC_KEY_PATH:-}" ]]; then
  cred=(--key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID")
elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
  cred=(--apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD")
else
  echo "no notarisation credentials set (see the header of this script)" >&2
  exit 1
fi

# notarytool needs a zip for a .app; a .dmg can be submitted directly.
submit="$target"
cleanup=""
if [[ "$target" == *.app ]]; then
  submit="$(dirname "$target")/$(basename "$target" .app)-notarize.zip"
  ditto -c -k --keepParent "$target" "$submit"
  cleanup="$submit"
fi

echo "==> Submitting $submit to the notary service…"
xcrun notarytool submit "$submit" "${cred[@]}" --wait

echo "==> Stapling ticket to $target"
xcrun stapler staple "$target"
xcrun stapler validate "$target" && echo "==> Notarised and stapled ✓"

[[ -n "$cleanup" ]] && rm -f "$cleanup" || true
