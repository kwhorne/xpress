#!/usr/bin/env bash
#
# Fetch the external optimisation tools xpress drives and place them either in the
# per-user bundle dir (default) or in vendor/bin/<target>/ for embedding.
#
# Usage:
#   scripts/fetch-tools.sh                 # install into the per-user bundle dir
#   scripts/fetch-tools.sh --vendor        # install into vendor/bin/<target> + link current/
#   scripts/fetch-tools.sh --tools ffmpeg,pngquant
#
# This script prefers a package manager when available (brew on macOS, apt/dnf on
# Linux) and copies the resolved binaries into the destination so xpress ships
# self-contained. For fully static/portable builds, drop prebuilt binaries into
# the destination directory manually.

set -euo pipefail

TOOLS_DEFAULT="ffmpeg pngquant jpegoptim gifsicle gs vips gifski cwebp exiftool"
TOOLS="$TOOLS_DEFAULT"
MODE="bundle"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --vendor) MODE="vendor"; shift ;;
    --bundle) MODE="bundle"; shift ;;
    --tools)  TOOLS="${2//,/ }"; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

os="$(uname -s)"; arch="$(uname -m)"
case "$os-$arch" in
  Darwin-arm64)   triple="aarch64-apple-darwin" ;;
  Darwin-x86_64)  triple="x86_64-apple-darwin" ;;
  Linux-x86_64)   triple="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  triple="aarch64-unknown-linux-gnu" ;;
  *) echo "unsupported platform: $os-$arch" >&2; exit 1 ;;
esac

root="$(cd "$(dirname "$0")/.." && pwd)"
if [[ "$MODE" == "vendor" ]]; then
  dest="$root/vendor/bin/$triple"
else
  case "$os" in
    Darwin) dest="$HOME/Library/Application Support/xpress/bin" ;;
    Linux)  dest="${XDG_DATA_HOME:-$HOME/.local/share}/xpress/bin" ;;
  esac
fi
mkdir -p "$dest"
echo "==> Destination: $dest"

# Make sure the tools exist on this machine via a package manager.
install_pkg() {
  if command -v brew >/dev/null 2>&1; then
    brew list "$1" >/dev/null 2>&1 || brew install "$1" || true
  elif command -v apt-get >/dev/null 2>&1; then
    sudo apt-get install -y "$1" || true
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y "$1" || true
  fi
}

# Map our tool names to package names where they differ.
pkg_for() {
  case "$1" in
    gs)    echo "ghostscript" ;;
    cwebp) echo "webp" ;;
    vips)  echo "vips" ;;
    *)     echo "$1" ;;
  esac
}

copied=0
for tool in $TOOLS; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "==> $tool missing, attempting install ($(pkg_for "$tool"))"
    install_pkg "$(pkg_for "$tool")"
  fi
  if src="$(command -v "$tool" 2>/dev/null)"; then
    # Resolve symlinks so we copy the real binary.
    real="$(readlink -f "$src" 2>/dev/null || echo "$src")"
    cp -f "$real" "$dest/$tool"
    chmod +x "$dest/$tool"
    echo "    ✓ $tool -> $dest/$tool"
    copied=$((copied+1))
  else
    echo "    ✗ $tool not available" >&2
  fi
done

if [[ "$MODE" == "vendor" ]]; then
  ln -sfn "$triple" "$root/vendor/bin/current"
  echo "==> Linked vendor/bin/current -> $triple"
  echo "    Build a self-contained binary with: cargo build --release --features embed-tools"
fi

echo "==> Done: $copied tool(s) placed."
echo "    Verify with: xpress doctor"
