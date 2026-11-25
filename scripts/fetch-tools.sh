#!/usr/bin/env bash
#
# Fetch the external optimisation tools xpress drives and place them either in the
# per-user bundle dir (default) or in vendor/bin/<target>/ for embedding.
#
# Usage:
#   scripts/fetch-tools.sh                 # install into the per-user bundle dir
#   scripts/fetch-tools.sh --vendor        # install into vendor/bin/<target> + copy to current/
#   scripts/fetch-tools.sh --tools ffmpeg,gifski
#
# This script prefers a package manager when available (brew on macOS, apt/dnf on
# Linux) and copies the resolved binaries into the destination so xpress ships
# self-contained. For fully static/portable builds, drop prebuilt binaries into
# the destination directory manually.
#
# exiftool is a Perl script with its library beside it: it can't be copied as
# one file, so it is only installed (system-wide), never copied.

set -euo pipefail

# The tools xpress actually runs (images and PDFs need none).
TOOLS_DEFAULT="ffmpeg gifsicle gifski gs heif-enc cjxl exiftool"
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
  Linux-x86_64)   triple="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  triple="aarch64-unknown-linux-gnu" ;;
  *) echo "unsupported platform: $os-$arch (macOS: Apple silicon only)" >&2; exit 1 ;;
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
    gs)       echo "ghostscript" ;;
    heif-enc) echo "libheif" ;;
    cjxl)     echo "jpeg-xl" ;;
    *)        echo "$1" ;;
  esac
}

copied=0
for tool in $TOOLS; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "==> $tool missing, attempting install ($(pkg_for "$tool"))"
    install_pkg "$(pkg_for "$tool")"
  fi
  if [[ "$tool" == "exiftool" ]]; then
    if command -v exiftool >/dev/null 2>&1; then
      echo "    ✓ exiftool installed system-wide (not copied: it needs its Perl library)"
    else
      echo "    ✗ exiftool not available" >&2
    fi
    continue
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
  # `current/` is a tracked folder (it keeps a .gitkeep), so it gets a copy of
  # this target's binaries — a symlink would land *inside* it.
  current="$root/vendor/bin/current"
  find "$current" -mindepth 1 ! -name .gitkeep -delete
  find "$dest" -maxdepth 1 -type f -exec cp -f {} "$current/" \;
  echo "==> Copied $triple into vendor/bin/current"
  echo "    Build a self-contained binary with: cargo build --release --features embed-tools"
fi

echo "==> Done: $copied tool(s) placed."
echo "    Verify with: xpress doctor"
