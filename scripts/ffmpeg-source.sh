#!/usr/bin/env bash
#
# Collect the Corresponding Source (GPL-3.0) of the ffmpeg that the macOS app
# bundles: FFmpeg itself, every library statically linked into it, and the
# build script that configured and built it. The result is published as a
# GitHub release (see NOTICE.md), once per bundled ffmpeg build.
#
# Usage:
#   scripts/ffmpeg-source.sh <out-dir>
#
# The bundled binary is Martin Riedl's release build (scripts/fetch-static-
# tools.sh), made by https://git.martin-riedl.de/ffmpeg/build-script at
# BUILD_SCRIPT_COMMIT; that commit's version/ folder pins each library, and
# its script/build-*.sh say where each source comes from. When moving to a
# new ffmpeg build, update FFMPEG_VERSION, BUILD_SCRIPT_COMMIT and
# X264_COMMIT (x264 is built from master: the last commit before the build).

set -euo pipefail

FFMPEG_VERSION="9.0.2"
BUILD_SCRIPT_REPO="https://git.martin-riedl.de/ffmpeg/build-script.git"
BUILD_SCRIPT_COMMIT="6a611e19870e197bc37c6e4c7fccddebd3715466"
X264_COMMIT="0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee"

out="${1:?usage: ffmpeg-source.sh <out-dir>}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "==> Build script $BUILD_SCRIPT_COMMIT"
git clone -q "$BUILD_SCRIPT_REPO" "$work/build-script"
git -C "$work/build-script" -c advice.detachedHead=false checkout -q "$BUILD_SCRIPT_COMMIT"
git -C "$work/build-script" archive --format=tar.gz \
  --prefix="ffmpeg-build-script-${BUILD_SCRIPT_COMMIT:0:7}/" \
  -o "$out/ffmpeg-build-script-${BUILD_SCRIPT_COMMIT:0:7}.tar.gz" HEAD

v() { tr -d '[:space:]' < "$work/build-script/version/$1"; }
built="$(v ffmpeg)"
if [[ "$built" != "$FFMPEG_VERSION" ]]; then
  echo "build script $BUILD_SCRIPT_COMMIT builds ffmpeg $built, not $FFMPEG_VERSION" >&2
  exit 1
fi

fetch() { # <file name> <url>
  echo "    $1"
  curl -fsSL --retry 3 --proto '=https,http' -o "$out/$1" "$2"
}

echo "==> Sources"
# The URLs follow script/build-*.sh at BUILD_SCRIPT_COMMIT.
x=$(v ffmpeg);     fetch "ffmpeg-$x.tar.bz2"        "https://ffmpeg.org/releases/ffmpeg-$x.tar.bz2"
x=$(v aom);        fetch "libaom-$x.tar.gz"         "https://storage.googleapis.com/aom-releases/libaom-$x.tar.gz"
x=$(v dav1d);      fetch "dav1d-$x.tar.gz"          "https://code.videolan.org/videolan/dav1d/-/archive/$x/dav1d-$x.tar.gz"
x=$(v fontconfig); fetch "fontconfig-$x.tar.xz"     "https://gitlab.freedesktop.org/api/v4/projects/890/packages/generic/fontconfig/$x/fontconfig-$x.tar.xz"
x=$(v freetype);   fetch "freetype-$x.tar.gz"       "https://download.savannah.gnu.org/releases/freetype/freetype-$x.tar.gz"
x=$(v fribidi);    fetch "fribidi-$x.tar.xz"        "https://github.com/fribidi/fribidi/releases/download/v$x/fribidi-$x.tar.xz"
x=$(v harfbuzz);   fetch "harfbuzz-$x.tar.xz"       "https://github.com/harfbuzz/harfbuzz/releases/download/$x/harfbuzz-$x.tar.xz"
x=$(v lame);       fetch "lame-$x.tar.gz"           "https://unlimited.dl.sourceforge.net/project/lame/lame/$x/lame-$x.tar.gz"
x=$(v libass);     fetch "libass-$x.tar.gz"         "https://github.com/libass/libass/releases/download/$x/libass-$x.tar.gz"
x=$(v libbluray);  fetch "libbluray-$x.tar.xz"      "https://download.videolan.org/pub/videolan/libbluray/$x/libbluray-$x.tar.xz"
x=$(v libiconv);   fetch "libiconv-$x.tar.gz"       "https://ftp.gnu.org/pub/gnu/libiconv/libiconv-$x.tar.gz"
x=$(v libklvanc);  fetch "libklvanc-$x.tar.gz"      "https://github.com/stoth68000/libklvanc/archive/refs/tags/vid.obe.$x.tar.gz"
x=$(v libogg);     fetch "libogg-$x.tar.gz"         "https://ftp.osuosl.org/pub/xiph/releases/ogg/libogg-$x.tar.gz"
x=$(v libtheora);  fetch "libtheora-$x.tar.gz"      "https://downloads.xiph.org/releases/theora/libtheora-$x.tar.gz"
x=$(v libvmaf);    fetch "vmaf-$x.tar.gz"           "https://github.com/Netflix/vmaf/archive/refs/tags/v$x.tar.gz"
x=$(v libvorbis);  fetch "libvorbis-$x.tar.gz"      "https://ftp.osuosl.org/pub/xiph/releases/vorbis/libvorbis-$x.tar.gz"
x=$(v libwebp);    fetch "libwebp-$x.tar.gz"        "https://github.com/webmproject/libwebp/archive/refs/tags/v$x.tar.gz"
x=$(v libxml2);    fetch "libxml2-$x.tar.xz"        "https://download.gnome.org/sources/libxml2/${x%.*}/libxml2-$x.tar.xz"
x=$(v openh264);   fetch "openh264-$x.tar.gz"       "https://github.com/cisco/openh264/archive/v$x.tar.gz"
x=$(v openjpeg);   fetch "openjpeg-$x.tar.gz"       "https://github.com/uclouvain/openjpeg/archive/refs/tags/v$x.tar.gz"
x=$(v openssl);    fetch "openssl-$x.tar.gz"        "https://github.com/openssl/openssl/releases/download/openssl-$x/openssl-$x.tar.gz"
x=$(v opus);       fetch "opus-$x.tar.gz"           "https://downloads.xiph.org/releases/opus/opus-$x.tar.gz"
x=$(v rav1e);      fetch "rav1e-$x.tar.gz"          "https://github.com/xiph/rav1e/archive/refs/tags/v$x.tar.gz"
x=$(v sdl);        fetch "SDL2-$x.tar.gz"           "https://www.libsdl.org/release/SDL2-$x.tar.gz"
x=$(v snappy);     fetch "snappy-$x.tar.gz"         "https://github.com/google/snappy/archive/refs/tags/$x.tar.gz"
x=$(v srt);        fetch "srt-$x.tar.gz"            "https://github.com/Haivision/srt/archive/refs/tags/v$x.tar.gz"
x=$(v svt-av1);    fetch "SVT-AV1-v$x.tar.gz"       "https://gitlab.com/AOMediaCodec/SVT-AV1/-/archive/v$x/SVT-AV1-v$x.tar.gz"
x=$(v vpx);        fetch "libvpx-$x.tar.gz"         "https://github.com/webmproject/libvpx/archive/v$x.tar.gz"
x=$(v vvenc);      fetch "vvenc-$x.tar.gz"          "https://github.com/fraunhoferhhi/vvenc/archive/refs/tags/v$x.tar.gz"
x=$(v x265);       fetch "x265-$x.tar.gz"           "https://bitbucket.org/multicoreware/x265_git/get/$x.tar.gz"
x=$(v zimg);       fetch "zimg-$x.tar.gz"           "https://github.com/sekrit-twc/zimg/archive/refs/tags/release-$x.tar.gz"
x=$(v zlib);       fetch "zlib-$x.tar.gz"           "https://www.zlib.net/fossils/zlib-$x.tar.gz"
x=$(v zvbi);       fetch "zvbi-$x.tar.bz2"          "https://sourceforge.net/projects/zapping/files/zvbi/$x/zvbi-$x.tar.bz2/download"
fetch "x264-${X264_COMMIT:0:10}.tar.gz" \
  "https://code.videolan.org/videolan/x264/-/archive/$X264_COMMIT/x264-$X264_COMMIT.tar.gz"

echo "==> Checksums"
( cd "$out" && shasum -a 256 -- *.tar.* | sort -k2 > SHA256SUMS )
cat "$out/SHA256SUMS"
