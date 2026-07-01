#!/usr/bin/env bash
#
# Download self-contained (statically/system-only linked) tool binaries to bundle
# inside xpress.app — no Homebrew required, for maintainers or the release CI.
#
# Currently fetches a portable ffmpeg (video + audio). Images are handled in
# pure Rust and need no binary; PDF still needs ghostscript installed separately.
#
# Usage:
#   scripts/fetch-static-tools.sh <target-triple> <dest-dir>
#     target-triple: aarch64-apple-darwin | x86_64-apple-darwin
#
# ffmpeg builds come from the well-known eugeneware/ffmpeg-static project and
# link only against macOS system frameworks.

set -euo pipefail

target="${1:?usage: fetch-static-tools.sh <target-triple> <dest-dir>}"
dest="${2:?usage: fetch-static-tools.sh <target-triple> <dest-dir>}"

FFMPEG_TAG="b6.0"
case "$target" in
  aarch64-apple-darwin) ff_url="https://github.com/eugeneware/ffmpeg-static/releases/download/${FFMPEG_TAG}/ffmpeg-darwin-arm64" ;;
  x86_64-apple-darwin)  ff_url="https://github.com/eugeneware/ffmpeg-static/releases/download/${FFMPEG_TAG}/ffmpeg-darwin-x64" ;;
  *) echo "unsupported target: $target" >&2; exit 1 ;;
esac

mkdir -p "$dest"

echo "==> Downloading ffmpeg ($target)"
curl -fsSL -o "$dest/ffmpeg" "$ff_url"
chmod +x "$dest/ffmpeg"
"$dest/ffmpeg" -version | head -1

echo "==> Done. Bundled tools in: $dest"
echo "    (PDF/ghostscript is not bundled; install 'gs' for PDF optimisation.)"
