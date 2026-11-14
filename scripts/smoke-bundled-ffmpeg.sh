#!/usr/bin/env bash
#
# Run xpress against the ffmpeg that the app bundles, through every encoder
# xpress uses, so a new ffmpeg build (new version, different options) is
# checked before it ships.
#
# Usage:
#   scripts/smoke-bundled-ffmpeg.sh <xpress-binary> <bin-dir-with-ffmpeg>

set -euo pipefail

xp="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
bin="$(cd "$2" && pwd)"
ff="$bin/ffmpeg"
export XPRESS_BIN_DIR="$bin"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

"$ff" -hide_banner -version | head -1

# Sources: a short clip with sound, and a few seconds of audio.
"$ff" -hide_banner -loglevel error -y \
  -f lavfi -i testsrc2=size=640x360:rate=30:duration=3 \
  -f lavfi -i sine=frequency=440:duration=3 \
  -c:v libx264 -preset ultrafast -crf 10 -pix_fmt yuv420p -c:a aac -shortest clip.mov
"$ff" -hide_banner -loglevel error -y -f lavfi -i sine=frequency=440:duration=3 -c:a pcm_s16le tone.wav

check() { # <description> <file>
  if [[ ! -s "$2" ]]; then
    echo "✗ $1: $2 missing or empty" >&2
    exit 1
  fi
  echo "✓ $1 ($(wc -c < "$2" | tr -d ' ') bytes)"
}
fresh() { rm -f "$work"/out.* "$work"/clip.{mp4,webm,gif} "$work"/tone.{mp3,ogg,m4a,flac}; }

# Video: every software encoder (libx264, libx265, libvpx-vp9, libsvtav1),
# the GIF path, two-pass libx264 for a size budget, and metadata stripping.
fresh; "$xp" convert -q --no-backup --to mp4 clip.mov;  check "H.264 (libx264)" clip.mp4
fresh; "$xp" convert -q --no-backup --to hevc clip.mov; check "HEVC (libx265)" clip.mp4
fresh; "$xp" convert -q --no-backup --to webm clip.mov; check "VP9 (libvpx)" clip.webm
fresh; "$xp" convert -q --no-backup --to av1 clip.mov -o out.mp4; check "AV1 (libsvtav1)" out.mp4
fresh; "$xp" convert -q --no-backup --to gif clip.mov;  check "GIF" clip.gif
# (Optimising writes .mp4 next to a .mov, so start from an .mp4.)
"$ff" -hide_banner -loglevel error -y -i clip.mov -c copy src.mp4
fresh; cp src.mp4 out.mp4
"$xp" optimise -q --no-backup --compression 80 out.mp4; check "optimise, smaller" out.mp4
fresh; cp src.mp4 out.mp4
"$xp" optimise -q --no-backup --max-size 150kb out.mp4; check "size budget (two-pass)" out.mp4
size=$(wc -c < out.mp4 | tr -d ' ')
if (( size > 160000 )); then
  echo "✗ size budget: $size bytes, wanted about 150 kB" >&2
  exit 1
fi
fresh; cp src.mp4 out.mp4
"$xp" optimise -q --no-backup --allow-larger --strip-metadata out.mp4; check "strip metadata" out.mp4

# Audio: MP3 (libmp3lame), Opus (libopus), AAC, FLAC, loudness.
fresh; "$xp" convert -q --no-backup --to mp3 tone.wav;  check "MP3 (libmp3lame)" tone.mp3
fresh; "$xp" convert -q --no-backup --to opus tone.wav; check "Opus (libopus)" tone.ogg
fresh; "$xp" convert -q --no-backup --to aac tone.wav; check "AAC" tone.m4a
fresh; "$xp" convert -q --no-backup --to flac tone.wav; check "FLAC" tone.flac
fresh; "$xp" pipeline run -q --no-backup 'normalize(lufs: -16) -> convert(to: mp3)' tone.wav
check "normalize -> MP3" tone.mp3

echo "==> bundled ffmpeg works with xpress"
