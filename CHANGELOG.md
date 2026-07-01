# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Images are now optimised, resized, cropped and converted in **pure Rust**
  (`imagequant` + `oxipng` + the `image` crate) — no external tools
  (pngquant/jpegoptim/gifsicle/vips/cwebp) are required for images anymore.
  Video (ffmpeg), audio (ffmpeg) and PDF (ghostscript) still use their tools.

## [0.4.1] - 2026-07-01

### Changed
- Redesigned the desktop GUI: a left sidebar with sections and colourful icon
  tiles, roomy tabbed views (Optimise / Preferences / About), rounded cards,
  macOS-style toggle switches, and a Tokyo Night colour palette.

### Added
- macOS code signing + notarisation: `make-app.sh`/`make-dmg.sh` sign with a
  Developer ID (hardened runtime) when available, a `notarize.sh` helper submits
  and staples, and the release workflow signs + notarises when the signing
  secrets are configured. See `docs/signing.md`.
- Auto-update: `xpress update` checks GitHub Releases and replaces the binary in
  place (`--check` to only report). The desktop app checks on launch and then
  periodically (every 6h), shows an “Update available” banner when a newer
  release is published, and has a “Check for updates” button in the About dialog.

## [0.4.0] - 2026-06-29

### Added
- `--timeout <secs>` kills any external tool that runs too long (prevents hangs).
- `--jobs <n>` caps how many files are processed in parallel.
- `completions <shell>` and `man` commands to generate shell completions and a
  man page.

### Changed
- CI now runs a real-tool smoke test (ffmpeg/pngquant/…) and `cargo audit`.

### Fixed
- `crop-pdf` had a conflicting `-r` short flag (`--ratio` vs `--recursive`);
  `--ratio` is now long-only.

### Added (tests)
- Failure-path tests (missing file, unsupported type, unreadable dimensions) and
  an isolated external-tool timeout test.

## [0.3.0] - 2026-06-29

> Supersedes the briefly-tagged 0.2.1: these are new features, so they belong in
> a minor release under SemVer.

### Added
- GUI: an interactive **crop** tool (drag a region, apply), plus **Reveal** and
  **Copy** actions on each result card.
- `crop::crop_rect` in the engine for arbitrary normalised-rectangle crops.
- Integrations guide (Shortcuts via Run Shell Script, Folder Actions, Photos
  export flow, uploads via `runScript`).
- PDF: non-destructive `crop-pdf` (sets the page CropBox) and `uncrop-pdf`
  (removes it), plus `extract-pages` to render pages to PNG/JPEG.
- New pipeline steps: `normalize(lufs:)` (audio loudness), `watermark(image:,
  position:, opacity:, scale:)`, `copyToClipboard`, and `runScript(code:|path:)`.
- Video codec conversion: `convert --to mp4|hevc|av1|webm` (and the same in the
  pipeline DSL), with a `--hw` flag for VideoToolbox on Apple Silicon.
- Adaptive image optimisation is now transparency-aware: PNGs with an alpha
  channel never get a JPEG candidate (no silent flattening).

## [0.2.0] - 2026-06-29

### Added
- Live progress: batch commands show a spinner with a `[done/total]` counter and
  elapsed time on a terminal (suppressed under `--quiet`/`--json` and when piped).
- Clipboard “paste small”: on macOS the optimised image is written back to the
  clipboard as PNG (in both `watch --clipboard` and the GUI's ⌘⇧O).
- `convert --to gif` (and pipeline `convert(to: gif)`) to turn videos into GIFs
  (gifski when available, otherwise ffmpeg).
- `optimise --max-size <budget>` and a `targetSize(bytes:)` pipeline step to
  compress to a byte budget.
- `optimise --adaptive` and an `adaptive` pipeline step that try multiple image
  formats and keep the smallest.
- Output filename templates for `--output` (`%f`, `%e`, `%P`, date/time, `%i`,
  `%r`, `%%`).
- `--json` and `--quiet` output modes.
- `restore` and `clean-backups` commands to manage `.orig` backups.
- A user config file (`config.json`) for default compression and behaviours, plus
  a `config` command to show it.
- Integration test suite using stub tools.
- MSRV declared (Rust 1.87) and a `rust-toolchain.toml`; GitHub issue/PR templates.

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

[Unreleased]: https://github.com/kwhorne/xpress/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/kwhorne/xpress/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/kwhorne/xpress/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/kwhorne/xpress/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/kwhorne/xpress/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kwhorne/xpress/releases/tag/v0.1.0
