#!/usr/bin/env bash
#
# Build the Apple Intelligence helper (tools/xpress-ai) into target/xpress-ai/.
#
# It needs Swift and a macOS SDK with the FoundationModels framework (Xcode
# or Command Line Tools 26+). Without one, this prints a note and exits 0 —
# the app then simply doesn't offer Apple Intelligence.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/xpress-ai/xpress-ai"

if [[ "$(uname)" != "Darwin" ]] || ! command -v swiftc >/dev/null 2>&1; then
  echo "    (xpress-ai skipped: needs Swift on macOS)"
  exit 0
fi
SDK="$(xcrun --show-sdk-path 2>/dev/null || true)"
if [[ ! -d "$SDK/System/Library/Frameworks/FoundationModels.framework" ]]; then
  echo "    (xpress-ai skipped: the macOS SDK has no FoundationModels — needs Xcode 26+)"
  exit 0
fi

mkdir -p "$(dirname "$OUT")"
swiftc -O -parse-as-library -target arm64-apple-macos26.0 \
  "$ROOT/tools/xpress-ai/main.swift" -o "$OUT"
echo "    ✓ xpress-ai → $OUT"
