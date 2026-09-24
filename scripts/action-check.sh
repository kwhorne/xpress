#!/usr/bin/env bash
#
# Entry point of the `xpress check` GitHub Action (action.yml): download the
# xpress release binary for this runner and run `xpress check`.
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
case "${RUNNER_OS:-}-${RUNNER_ARCH:-}" in
  Linux-X64) target="x86_64-unknown-linux-gnu" ;;
  macOS-ARM64) target="aarch64-apple-darwin" ;;
  *)
    echo "::error::xpress check: no release binary for ${RUNNER_OS:-?}/${RUNNER_ARCH:-?} (use ubuntu-latest or macos-latest)"
    exit 1
    ;;
esac

version="${XP_VERSION:-latest}"
if [[ "$version" == "latest" ]]; then
  # The /releases/latest page redirects to /releases/tag/<tag>.
  version="$(curl -fsSLo /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")"
  version="${version##*/}"
fi

dir="${RUNNER_TEMP:-/tmp}/xpress-$version"
name="xpress-$version-$target"
if [[ ! -x "$dir/$name/xpress" ]]; then
  mkdir -p "$dir"
  echo "Downloading xpress $version ($target)…"
  curl -fsSL "https://github.com/$repo/releases/download/$version/$name.tar.gz" | tar -xz -C "$dir"
fi

args=(check --recursive --min-savings "${XP_MIN_SAVINGS:-10}" --quality "${XP_QUALITY:-visually-lossless}")
[[ -n "${XP_MAX_SIZE:-}" ]] && args+=(--max-size "$XP_MAX_SIZE")
for ex in ${XP_EXCLUDE-node_modules .git}; do
  args+=(--exclude "$ex")
done
# Word splitting of XP_PATHS is intended: it's a space-separated list.
# shellcheck disable=SC2206
paths=(${XP_PATHS:-.})

exec "$dir/$name/xpress" "${args[@]}" "${paths[@]}"
