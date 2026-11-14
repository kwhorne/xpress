#!/usr/bin/env bash
#
# Entry point of the `xpress check` GitHub Action (action.yml): download the
# xpress release binary for this runner, verify it against the release's
# SHA-256 file, and run `xpress check`.
#
# Inputs arrive as environment variables:
#   XP_PATHS        files/folders to check (space separated)       [.]
#   XP_MAX_SIZE     fail for files larger than this, e.g. 500kb    []
#   XP_MIN_SAVINGS  fail when optimising would save >= this %      [10]
#   XP_QUALITY      how good an optimised image must still look    [visually-lossless]
#   XP_EXCLUDE      directory names to skip (space separated)      [node_modules .git]
#   XP_VERSION      release tag, e.g. v0.5.0, or "latest"          [latest]
#   RUNNER_OS, RUNNER_ARCH, RUNNER_TEMP — provided by GitHub Actions.

set -euo pipefail

repo="kwhorne/xpress"
exe="xpress"
archive="tar.gz"
case "${RUNNER_OS:-}-${RUNNER_ARCH:-}" in
  Linux-X64) target="x86_64-unknown-linux-gnu" ;;
  macOS-ARM64) target="aarch64-apple-darwin" ;;
  macOS-X64) target="x86_64-apple-darwin" ;;
  Windows-X64) target="x86_64-pc-windows-msvc"; exe="xpress.exe"; archive="zip" ;;
  *)
    echo "::error::xpress check: no release binary for ${RUNNER_OS:-?}/${RUNNER_ARCH:-?} (use ubuntu-latest, macos-latest, macos-13 or windows-latest)"
    exit 1
    ;;
esac

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

version="${XP_VERSION:-latest}"
if [[ "$version" == "latest" ]]; then
  # The /releases/latest page redirects to /releases/tag/<tag>.
  version="$(curl -fsSLo /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")"
  version="${version##*/}"
fi
if [[ ! "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-.+][0-9A-Za-z.-]+)?$ ]]; then
  echo "::error::xpress check: '$version' is not a release tag like v1.0.0"
  exit 1
fi

dir="${RUNNER_TEMP:-/tmp}/xpress-$version"
name="xpress-$version-$target"
if [[ ! -x "$dir/$name/$exe" ]]; then
  mkdir -p "$dir"
  url="https://github.com/$repo/releases/download/$version/$name.$archive"
  echo "Downloading xpress $version ($target)…"
  curl -fsSL --proto '=https' -o "$dir/$name.$archive" "$url"
  curl -fsSL --proto '=https' -o "$dir/$name.$archive.sha256" "$url.sha256"
  expected="$(awk '{print $1; exit}' "$dir/$name.$archive.sha256")"
  actual="$(sha256 "$dir/$name.$archive")"
  if [[ -z "$expected" || "$expected" != "$actual" ]]; then
    echo "::error::xpress check: checksum mismatch for $name.$archive (expected '$expected', got '$actual')"
    exit 1
  fi
  # bsdtar (macOS, Windows) and GNU tar both read the archive; Windows
  # releases are .zip, which only bsdtar extracts.
  tar -xf "$dir/$name.$archive" -C "$dir"
fi

args=(check --recursive --min-savings "${XP_MIN_SAVINGS:-10}" --quality "${XP_QUALITY:-visually-lossless}")
if [[ -n "${XP_MAX_SIZE:-}" ]]; then
  args+=(--max-size "$XP_MAX_SIZE")
fi
# Space-separated lists, split without glob expansion (a `*` stays literal).
read -r -a excludes <<< "${XP_EXCLUDE-node_modules .git}"
for ex in ${excludes[@]+"${excludes[@]}"}; do
  args+=(--exclude "$ex")
done
read -r -a paths <<< "${XP_PATHS:-.}"
if [[ ${#paths[@]} -eq 0 ]]; then
  paths=(.)
fi

# `--`: a path that starts with `-` is still a path.
exec "$dir/$name/$exe" "${args[@]}" -- "${paths[@]}"
