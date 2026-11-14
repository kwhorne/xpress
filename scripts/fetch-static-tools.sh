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
#     target-triple: aarch64-apple-darwin (releases are Apple silicon only)
#
# ffmpeg builds come from the eugeneware/ffmpeg-static project and link only
# against macOS system frameworks. Each download is checked against the
# SHA-256 pinned below *before* it is made executable or run; a changed or
# tampered file stops the build. To move to a new build, download it, review
# it, and update the tag and both checksums together.
#
# Licence: this ffmpeg is configured with --enable-gpl --enable-version3
# (GPL-3.0). make-app.sh ships the licence texts from assets/licenses/ in the
# app; NOTICE.md says where its source code is.

set -euo pipefail

target="${1:?usage: fetch-static-tools.sh <target-triple> <dest-dir>}"
dest="${2:?usage: fetch-static-tools.sh <target-triple> <dest-dir>}"

FFMPEG_TAG="b6.0"
base="https://github.com/eugeneware/ffmpeg-static/releases/download/${FFMPEG_TAG}"
case "$target" in
  aarch64-apple-darwin)
    ff_url="$base/ffmpeg-darwin-arm64"
    ff_sha256="a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584" ;;
  *) echo "unsupported target: $target" >&2; exit 1 ;;
esac

mkdir -p "$dest"
tmp="$(mktemp "$dest/.ffmpeg.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

echo "==> Downloading ffmpeg ($target)"
curl -fsSL --proto '=https' --tlsv1.2 -o "$tmp" "$ff_url"

actual="$(shasum -a 256 "$tmp" | awk '{print $1}')"
if [[ "$actual" != "$ff_sha256" ]]; then
  echo "ffmpeg checksum mismatch for $ff_url" >&2
  echo "  expected $ff_sha256" >&2
  echo "  got      $actual" >&2
  exit 1
fi
echo "    ✓ sha256 $actual"

mv "$tmp" "$dest/ffmpeg"
trap - EXIT
chmod +x "$dest/ffmpeg"
"$dest/ffmpeg" -version | head -1

echo "==> Done. Bundled tools in: $dest"
echo "    (PDF/ghostscript is not bundled; install 'gs' for PDF optimisation.)"
