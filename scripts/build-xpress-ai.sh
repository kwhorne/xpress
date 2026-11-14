#!/usr/bin/env bash
#
# Build the Apple Intelligence helper (tools/xpress-ai) into target/xpress-ai/.
#
# It needs Swift and a macOS SDK with the FoundationModels framework (Xcode
# or Command Line Tools 26+). Without one, this prints a note and exits 0 —
# the app then simply doesn't offer Apple Intelligence — unless
# XPRESS_REQUIRE_AI=1 (release builds), which makes that an error.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/xpress-ai/xpress-ai"

skip() {
  if [[ "${XPRESS_REQUIRE_AI:-0}" == "1" ]]; then
    echo "xpress-ai can't be built: $1 (XPRESS_REQUIRE_AI=1)" >&2
    exit 1
  fi
  echo "    (xpress-ai skipped: $1)"
  exit 0
}

if [[ "$(uname)" != "Darwin" ]] || ! command -v swiftc >/dev/null 2>&1; then
  skip "needs Swift on macOS"
fi
SDK="$(xcrun --show-sdk-path 2>/dev/null || true)"
if [[ ! -d "$SDK/System/Library/Frameworks/FoundationModels.framework" ]]; then
  skip "the macOS SDK has no FoundationModels — needs Xcode 26+"
fi

mkdir -p "$(dirname "$OUT")"
swiftc -O -parse-as-library -target arm64-apple-macos26.0 \
  "$ROOT/tools/xpress-ai/main.swift" -o "$OUT"
echo "    ✓ xpress-ai → $OUT"
