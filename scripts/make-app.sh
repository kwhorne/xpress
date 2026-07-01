#!/usr/bin/env bash
#
# Build a macOS .app bundle for the xpress GUI.
#
# Usage:
#   scripts/make-app.sh [--gui <path>] [--cli <path>] [--out <dir>] [--version <v>] [--tools]
#
# Defaults assume a release build in target/release. With --tools, the external
# optimisation binaries found on $PATH (or in vendor/bin) are copied into
# Contents/Resources/bin so the app is self-contained.
#
# The bundle is ad-hoc code-signed (`codesign -s -`), which is enough to run
# locally. For distribution, sign with a Developer ID and notarise — see the
# notes at the bottom of this script.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUI_BIN="$ROOT/target/release/xpress-gui"
CLI_BIN="$ROOT/target/release/xpress"
OUT_DIR="$ROOT/dist"
VERSION="0.1.0"
WITH_TOOLS=0
BUNDLE_ID="com.kwhorne.xpress"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --gui) GUI_BIN="$2"; shift 2 ;;
    --cli) CLI_BIN="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --tools) WITH_TOOLS=1; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "make-app.sh only runs on macOS" >&2
  exit 1
fi
if [[ ! -x "$GUI_BIN" ]]; then
  echo "GUI binary not found at $GUI_BIN (build with: cargo build --release -p xpress-gui)" >&2
  exit 1
fi

APP="$OUT_DIR/xpress.app"
echo "==> Building $APP (version $VERSION)"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$GUI_BIN" "$APP/Contents/MacOS/xpress"
chmod +x "$APP/Contents/MacOS/xpress"
# The CLI is shipped separately (tar.gz / Homebrew), not inside the GUI bundle:
# a second executable in Contents/MacOS breaks code-signing/notarisation.

if [[ "$WITH_TOOLS" == "1" ]]; then
  echo "==> Bundling optimisation tools into Resources/bin"
  mkdir -p "$APP/Contents/Resources/bin"
  for tool in ffmpeg pngquant jpegoptim gifsicle gs vips gifski cwebp exiftool; do
    src=""
    if [[ -x "$ROOT/vendor/bin/current/$tool" ]]; then
      src="$ROOT/vendor/bin/current/$tool"
    elif command -v "$tool" >/dev/null 2>&1; then
      src="$(command -v "$tool")"
    fi
    if [[ -n "$src" ]]; then
      real="$(readlink -f "$src" 2>/dev/null || echo "$src")"
      cp -f "$real" "$APP/Contents/Resources/bin/$tool"
      chmod +x "$APP/Contents/Resources/bin/$tool"
      echo "    ✓ $tool"
    fi
  done
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>            <string>xpress</string>
    <key>CFBundleDisplayName</key>     <string>xpress</string>
    <key>CFBundleIdentifier</key>      <string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key>         <string>$VERSION</string>
    <key>CFBundleShortVersionString</key> <string>$VERSION</string>
    <key>CFBundleExecutable</key>      <string>xpress</string>
    <key>CFBundlePackageType</key>     <string>APPL</string>
    <key>LSMinimumSystemVersion</key>  <string>11.0</string>
    <key>NSHighResolutionCapable</key> <true/>
    <key>LSApplicationCategoryType</key> <string>public.app-category.utilities</string>
PLIST

# Reference an icon only if one was provided.
if [[ -f "$ROOT/assets/AppIcon.icns" ]]; then
  cp "$ROOT/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
  echo "    <key>CFBundleIconFile</key> <string>AppIcon</string>" >> "$APP/Contents/Info.plist"
fi

cat >> "$APP/Contents/Info.plist" <<'PLIST'
</dict>
</plist>
PLIST

# Sign with a Developer ID if available (from $XPRESS_SIGN_ID or the keychain),
# otherwise fall back to an ad-hoc signature for local use.
SIGN_ID="${XPRESS_SIGN_ID:-}"
if [[ -z "$SIGN_ID" ]]; then
  SIGN_ID="$(security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Developer ID Application/{print $2; exit}')"
fi

if [[ -n "$SIGN_ID" ]]; then
  echo "==> Signing with: $SIGN_ID"
  # Sign nested Mach-O binaries first (bundled tools), then the bundle.
  if [[ -d "$APP/Contents/Resources/bin" ]]; then
    while IFS= read -r f; do
      codesign --force --options runtime --timestamp --sign "$SIGN_ID" "$f" || true
    done < <(find "$APP/Contents/Resources/bin" -type f)
  fi
  codesign --force --options runtime --timestamp --sign "$SIGN_ID" "$APP"
  codesign --verify --deep --strict --verbose=2 "$APP" && echo "    ✓ signature valid"
else
  codesign --force --deep --sign - "$APP" 2>/dev/null \
    && echo "    (ad-hoc signed — for local use only)" \
    || echo "    (codesign unavailable — bundle is unsigned)"
fi

echo "==> Done: $APP"

# Notarise when credentials are provided (Apple ID + app-specific password, or
# an App Store Connect API key). Set XPRESS_NOTARIZE=1 to enable.
if [[ "${XPRESS_NOTARIZE:-0}" == "1" && -n "$SIGN_ID" ]]; then
  "$ROOT/scripts/notarize.sh" "$APP"
fi

# ---------------------------------------------------------------------------
# Distribution (Developer ID signing + notarisation), for reference:
#
#   codesign --force --options runtime --timestamp \
#     --sign "Developer ID Application: Your Name (TEAMID)" "$APP"
#   ditto -c -k --keepParent "$APP" xpress.zip
#   xcrun notarytool submit xpress.zip \
#     --apple-id you@example.com --team-id TEAMID --password APP_SPECIFIC_PWD --wait
#   xcrun stapler staple "$APP"
# ---------------------------------------------------------------------------
