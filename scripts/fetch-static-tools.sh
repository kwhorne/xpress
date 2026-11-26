#!/usr/bin/env bash
#
# Download self-contained (statically/system-only linked) tool binaries to bundle
# inside xpress.app — no Homebrew required, for maintainers or the release CI.
#
# Currently fetches a portable ffmpeg (video + audio). Images and PDFs are
# handled in pure Rust and need no binary (except PDF `extract-pages`, which
# uses Ghostscript when it's installed).
#
# Usage:
#   scripts/fetch-static-tools.sh <target-triple> <dest-dir>
#     target-triple: aarch64-apple-darwin (releases are Apple silicon only)
#
# ffmpeg is Martin Riedl's static macOS release build
# (https://ffmpeg.martin-riedl.de, build script:
# https://git.martin-riedl.de/ffmpeg/build-script); it links only against
# macOS system frameworks. The download is checked against the SHA-256 pinned
# below *before* it is unpacked or run; a changed or tampered file stops the
# build. To move to a new build: download it, check that `ffmpeg -L` doesn't
# say "nonfree" / "not legally redistributable", update FFMPEG_VERSION,
# FFMPEG_BUILD and the checksum together, and publish a matching source
# release (see NOTICE.md).
#
# Licence: this ffmpeg is configured with --enable-gpl --enable-version3
# (GPL-3.0). make-app.sh ships the licence texts from assets/licenses/ in the
# app; NOTICE.md says where its source code is.

set -euo pipefail

target="${1:?usage: fetch-static-tools.sh <target-triple> <dest-dir>}"
dest="${2:?usage: fetch-static-tools.sh <target-triple> <dest-dir>}"

FFMPEG_VERSION="9.0.2"
FFMPEG_BUILD="1789931890"
case "$target" in
  aarch64-apple-darwin)
    ff_url="https://ffmpeg.martin-riedl.de/download/macos/arm64/${FFMPEG_BUILD}_${FFMPEG_VERSION}/ffmpeg.zip"
    ff_sha256="c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924" ;;
  *) echo "unsupported target: $target (bundled ffmpeg is Apple silicon only)" >&2; exit 1 ;;
esac

mkdir -p "$dest"
tmp="$(mktemp -d "$dest/.ffmpeg.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

echo "==> Downloading ffmpeg ($target)"
curl -fsSL --proto '=https' --tlsv1.2 -o "$tmp/ffmpeg.zip" "$ff_url"

actual="$(shasum -a 256 "$tmp/ffmpeg.zip" | awk '{print $1}')"
if [[ "$actual" != "$ff_sha256" ]]; then
  echo "ffmpeg checksum mismatch for $ff_url" >&2
  echo "  expected $ff_sha256" >&2
  echo "  got      $actual" >&2
  exit 1
fi
echo "    ✓ sha256 $actual"

unzip -q "$tmp/ffmpeg.zip" ffmpeg -d "$tmp"
mv -f "$tmp/ffmpeg" "$dest/ffmpeg"
chmod +x "$dest/ffmpeg"
"$dest/ffmpeg" -version | head -1
# Never bundle a build that ffmpeg itself calls unredistributable.
if "$dest/ffmpeg" -hide_banner -L 2>&1 | grep -qi 'nonfree\|not legally redistributable'; then
  echo "ffmpeg build has nonfree parts and may not be redistributed" >&2
  rm -f "$dest/ffmpeg"
  exit 1
fi
echo "    ✓ licence: GPL-3.0 (no nonfree parts)"

echo "==> Done. Bundled tools in: $dest"
