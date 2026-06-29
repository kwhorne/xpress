# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `convert --to gif` (and pipeline `convert(to: gif)`) to turn videos into GIFs
  (gifski when available, otherwise ffmpeg).
- `optimise --max-size <budget>` and a `targetSize(bytes:)` pipeline step to
  compress to a byte budget.
- `optimise --adaptive` and an `adaptive` pipeline step that try multiple image
  formats and keep the smallest.
- Output filename templates for `--output` (`%f`, `%e`, `%P`, date/time, `%i`,
  `%r`, `%%`).
- `--json` and `--quiet` output modes.
- Integration test suite using stub tools.

### Fixed
- `convert` to PNG/JPEG no longer optimises a file in place onto itself.
- Hardened path handling (no more `unwrap()` on `file_name`/`file_stem`).

## [0.1.0] - 2026-06-29

### Added
- **Core engine** (`xpress-core`): a percentage-based compression model and tool
  runner that drives `ffmpeg`, `pngquant`, `jpegoptim`, `gifsicle`, `ghostscript`,
  `vips`, `gifski`, `cwebp`, `heif-enc`, `cjxl` and `exiftool`.
  - Image (jpeg/png/gif), video (H.264), PDF (ghostscript) and audio optimisation.
  - Resolution scaling (`downscale`) for images and videos.
  - Format conversion: images (webp/avif/heic/jxl/png/jpeg) and audio
    (aac/mp3/opus/wav/flac/aiff).
  - Crop to a size, aspect ratio or long edge (vips smart crop / ffmpeg).
  - A pipeline DSL (`crop(width: 1600) -> convert(to: webp)`) with sequential
    execution, plus a saved-pipeline library and folder automations.
  - Parallel batch optimisation, backups, size guard, metadata and timestamp
    preservation.
- **CLI** (`xpress`): `optimise`, `downscale`, `convert`, `crop`, `pipeline`
  (run/add/list/show/delete/attach/detach), `watch`, `strip-exif`, `bundle`,
  `doctor`.
- **Daemon**: `watch` monitors folders (and optionally the clipboard) and runs
  the attached pipeline automatically, with debouncing and loop prevention.
- **Desktop GUI** (`xpress-gui`): egui/eframe app with drag-and-drop, floating
  result cards and thumbnails, a global hotkey (⌘⇧O) to optimise the clipboard
  image, always-on-top mode, and off-thread processing.
- **Binary bundling**: per-user bundle dir resolution, `scripts/fetch-tools.sh`,
  and an `embed-tools` feature that bakes binaries into the executable.
- **CI/Release**: GitHub Actions for fmt + clippy + tests, and tagged releases
  building macOS (arm64/x86_64) and Linux binaries, a macOS `.app` zip, and a
  macOS `.dmg` (with an Applications shortcut for drag-installing).

### Notes
- xpress is an independent project under the MIT License, inspired by the
  functionality of Clop. It contains no Clop source code. See `NOTICE.md`.

[Unreleased]: https://github.com/kwhorne/xpress/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kwhorne/xpress/releases/tag/v0.1.0
