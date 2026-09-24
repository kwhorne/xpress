# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **`--max-size` for video and audio now targets the size directly.** The
  bitrate is computed from the budget and duration and encoded two-pass
  (video), with the frame downscaled when the bits are too thin and up to three
  corrections — instead of up to six CRF re-encodes that couldn't aim at a
  size. Real 20 s 1080p clips landed at 87–93% of 10 MB / 4 MB / 1.5 MB budgets;
  MP3/AAC at 87–97%. Files still over budget get a warning.
- **Perceptual quality targets.** `optimise --quality high` (or
  `visually-lossless`, `medium`, `low`, or a score 1–100) and
  `convert --to webp|jpeg|png --quality …` find the smallest file that still
  meets a SSIMULACRA2 score against the original, instead of guessing a
  compression factor, and report the score achieved.
- **Already-optimised files are skipped.** `optimise` marks each result with an
  extended attribute (settings + CRC32 of the content); re-running over a
  folder skips unchanged files that were optimised at least as hard — instantly
  and without piling up generation loss. `--force` re-processes; `--json`
  reports `"cached"`.
- `--smart-crop` (and `smart: true` in pipelines) now works for images: a
  pure-Rust saliency crop (edges, saturation, skin tones) picks the most
  interesting region instead of the centre. It previously did nothing.
- `--pdf-dpi` now works: embedded JPEG images are downsampled to at most that
  DPI at the size they are actually drawn (read from the page content streams,
  falling back to the page size).

### Fixed
- PDF optimisation no longer re-encodes CMYK (or Lab/Indexed/Separation) JPEGs
  — decoding turned them into RGB while the PDF still declared CMYK, corrupting
  the colours. Gray images stay 1-component JPEGs.
- Shrink-only pipelines (e.g. the watch daemon's default `optimise`) no longer
  replace a file with a bigger result.
- **Animated GIFs were flattened to their first frame** (and reported as a big
  saving). Animated GIF/WebP/APNG are now never decoded to a single frame: GIFs
  go through `gifsicle` when installed, otherwise they are left untouched;
  resize/crop/convert of an animated image fails instead of dropping frames.
- **Photos lost their orientation, colour profile and EXIF** when optimised,
  converted, resized or cropped — portrait iPhone shots came out sideways and
  Display-P3 images washed out. The EXIF orientation is now applied to the
  pixels (tag reset), and the ICC profile and EXIF are carried through.
  `--strip-metadata` drops the EXIF but keeps the colour profile.
- **AVIF conversion** failed with "format not supported"; it now works (pure
  Rust, `ravif`), with quality taken from the compression value.
- **WebP conversion** was lossless-only (often larger than the PNG); it is now
  lossy via libwebp, keeps alpha and the ICC profile.
- XMP metadata (ratings, captions, edit history) is kept through JPEG, PNG and
  WebP outputs, and WebP now also carries EXIF; `--strip-metadata` drops XMP.
- AVIF output (which can't embed an ICC profile) is converted to sRGB from the
  source's colour profile, so Display-P3 and other wide-gamut images no longer
  shift colour.
- Transparent images converted to JPEG are flattened onto white instead of
  black.
- `downscale`/`crop` of an image **backed up the already-modified file**, so
  `restore` could not bring the original back. The backup is now taken first.
- Optimising a `.mov` no longer replaces it with a *larger* `.mp4` (and deletes
  the original); video conversions back the source up before removing it.
- H.264 output is always 8-bit 4:2:0, so iPhone HDR (10-bit) clips no longer
  become High-10 files that QuickTime/Safari can't play — and HDR sources (HLG
  or PQ) are tone-mapped to SDR BT.709 (zscale + mobius) instead of coming out
  grey and washed out. Falls back to a plain encode if ffmpeg lacks zscale;
  HEVC/AV1/VP9 conversions keep HDR untouched.
- AAC encoding falls back to ffmpeg's native `aac` encoder when `aac_at`
  (macOS AudioToolbox) is unavailable — e.g. on Linux.
- `watch`: files are only processed once they stop changing (no more
  half-copied files), and a run's own output (e.g. `clip.mp4`, `photo.webp`) no
  longer triggers a second run.
- `watch --clipboard`: the optimised image put back on the clipboard is no
  longer picked up and re-optimised in a loop; clipboard images are written to
  PNG in pure Rust (no ffmpeg needed).

### Changed
- **`convert` keeps the source for video and audio**, like it already did for
  images: the result is written alongside. Previously a video/audio conversion
  deleted the original (irrecoverably with `--no-backup`). Optimising a `.mov`
  to `.mp4` still replaces it. An audio conversion to a bigger format (e.g.
  MP3 → FLAC) is no longer refused by the size guard.
- Dependencies: `lopdf` 0.45 (fixes RUSTSEC-2026-0187, a stack overflow on
  crafted PDFs), `self_update` 1.x without the S3 backend, `egui`/`eframe`
  0.36, and a `cargo update` — `cargo audit` reports no vulnerabilities.
- An explicit MP3 bitrate (`lowerBitrate(kbps: …)`, size budgets) now encodes
  CBR at that bitrate instead of the nearest VBR quality level, which could land
  far from it.
- WebP encodes use libwebp's sharp RGB→YUV conversion, keeping coloured edges
  crisp; compression factors below 30 now reach WebP/HEIC/AVIF quality 95
  (previously capped at 72). The normal preset is unchanged.
- **Licence:** PNG quantisation no longer uses GPL-3.0 `libimagequant`
  (statically linked, which made the MIT binaries effectively GPL). Opaque PNGs
  now use `quantette` (k-means in Oklab, dithered), PNGs with transparency use
  `exoquant`; on our samples results are smaller for screenshots/icons and ~6 %
  larger for photos, with no visible difference. A PSNR floor
  derived from the compression value keeps an image lossless when a palette
  would cost too much quality. MSRV is now Rust 1.90.
- All outputs are written atomically (temp file + rename next to the
  destination, keeping permissions and, on macOS, Finder tags/xattrs), so a
  crash mid-write can't leave a truncated file.

## [0.4.8] - 2026-07-01

### Added
- **iPhone photo (HEIC/HEIF) conversion** on macOS — both directions. Read an
  Apple photo (`convert --to jpeg IMG.HEIC`) or write the iPhone format
  (`convert --to heic photo.png`), and optimise `.heic` files in place. Uses the
  built-in `sips`, so no extra tools are required.

### Changed
- `convert` now reports produced files clearly even when the new format is
  larger than the source (instead of the misleading “already optimal”).

## [0.4.7] - 2026-07-01

### Added
- Global hotkey **⌘⇧X** brings the xpress window to the front from anywhere
  (handy for the menu-bar app). ⌘⇧O still optimises the clipboard. Both are
  shown in the menu-bar menu.

## [0.4.6] - 2026-07-01

### Changed
- The macOS app is now a **pure menu-bar (agent) app** — no Dock icon
  (`LSUIElement`). It lives in the menu bar and brings its window to the front
  when opened.

## [0.4.5] - 2026-07-01

### Added
- The desktop app now lives in the **macOS menu bar**: a status-bar icon with a
  menu (Open, Optimise clipboard, Check for updates, Quit). Clicking it shows the
  window. Closing the window hides it to the menu bar instead of quitting (use
  the menu's “Quit” to exit), so xpress stays a click away.

## [0.4.4] - 2026-07-01

### Changed
- The desktop app now **updates itself**: the “Update available” banner (and the
  About dialog) show an **Update & Restart** button that downloads the new
  release, replaces the installed `.app`, and relaunches automatically — no more
  bouncing to the GitHub releases page. (macOS; falls back to a download link
  when not running from an installed `.app`.)

## [0.4.3] - 2026-07-01

### Changed
- PDF optimisation is now **pure Rust**: embedded JPEG images are recompressed
  via the image engine and streams are losslessly re-compressed with `lopdf` —
  no `ghostscript` needed. (`gs` is still used only by `extract-pages`.)
  Images + video + audio + PDF now all work with no external tools to install
  (video/audio via the bundled ffmpeg).

## [0.4.2] - 2026-07-01

### Changed
- Images are now optimised, resized, cropped and converted in **pure Rust**
  (`imagequant` + `oxipng` + the `image` crate) — no external tools
  (pngquant/jpegoptim/gifsicle/vips/cwebp) are required for images anymore.

### Added
- The macOS `.app`/`.dmg` now **bundle a self-contained `ffmpeg`** (video +
  audio), signed and notarised alongside the app. Combined with the pure-Rust
  image engine, images + video + audio work out of the box with nothing to
  install. (`scripts/fetch-static-tools.sh`, `make-app.sh --bin-dir`.)
  PDF still uses an external `ghostscript` if present.

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

[Unreleased]: https://github.com/kwhorne/xpress/compare/v0.4.8...HEAD
[0.4.8]: https://github.com/kwhorne/xpress/compare/v0.4.7...v0.4.8
[0.4.7]: https://github.com/kwhorne/xpress/compare/v0.4.6...v0.4.7
[0.4.6]: https://github.com/kwhorne/xpress/compare/v0.4.5...v0.4.6
[0.4.5]: https://github.com/kwhorne/xpress/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/kwhorne/xpress/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/kwhorne/xpress/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/kwhorne/xpress/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/kwhorne/xpress/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/kwhorne/xpress/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/kwhorne/xpress/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/kwhorne/xpress/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kwhorne/xpress/releases/tag/v0.1.0
